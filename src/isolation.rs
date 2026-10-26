// SPDX-License-Identifier: Apache-2.0
//! Stage 2's attempts, made by GDCS's isolation probe (SPECIFICATION.md 4, Stage 2).
//!
//! A refusal is shown only by an attempt the system refuses. The probe (`fixtures/isolation-probe`) is an
//! ordinary service that holds nothing but `log_write` and, on startup, tries each thing an isolated service
//! must not be able to do, logging whether it was refused. GDCS installs it on the machine exactly as an
//! operator installs any package, waits for it to finish, and turns its lines into results. The probe is
//! bundled into this binary, built and signed for one release; against a different one the attempts read NOT
//! RUN, saying to rebuild it (`fixtures/README.md`).
//!
//! What this does NOT show, recorded rather than implied (CLAUDE.md 26.7): the attempts here are the ones a
//! running service can make and SURVIVE to report - a fabricated capability, a bad pointer handed to a
//! syscall, a gated or privileged call, a guess past the syscall surface. The pure MMU faults - dereferencing
//! an unmapped MMIO address or another service's memory directly - kill the caller rather than returning an
//! error, so they cannot be reported by the service that makes them; a deliberate one would also restart-loop
//! a managed package. `S2.kernel-pointer` shows the syscall boundary rejecting such addresses; the MMU-enforced
//! dereference fault is what the OS's own CLAUDE.md 22 A15/P7 pin on every release, and is not re-shown here.

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

/// The probe compiled into this binary (`fixtures/isolation-probe-<release>-x86_64.gsp`).
pub fn bundled() -> Probe {
    Probe {
        gsp: include_bytes!("../fixtures/isolation-probe-0.31.0-x86_64.gsp"),
        release: "0.31.0",
        key: "ed25519:d04ab232742bb4ab3a1368bd4615e4e6d0224ab71a016baf8520a332c9778737",
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

/// Install the probe on a running machine (the subject is already installed) and read its results. The
/// machine's log is left for the caller to scan for a kernel stop.
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
    let file_name = "isolation-probe.gsp";
    // Trust the probe's key for gdcs (a disposable QEMU machine), fetch it, and install it. Any step that does
    // not reach its mark leaves every attempt NOT RUN, saying where it stopped.
    if m.type_line(&format!("godspeed trust add gdcs {}", probe.key)).is_err() || m.wait_for("[y/N]", 60).is_none() {
        return not_run(env, "could not reach the trust prompt for the probe's key");
    }
    if !m
        .command("y", 60)
        .map(|o| o.contains("trusted by this machine's operator"))
        .unwrap_or(false)
    {
        return not_run(env, "the probe's key was not trusted");
    }
    let server = match FileServer::start(file_name, probe.gsp.to_vec()) {
        Ok(s) => s,
        Err(e) => return not_run(env, &format!("the host's file server for the probe: {e}")),
    };
    if !m
        .command(&format!("download {}", server.link()), 180)
        .map(|o| o.contains("saved"))
        .unwrap_or(false)
    {
        return not_run(env, "the probe package did not download onto the machine");
    }
    let from = m.len();
    if m.command(&format!("godspeed install {file_name} yes"), 300).is_none() {
        return not_run(env, "`godspeed install` of the probe did not return to the prompt");
    }
    if !m.seen_since(from, "supervisor: installed gdcs/isolation-probe 0.1.0 started", 60) {
        let said = m.text();
        let tail: Vec<String> = said.lines().rev().take(6).map(|l| l.trim_end().to_string()).collect();
        return ATTEMPTS
            .iter()
            .map(|a| {
                result(
                    a.id,
                    a.title,
                    env,
                    Outcome::NotRun,
                    "the probe did not install and start",
                    tail.clone(),
                )
            })
            .collect();
    }
    // The probe runs its attempts on startup and ends with its `done,` line.
    m.seen_since(from, "isolation-probe: done,", 60);
    let log = m.text();
    from_log(&log[from.min(log.len())..], env)
}
