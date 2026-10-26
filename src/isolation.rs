// SPDX-License-Identifier: Apache-2.0
//! Stage 2's attempts, made by GDCS's two probes (SPECIFICATION.md 4, Stage 2).
//!
//! A refusal is shown only by an attempt the system refuses. GDCS ships two probe packages, each a service
//! holding nothing but `log_write`, installed on the machine exactly as an operator installs any package, and
//! both bundled into this binary for one release (against a different one the attempts read NOT RUN, saying to
//! rebuild them - `fixtures/README.md`):
//!
//! - **`fixtures/isolation-probe`** makes the attempts a running service can make and SURVIVE to report - a
//!   fabricated capability, a bad pointer handed to a syscall, a gated or privileged call, a guess past the
//!   syscall surface - and logs whether each was refused. `run` reads those lines (`from_log`).
//! - **`fixtures/mmu-fault-probe`** makes the attempt that KILLS the caller: it reads an address it was never
//!   granted (an unmapped MMIO window, another service's memory, or any unmapped address - all unmapped for it,
//!   so all the same fault). A service cannot report its own death, so the DEATH is the evidence: the kernel
//!   kills the faulting task at the bad access, the supervisor reports `died - memory fault` (a fault, not a
//!   kill) and restarts it a bounded number of times before leaving it down, and the kernel never panics.
//!   `run_fault` reads that (`fault_from_log`). This used to be recorded as not built (the earlier worry was a
//!   restart loop); the installed-package restart bound makes the loop terminate, so it is built and bounded.

use crate::machine::{FileServer, Machine};
use crate::report::{Environment, Outcome, TestResult};

/// The bundled probe: its package bytes, the release it was built for, and the key that signed it.
pub struct Probe {
    /// The signed `.gsp`.
    pub gsp: &'static [u8],
    /// The GodspeedOS release its manifest names (`[os] release`).
    pub release: &'static str,
    /// The public key that signed it, which the run trusts for `gdcs` and then leaves.
    pub key: &'static str,
}

/// The GDCS fixtures' throwaway signing key (fixtures/README.md). The run trusts it for `gdcs` and leaves it.
const FIXTURE_KEY: &str = "ed25519:d04ab232742bb4ab3a1368bd4615e4e6d0224ab71a016baf8520a332c9778737";

/// The survivable-attempts probe compiled into this binary (`fixtures/isolation-probe-<release>.gsp`).
pub fn bundled() -> Probe {
    Probe {
        gsp: include_bytes!("../fixtures/isolation-probe-0.32.0.gsp"),
        release: "0.32.0",
        key: FIXTURE_KEY,
    }
}

/// Both bundled probes, for a Basic run.
pub struct Probes {
    /// The survivable-attempts probe (`run`).
    pub survivable: Probe,
    /// The kill-the-caller probe (`run_fault`).
    pub fault: Probe,
}

/// Both probes compiled into this binary.
pub fn bundled_probes() -> Probes {
    Probes {
        survivable: bundled(),
        fault: bundled_fault(),
    }
}

/// The kill-the-caller probe (`fixtures/mmu-fault-probe-<release>.gsp`): it reads an address it was never
/// granted, and its death is the evidence (`run_fault`).
pub fn bundled_fault() -> Probe {
    Probe {
        gsp: include_bytes!("../fixtures/mmu-fault-probe-0.32.0.gsp"),
        release: "0.32.0",
        key: FIXTURE_KEY,
    }
}

/// One Stage 2 attempt: the GDCS result id, the line-tag the probe logs it under, and the title.
struct Attempt {
    id: &'static str,
    tag: &'static str,
    title: &'static str,
}

const ATTEMPTS: &[Attempt] = &[
    Attempt {
        id: "S2.forged-cap",
        tag: "forged-cap",
        title: "a fabricated capability is refused",
    },
    Attempt {
        id: "S2.kernel-pointer",
        tag: "kernel-pointer",
        title: "a kernel-space pointer to a syscall is refused",
    },
    Attempt {
        id: "S2.introspect",
        tag: "introspect",
        title: "a gated kernel query is denied without INTROSPECT",
    },
    Attempt {
        id: "S2.reboot",
        tag: "reboot",
        title: "the machine's power is denied without the capability",
    },
    Attempt {
        id: "S2.acquire",
        tag: "acquire",
        title: "a send capability to a non-peer is denied",
    },
    Attempt {
        id: "S2.fuzz",
        tag: "fuzz",
        title: "out-of-range syscalls return a defined error, no crash",
    },
];

const VERSION: u32 = 1;

fn result(
    id: &str,
    title: &str,
    env: &Environment,
    outcome: Outcome,
    detail: impl Into<String>,
    evidence: Vec<String>,
) -> TestResult {
    TestResult {
        id: id.to_string(),
        version: VERSION,
        title: title.to_string(),
        outcome,
        detail: detail.into(),
        evidence,
        environment: env.clone(),
    }
}

/// Every Stage 2 attempt as NOT RUN, with one reason. Used by GDCS wherever the attempts cannot be made.
pub fn not_run(env: &Environment, why: &str) -> Vec<TestResult> {
    ATTEMPTS
        .iter()
        .map(|a| result(a.id, a.title, env, Outcome::NotRun, why, Vec::new()))
        .collect()
}

/// Turn the probe's output (the slice of the serial log from when the probe was installed) into results.
/// Independent of the machine, so it can be tested on written logs.
pub fn from_log(output: &str, env: &Environment) -> Vec<TestResult> {
    let finished = output.contains("isolation-probe: done,");
    ATTEMPTS
        .iter()
        .map(|a| {
            let pass = format!("isolation-probe: {} PASS", a.tag);
            let fail = format!("isolation-probe: {} FAIL", a.tag);
            if let Some(line) = output.lines().find(|l| l.contains(&pass)) {
                result(
                    a.id,
                    a.title,
                    env,
                    Outcome::Pass,
                    "refused, as the probe reports",
                    vec![line.trim().to_string()],
                )
            } else if let Some(line) = output.lines().find(|l| l.contains(&fail)) {
                let detail = line
                    .split(" FAIL - ")
                    .nth(1)
                    .unwrap_or("the probe reports it was not refused")
                    .trim();
                result(
                    a.id,
                    a.title,
                    env,
                    Outcome::Fail,
                    detail.to_string(),
                    vec![line.trim().to_string()],
                )
            } else if finished {
                result(
                    a.id,
                    a.title,
                    env,
                    Outcome::NotRun,
                    format!("the probe finished without reporting `{}`", a.tag),
                    Vec::new(),
                )
            } else {
                result(
                    a.id,
                    a.title,
                    env,
                    Outcome::NotRun,
                    "the probe did not finish (the machine may have stopped mid-probe)",
                    Vec::new(),
                )
            }
        })
        .collect()
}

/// The result id for the kill-the-caller attempt.
const UNMAPPED: (&str, &str) = (
    "S2.unmapped-read",
    "a dereference of an address not granted faults, killing the caller, not the kernel",
);

/// The kill-the-caller attempt as NOT RUN, for wherever it cannot be made.
pub fn fault_not_run(env: &Environment, why: &str) -> TestResult {
    result(UNMAPPED.0, UNMAPPED.1, env, Outcome::NotRun, why, Vec::new())
}

/// The on-disk file names the probes are downloaded to and installed from.
const ISOLATION_FILE: &str = "isolation-probe.gsp";
const FAULT_FILE: &str = "mmu-fault-probe.gsp";

/// Fetch both probe packages onto the machine, the way an operator does: trust the fixtures key for `gdcs`
/// once, then download each `.gsp`. This is done BEFORE the driver under test is installed, while the network
/// is still up - a driver that claims the network device takes `download` away, so a NIC driver's probes could
/// not be fetched afterward (found dogfooding the e1000 driver). Install is offline, from the files left here.
/// `Err` is one sentence saying where it stopped, or that the probes are built for another release.
pub fn fetch_all(m: &mut Machine, probes: &Probes, target_release: &str) -> Result<(), String> {
    if probes.survivable.release != target_release {
        return Err(format!(
            "the bundled probes are built for {}, and this run targets {}: rebuild fixtures/isolation-probe and \
             fixtures/mmu-fault-probe against {} (fixtures/README.md)",
            probes.survivable.release, target_release, target_release
        ));
    }
    if m.type_line(&format!("godspeed trust add gdcs {}", probes.survivable.key))
        .is_err()
        || m.wait_for("[y/N]", 60).is_none()
    {
        return Err("could not reach the trust prompt for the probes' key".to_string());
    }
    if !m
        .command("y", 60)
        .map(|o| o.contains("trusted by this machine's operator"))
        .unwrap_or(false)
    {
        return Err("the probes' key was not trusted".to_string());
    }
    download(m, ISOLATION_FILE, probes.survivable.gsp)?;
    download(m, FAULT_FILE, probes.fault.gsp)?;
    Ok(())
}

/// Download one `.gsp` from a host file server onto the machine.
fn download(m: &mut Machine, file_name: &str, bytes: &[u8]) -> Result<(), String> {
    let server = FileServer::start(file_name, bytes.to_vec())
        .map_err(|e| format!("the host's file server for {file_name}: {e}"))?;
    if !m
        .command(&format!("download {}", server.link()), 180)
        .map(|o| o.contains("saved"))
        .unwrap_or(false)
    {
        return Err(format!(
            "{file_name} did not download onto the machine (is the driver under test holding the network device?)"
        ));
    }
    Ok(())
}

/// Install an already-downloaded probe from disk (no network) and wait for the supervisor to start it. Returns
/// the log cursor just before the install, so the caller reads only what the probe produced.
fn install_from_disk(m: &mut Machine, file_name: &str, label: &str) -> Result<usize, String> {
    let from = m.len();
    if m.command(&format!("godspeed install {file_name} yes"), 300).is_none() {
        return Err(format!(
            "`godspeed install` of the {label} did not return to the prompt"
        ));
    }
    if !m.seen_since(from, &format!("supervisor: installed gdcs/{label} 0.1.0 started"), 60) {
        return Err(format!("the {label} did not install and start"));
    }
    Ok(from)
}

/// Install the survivable-attempts probe on a running machine (the subject is already installed) and read its
/// results. The machine's log is left for the caller to scan for a kernel stop.
pub fn run(m: &mut Machine, probe: &Probe, target_release: &str, env: &Environment) -> Vec<TestResult> {
    if probe.release != target_release {
        return not_run(
            env,
            &format!(
                "the bundled isolation probe is built for {}, and this run targets {}: rebuild \
                 fixtures/isolation-probe against {} (fixtures/README.md)",
                probe.release, target_release, target_release
            ),
        );
    }
    let from = match install_from_disk(m, ISOLATION_FILE, "isolation-probe") {
        Ok(from) => from,
        Err(why) => return not_run(env, &why),
    };
    // The probe runs its attempts on startup and ends with its `done,` line.
    m.seen_since(from, "isolation-probe: done,", 60);
    from_log(&m.text_from(from), env)
}

/// `S2.unmapped-read` from a serial log: the fault probe announced its read, the supervisor reported the death
/// as a MEMORY FAULT (not a kill), it was restarted a bounded number of times and left down, and the kernel
/// never panicked. Independent of the machine, so it can be tested on written logs.
pub fn fault_from_log(output: &str, env: &Environment) -> TestResult {
    let (id, title) = UNMAPPED;
    let r = |o: Outcome, d: &str, ev: Vec<String>| result(id, title, env, o, d, ev);
    if output.contains("KERNEL PANIC") || output.contains("LIVENESS WEDGE") {
        let ev = output
            .lines()
            .filter(|l| l.contains("KERNEL PANIC") || l.contains("LIVENESS WEDGE"))
            .map(|l| l.trim().to_string())
            .collect();
        return r(
            Outcome::Fail,
            "the deliberate fault stopped the kernel, instead of only the faulting task",
            ev,
        );
    }
    // The probe's own FAIL line means the read of 0x0 returned - the kernel did not refuse the access.
    if let Some(line) = output.lines().find(|l| l.contains("mmu-fault-probe: FAIL")) {
        return r(
            Outcome::Fail,
            "the read of an ungranted address returned instead of faulting",
            vec![line.trim().to_string()],
        );
    }
    let announced = output.lines().any(|l| l.contains("mmu-fault-probe: reading 0x0"));
    // The supervisor reports an installed package's death and its cause. A FAULT (`died - memory fault at`) is
    // the kernel killing the faulting task; a KILL (`killed by`) would mean something else ended it.
    let fault_line = output
        .lines()
        .find(|l| l.contains("mmu-fault-probe died") && l.contains("memory fault at"));
    let left_down = output
        .lines()
        .find(|l| l.contains("mmu-fault-probe died") && l.contains("left down"));
    match (announced, fault_line) {
        (false, _) => r(
            Outcome::NotRun,
            "the fault probe never announced its read (it may not have started)",
            Vec::new(),
        ),
        (true, None) => {
            let killed = output
                .lines()
                .find(|l| l.contains("mmu-fault-probe died") && l.contains("killed by"));
            match killed {
                Some(line) => r(Outcome::Fail, "the task ended by a kill, not by a fault at the bad access", vec![line.trim().to_string()]),
                None => r(Outcome::NotRun, "the fault probe announced its read but the supervisor reported no death (the machine may have stopped)", Vec::new()),
            }
        }
        (true, Some(line)) => {
            let mut ev = vec![line.trim().to_string()];
            if let Some(ld) = left_down {
                ev.push(ld.trim().to_string());
            }
            let bounded = if left_down.is_some() {
                "; restarted a bounded number of times, then left down"
            } else {
                ""
            };
            r(
                Outcome::Pass,
                &format!("the kernel killed the faulting task at the bad access and kept running{bounded}"),
                ev,
            )
        }
    }
}

/// Install the kill-the-caller probe and read `S2.unmapped-read`. It faults on startup, so it is restarted up to
/// the installed-package bound and then left down; this waits for that to settle so the machine is quiet before
/// the caller scans for a kernel stop.
pub fn run_fault(m: &mut Machine, probe: &Probe, target_release: &str, env: &Environment) -> TestResult {
    let (id, title) = UNMAPPED;
    if probe.release != target_release {
        return result(
            id,
            title,
            env,
            Outcome::NotRun,
            format!(
                "the bundled fault probe is built for {}, and this run targets {}: rebuild \
                 fixtures/mmu-fault-probe against {} (fixtures/README.md)",
                probe.release, target_release, target_release
            ),
            Vec::new(),
        );
    }
    let from = match install_from_disk(m, FAULT_FILE, "mmu-fault-probe") {
        Ok(from) => from,
        Err(why) => return result(id, title, env, Outcome::NotRun, why, Vec::new()),
    };
    // It faults immediately and is restarted; wait for it to be left down (bounded), else for a first death.
    if !m.seen_since(from, "mmu-fault-probe died", 90) {
        return result(
            id,
            title,
            env,
            Outcome::NotRun,
            "the fault probe installed but the supervisor reported no death within 90 s",
            Vec::new(),
        );
    }
    m.seen_since(from, "left down", 60);
    fault_from_log(&m.text_from(from), env)
}
