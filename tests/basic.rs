// SPDX-License-Identifier: Apache-2.0
//! Stages 2 and 3, each check seen to fail.
//!
//! The checks that read a serial log are tested here on logs written for the purpose. The ones that need a machine
//! are `#[ignore]`d QEMU runs - minutes each, one machine at a time - run with
//! `cargo test --test basic -- --ignored --test-threads=1` and these set:
//!
//! - `GDCS_IMAGE`: a release's x86-64 image, unpacked
//! - `GDCS_SUBJECT`: a driver package built for that release that drives QEMU's `edu` and logs `GDCS_EXPECT`
//! - `GDCS_EXPECT`: the line it logs when it works
//! - `GDCS_QEMU`: `qemu-system-x86_64` (optional when on PATH)

use std::path::PathBuf;

use gdcs::basic::{self, Subject};
use gdcs::declare::{Behaviour, Declaration, Qemu, FORMAT};
use gdcs::machine::{Board, Config};
use gdcs::report::{Environment, Outcome, TestResult};

fn env() -> Environment {
    Environment::Simulated { detail: "test".into() }
}

fn subject<'a>(file: &'a [u8], key: &'a str, dma: bool) -> Subject<'a> {
    Subject {
        name: "gdcs/edu-driver",
        version: "0.1.0",
        key,
        file,
        file_name: "edu-driver-0.1.0.gsp",
        dma,
    }
}

#[test]
fn the_kernel_check_fires_on_a_panic_and_on_a_wedge() {
    assert_eq!(basic::kernel("gsh> fine\n", &env()).outcome, Outcome::Pass);
    assert_eq!(
        basic::kernel("x\nKERNEL PANIC: panicked at y\n", &env()).outcome,
        Outcome::Fail
    );
    assert_eq!(
        basic::kernel("LIVENESS WEDGE: core 0 made NO progress\n", &env()).outcome,
        Outcome::Fail
    );
}

#[test]
fn dma_is_never_a_pass_where_the_machine_cannot_confine_it() {
    let s = subject(b"", "k", true);
    let no_iommu = "iommu: no IVRS table in 5 ACPI entries -> no AMD-Vi IOMMU on this machine\n";
    assert_eq!(basic::dma(no_iommu, &s, &env()).outcome, Outcome::NotEnforceable);
    let confined = "spawn[dma]: 'gdcs/edu-driver' arena confined to its domain\n";
    assert_eq!(basic::dma(confined, &s, &env()).outcome, Outcome::NotShown);
    assert_eq!(basic::dma("nothing said\n", &s, &env()).outcome, Outcome::NotRun);
    // Registers only: no DMA reach, so nothing to confine.
    assert_eq!(
        basic::dma(no_iommu, &subject(b"", "k", false), &env()).outcome,
        Outcome::Pass
    );
}

/// How one QEMU case changes the good run.
#[derive(Default)]
struct Case<'a> {
    devices: Option<&'a [&'a str]>,
    expect: Option<&'a str>,
    key: Option<&'a str>,
    firmware: Option<PathBuf>,
}

/// One QEMU run of the subject, changed as the case says.
fn qemu_run(tag: &str, case: Case) -> Vec<TestResult> {
    let image = PathBuf::from(std::env::var("GDCS_IMAGE").expect("GDCS_IMAGE"));
    let file = std::fs::read(std::env::var("GDCS_SUBJECT").expect("GDCS_SUBJECT")).unwrap();
    let qemu = PathBuf::from(std::env::var("GDCS_QEMU").unwrap_or_else(|_| "qemu-system-x86_64".into()));
    let real_key = {
        let pkg = gdcs::package::read(&file).unwrap();
        let sig = std::str::from_utf8(pkg.own("manifest.sig").unwrap()).unwrap();
        godspeed_package::ManifestSignature::parse(sig).unwrap().key.to_string()
    };
    let key = case.key.map(str::to_string).unwrap_or(real_key);
    let expect = case
        .expect
        .map(str::to_string)
        .unwrap_or_else(|| std::env::var("GDCS_EXPECT").expect("GDCS_EXPECT"));
    let devices = case.devices.unwrap_or(&["edu"]);
    let decl = Declaration {
        format: FORMAT,
        target: Default::default(),
        qemu: Qemu {
            devices: devices.iter().map(|d| d.to_string()).collect(),
            ..Default::default()
        },
        behaviour: vec![Behaviour {
            id: "works".into(),
            expect,
            within_secs: Some(30),
        }],
    };
    let cfg = Config {
        board: Board::Pc {
            firmware: case
                .firmware
                .unwrap_or_else(|| gdcs::machine::default_firmware(&qemu).expect("QEMU's firmware")),
        },
        qemu,
        image,
        work: std::env::temp_dir().join(format!("gdcs-basic-{}-{tag}", std::process::id())),
        devices: decl.qemu.devices.clone(),
        smp: 2,
    };
    let s = subject(&file, &key, true);
    let probes = gdcs::isolation::bundled_probes();
    let r = basic::run(&cfg, &decl, &s, Some(&probes), "0.31.0", &env());
    for x in &r {
        eprintln!("{tag}: {} {} - {}", x.id, x.outcome.word(), x.detail);
    }
    r
}

fn outcome(r: &[TestResult], id: &str) -> Outcome {
    r.iter()
        .find(|x| x.id == id)
        .unwrap_or_else(|| panic!("no {id}"))
        .outcome
}

#[test]
#[ignore = "boots QEMU"]
fn the_good_run_passes_every_stage_3_test() {
    let r = qemu_run("good", Case::default());
    for id in [
        "S3.boot",
        "S3.setup",
        "S3.install",
        "S3.behaviour.works",
        "S3.running",
        "S3.kernel",
        "S2.privilege",
    ] {
        assert_eq!(outcome(&r, id), Outcome::Pass, "{id}");
    }
}

#[test]
#[ignore = "boots QEMU"]
fn a_machine_that_does_not_start_fails_boot() {
    let r = qemu_run(
        "nofw",
        Case {
            firmware: Some(PathBuf::from("no-such-firmware.fd")),
            ..Case::default()
        },
    );
    assert_eq!(outcome(&r, "S3.boot"), Outcome::Fail);
    assert_eq!(outcome(&r, "S3.install"), Outcome::NotRun);
}

#[test]
#[ignore = "boots QEMU"]
fn a_key_that_did_not_sign_it_fails_the_install() {
    // A well-formed key that did not sign the package: the operator trusts it, and the install is refused.
    let other = "ed25519:7721d3618c7946587c59ce0693e08876368b5cc371f06105953e68d1a1396b3d";
    let r = qemu_run(
        "wrongkey",
        Case {
            key: Some(other),
            ..Case::default()
        },
    );
    assert_eq!(outcome(&r, "S3.setup"), Outcome::Pass);
    assert_eq!(outcome(&r, "S3.install"), Outcome::Fail);
    assert_eq!(outcome(&r, "S3.running"), Outcome::NotRun);
    assert_eq!(outcome(&r, "S2.privilege"), Outcome::NotRun);
}

#[test]
#[ignore = "boots QEMU"]
fn a_malformed_key_fails_the_setup() {
    let r = qemu_run(
        "badkey",
        Case {
            key: Some("ed25519:not-a-key"),
            ..Case::default()
        },
    );
    assert_eq!(outcome(&r, "S3.setup"), Outcome::Fail);
    assert_eq!(outcome(&r, "S3.install"), Outcome::NotRun);
}

#[test]
#[ignore = "boots QEMU"]
fn a_behaviour_not_seen_fails_and_the_rest_still_pass() {
    let r = qemu_run(
        "wrongline",
        Case {
            expect: Some("edu-driver: a line this driver never logs"),
            ..Case::default()
        },
    );
    assert_eq!(outcome(&r, "S3.install"), Outcome::Pass);
    assert_eq!(outcome(&r, "S3.behaviour.works"), Outcome::Fail);
    assert_eq!(outcome(&r, "S3.kernel"), Outcome::Pass);
}

#[test]
#[ignore = "boots QEMU"]
fn without_its_device_the_driver_is_not_shown_working() {
    let r = qemu_run(
        "nodevice",
        Case {
            devices: Some(&[]),
            ..Case::default()
        },
    );
    let failed: Vec<&str> = r
        .iter()
        .filter(|x| x.outcome == Outcome::Fail)
        .map(|x| x.id.as_str())
        .collect();
    assert!(!failed.is_empty(), "nothing failed with no device on the machine");
    assert!(failed.iter().all(|id| id.starts_with("S3.")), "{failed:?}");
}

// --- Stage 2 (isolation), at the parser level: each outcome seen without a machine ---

use gdcs::isolation;

#[test]
fn isolation_reports_pass_fail_and_missing() {
    let env = env();
    let good = "\
isolation-probe: started\n\
isolation-probe: forged-cap PASS - a fabricated capability handle is CapNotHeld\n\
isolation-probe: kernel-pointer PASS - kernel addresses refused\n\
isolation-probe: introspect PASS - denied without INTROSPECT\n\
isolation-probe: reboot PASS - denied without the capability\n\
isolation-probe: acquire PASS - denied without ACQUIRE_ANY\n\
isolation-probe: fuzz PASS - all UnknownSyscall\n\
isolation-probe: done, 6 of 6 refused\n";
    let r = isolation::from_log(good, &env);
    assert_eq!(r.len(), 6);
    assert!(
        r.iter().all(|x| x.outcome == Outcome::Pass),
        "{:?}",
        r.iter().map(|x| (&x.id, x.outcome)).collect::<Vec<_>>()
    );

    // A leak the probe reports as FAIL is a Stage 2 failure, carrying the probe's reason.
    let leak = good.replace(
        "forged-cap PASS - a fabricated capability handle is CapNotHeld",
        "forged-cap FAIL - a fabricated capability handle was accepted as authority",
    );
    let r = isolation::from_log(&leak, &env);
    let forged = r.iter().find(|x| x.id == "S2.forged-cap").unwrap();
    assert_eq!(forged.outcome, Outcome::Fail);
    assert!(forged.detail.contains("accepted as authority"), "{}", forged.detail);
    // The others still pass: one leak fails one attempt.
    assert_eq!(r.iter().filter(|x| x.outcome == Outcome::Pass).count(), 5);

    // The probe finished but never reported an attempt (a check it was built with went missing): NOT RUN.
    let missing = good.replace("isolation-probe: fuzz PASS - all UnknownSyscall\n", "");
    let fuzz = isolation::from_log(&missing, &env)
        .into_iter()
        .find(|x| x.id == "S2.fuzz")
        .unwrap();
    assert_eq!(fuzz.outcome, Outcome::NotRun);
    assert!(fuzz.detail.contains("without reporting"), "{}", fuzz.detail);

    // The machine stopped mid-probe (no `done,` line): every attempt NOT RUN, saying so.
    let stopped = "isolation-probe: started\nisolation-probe: forged-cap PASS - x\n";
    let r = isolation::from_log(stopped, &env);
    assert_eq!(
        r.iter().find(|x| x.id == "S2.forged-cap").unwrap().outcome,
        Outcome::Pass
    );
    assert_eq!(r.iter().find(|x| x.id == "S2.reboot").unwrap().outcome, Outcome::NotRun);
    assert!(r
        .iter()
        .find(|x| x.id == "S2.reboot")
        .unwrap()
        .detail
        .contains("did not finish"));
}

#[test]
fn a_probe_built_for_another_release_does_not_run() {
    let probe = isolation::bundled();
    // from_log is release-agnostic; the release gate is in run(), checked here by its public contract:
    // the bundled probe names 0.31.0, so a mismatch is the caller's to detect. We assert the fact the
    // gate depends on, so a rebuilt probe that forgets to update the release constant fails this.
    assert_eq!(probe.release, "0.31.0");
    assert!(probe.gsp.starts_with(b"godspeed-package-file 1"));
    assert!(probe.key.starts_with("ed25519:"));
}

// --- Stage 2 kill-the-caller (S2.unmapped-read), at the parser level ---

#[test]
fn the_fault_probe_pass_fail_and_missing() {
    let env = env();
    let good = "\
mmu-fault-probe: reading 0x0, an address this service was never granted\n\
supervisor: installed gdcs/mmu-fault-probe died - memory fault at 0x0, restarting (1 of 5)\n\
supervisor: installed gdcs/mmu-fault-probe died - memory fault at 0x0 - restarted 5 times already, left down\n";
    let r = isolation::fault_from_log(good, &env);
    assert_eq!(r.outcome, Outcome::Pass, "{}", r.detail);
    assert!(r.detail.contains("left down"), "{}", r.detail);

    // The kernel stopped: a deliberate fault that panics or wedges the kernel is the worst outcome, and fails.
    let panic = format!("{good}KERNEL PANIC: panicked at kernel/src/x.rs\n");
    assert_eq!(isolation::fault_from_log(&panic, &env).outcome, Outcome::Fail);
    assert!(isolation::fault_from_log(&panic, &env)
        .detail
        .contains("stopped the kernel"));

    // The read returned instead of faulting: the probe says so, and it is a failure.
    let returned = "mmu-fault-probe: reading 0x0, ...\nmmu-fault-probe: FAIL - the read of 0x0 returned; the kernel did not refuse it\n";
    assert_eq!(isolation::fault_from_log(returned, &env).outcome, Outcome::Fail);

    // A KILL, not a fault: something ended the task rather than the MMU faulting it. Not the property claimed.
    let killed = "mmu-fault-probe: reading 0x0, ...\nsupervisor: installed gdcs/mmu-fault-probe died - killed by shell, restarting (1 of 5)\n";
    assert_eq!(isolation::fault_from_log(killed, &env).outcome, Outcome::Fail);
    assert!(isolation::fault_from_log(killed, &env).detail.contains("kill, not"));

    // It never announced its read (never started): NOT RUN, not a false pass.
    assert_eq!(
        isolation::fault_from_log("nothing here\n", &env).outcome,
        Outcome::NotRun
    );
    // It announced but no death was reported (machine stopped mid-way): NOT RUN.
    let hung = "mmu-fault-probe: reading 0x0, an address this service was never granted\n";
    assert_eq!(isolation::fault_from_log(hung, &env).outcome, Outcome::NotRun);
}

// --- Stage 4 (MAX CARNAGE) parser, each outcome seen without a machine ---

use gdcs::carnage;

/// A storm log where the driver was killed 5 times (each recovered) then left down on the 6th, each death
/// reclaiming frames. The bystander result is passed in (driven live by the real run).
fn carnage_good() -> String {
    let mut s = String::new();
    for n in 1..=5 {
        s.push_str("kill_task: slot=15 'gdcs/edu-driver' freed 84 frames\n");
        s.push_str(&format!(
            "supervisor: installed gdcs/edu-driver died - killed by shell, restarting ({n} of 5)\n"
        ));
        s.push_str("supervisor: installed gdcs/edu-driver 0.1.0 started\n");
    }
    s.push_str("kill_task: slot=15 'gdcs/edu-driver' freed 84 frames\n");
    s.push_str("supervisor: installed gdcs/edu-driver died - killed by shell - restarted 5 times already, left down\n");
    s
}

fn pass_bystanders() -> TestResult {
    TestResult {
        id: "S4.bystanders".into(),
        version: 1,
        title: "t".into(),
        outcome: Outcome::Pass,
        detail: String::new(),
        evidence: vec![],
        environment: env(),
    }
}

fn c_outcome(log: &str, id: &str) -> Outcome {
    carnage::from_log(log, "gdcs/edu-driver", pass_bystanders(), &env())
        .into_iter()
        .find(|r| r.id == id)
        .unwrap()
        .outcome
}

#[test]
fn carnage_good_log_passes_recover_bounded_cleanup() {
    let g = carnage_good();
    assert_eq!(c_outcome(&g, "S4.recover"), Outcome::Pass);
    assert_eq!(c_outcome(&g, "S4.bounded"), Outcome::Pass);
    assert_eq!(c_outcome(&g, "S4.cleanup"), Outcome::Pass);
    assert_eq!(c_outcome(&g, "S4.kernel"), Outcome::Pass);
}

#[test]
fn carnage_bounded_fails_when_unbounded_or_wrong_cap() {
    // Never left down, more restarts than the cap: unbounded, a failure.
    let mut unb = String::new();
    for n in 1..=7 {
        unb.push_str(&format!(
            "supervisor: installed gdcs/edu-driver died - killed by shell, restarting ({n} of 5)\n"
        ));
    }
    assert_eq!(c_outcome(&unb, "S4.bounded"), Outcome::Fail);
    // Left down early (not at the cap): also a failure.
    let early = "supervisor: installed gdcs/edu-driver died - killed by shell, restarting (1 of 5)\nsupervisor: installed gdcs/edu-driver died - killed by shell - restarted 5 times already, left down\n";
    assert_eq!(c_outcome(early, "S4.bounded"), Outcome::Fail);
}

#[test]
fn carnage_cleanup_fails_on_a_zero_reclaim_or_a_missing_one() {
    // A death with no `freed` line at all.
    let none = "supervisor: installed gdcs/edu-driver died - killed by shell, restarting (1 of 5)\n";
    assert_eq!(c_outcome(none, "S4.cleanup"), Outcome::Fail);
    // A death that reclaimed 0 frames.
    let zero = "kill_task: slot=15 'gdcs/edu-driver' freed 0 frames\nsupervisor: installed gdcs/edu-driver died - killed by shell, restarting (1 of 5)\n";
    assert_eq!(c_outcome(zero, "S4.cleanup"), Outcome::Fail);
}

#[test]
fn carnage_recover_fails_when_the_driver_never_restarts() {
    let log = "supervisor: installed gdcs/edu-driver died - killed by shell - restarted 5 times already, left down\n";
    assert_eq!(c_outcome(log, "S4.recover"), Outcome::Fail);
}

#[test]
fn carnage_kernel_fails_on_a_panic_or_wedge_and_notes_a_wedge() {
    let g = carnage_good();
    assert_eq!(carnage::kernel(&g, &env()).outcome, Outcome::Pass);
    let panic = format!("{g}KERNEL PANIC: panicked at x\n");
    assert_eq!(carnage::kernel(&panic, &env()).outcome, Outcome::Fail);
    let wedge = format!("{g}LIVENESS WEDGE: core 0 made NO progress\n");
    let r = carnage::kernel(&wedge, &env());
    assert_eq!(r.outcome, Outcome::Fail);
    assert!(r.detail.contains("rerun once"), "{}", r.detail);
}

#[test]
#[ignore = "boots QEMU"]
fn carnage_on_a_real_machine_is_bounded_and_survivable() {
    let image = PathBuf::from(std::env::var("GDCS_IMAGE").expect("GDCS_IMAGE"));
    let file = std::fs::read(std::env::var("GDCS_SUBJECT").expect("GDCS_SUBJECT")).unwrap();
    let qemu = PathBuf::from(std::env::var("GDCS_QEMU").unwrap_or_else(|_| "qemu-system-x86_64".into()));
    let key = {
        let pkg = gdcs::package::read(&file).unwrap();
        let sig = std::str::from_utf8(pkg.own("manifest.sig").unwrap()).unwrap();
        godspeed_package::ManifestSignature::parse(sig).unwrap().key.to_string()
    };
    let cfg = Config {
        board: Board::Pc {
            firmware: gdcs::machine::default_firmware(&qemu).expect("firmware"),
        },
        qemu,
        image,
        work: std::env::temp_dir().join(format!("gdcs-carnage-{}", std::process::id())),
        devices: vec!["edu".into()],
        smp: 2,
    };
    let s = subject(&file, &key, true);
    let expect = std::env::var("GDCS_EXPECT").expect("GDCS_EXPECT");
    let r = carnage::run(&cfg, &s, Some(&expect), "0.31.0", &env());
    for x in &r {
        eprintln!("carnage: {} {} - {}", x.id, x.outcome.word(), x.detail);
    }
    for id in ["S4.recover", "S4.bounded", "S4.cleanup", "S4.bystanders", "S4.kernel"] {
        assert_eq!(r.iter().find(|x| x.id == id).unwrap().outcome, Outcome::Pass, "{id}");
    }
}
