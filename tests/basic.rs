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
use gdcs::machine::Config;
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
        qemu: Qemu {
            devices: devices.iter().map(|d| d.to_string()).collect(),
        },
        behaviour: vec![Behaviour {
            id: "works".into(),
            expect,
            within_secs: Some(30),
        }],
    };
    let cfg = Config {
        firmware: case
            .firmware
            .unwrap_or_else(|| gdcs::machine::default_firmware(&qemu).expect("QEMU's firmware")),
        qemu,
        image,
        work: std::env::temp_dir().join(format!("gdcs-basic-{}-{tag}", std::process::id())),
        devices: decl.qemu.devices.clone(),
        smp: 2,
    };
    let s = subject(&file, &key, true);
    let r = basic::run(&cfg, &decl, &s, &env());
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
