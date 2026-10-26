// SPDX-License-Identifier: Apache-2.0
//! Stages 2 and 3 of the Basic profile, on a release image in QEMU (SPECIFICATION.md 4, 5).
//!
//! The driver is installed exactly as an operator installs one - the data disk formatted with `drives flash`, the
//! package fetched with `download`, its key trusted with `godspeed trust add`, the package installed with `godspeed
//! install` - and then watched. Every result here is `simulated` and names the emulator, its devices and the image.
//!
//! **What Stage 2 does not yet show, said rather than implied.** A refusal is shown only by an attempt the system
//! refuses (SPECIFICATION.md 4, Stage 2). The attempts - an MMIO region not granted, a capability not given, another
//! service's memory, another device's interrupt, a malformed request - are made by an isolation probe that is not
//! built yet, so those tests read NOT RUN and a Basic run is INCOMPLETE until it is. What this stage reports now is
//! what the machine can say without one: the grant the supervisor made, and whether this machine can confine DMA.

use crate::declare::Declaration;
use crate::machine::{Config, FileServer, Machine};
use crate::report::{Environment, Outcome, TestResult};

/// Stage 2 and 3 tests, by identifier, version and title. Behaviours are `S3.behaviour.<id>`, one per declaration.
const PRIVILEGE: (&str, u32, &str) = ("S2.privilege", 1, "given no privilege");
const DMA: (&str, u32, &str) = ("S2.dma", 1, "DMA confined to its arena");
const ATTEMPTS: &[(&str, u32, &str)] = &[
    ("S2.mmio", 1, "an MMIO region not granted is refused"),
    ("S2.capability", 1, "a capability not given is refused"),
    ("S2.memory", 1, "another service's memory is unreachable"),
    ("S2.interrupt", 1, "another device's interrupt is not delivered"),
    ("S2.requests", 1, "malformed requests widen nothing"),
];
const BOOT: (&str, u32, &str) = ("S3.boot", 1, "the release boots to a prompt");
const SETUP: (&str, u32, &str) = ("S3.setup", 1, "a data disk, the package fetched, its key trusted");
const INSTALL: (&str, u32, &str) = ("S3.install", 1, "installed and started by the supervisor");
const BEHAVIOUR: (&str, u32, &str) = ("S3.behaviour", 1, "declared behaviour");
const RUNNING: (&str, u32, &str) = ("S3.running", 1, "still running after its behaviour");
const KERNEL: (&str, u32, &str) = ("S3.kernel", 1, "no kernel panic or wedge");

/// Lines that mean the kernel stopped. Seeing one fails `S3.kernel`, whatever else passed.
const KERNEL_STOPS: &[&str] = &["KERNEL PANIC", "LIVENESS WEDGE"];

/// What a Basic run needs to know about the package.
pub struct Subject<'a> {
    /// `publisher/name`.
    pub name: &'a str,
    /// Its version.
    pub version: &'a str,
    /// The key that signed it, which the operator trusts for its publisher (`godspeed trust add`).
    pub key: &'a str,
    /// The package file's bytes and the name it is served and saved under.
    pub file: &'a [u8],
    /// The file name.
    pub file_name: &'a str,
    /// Whether its contract asks for a DMA arena.
    pub dma: bool,
}

fn result(
    t: (&str, u32, &str),
    env: &Environment,
    outcome: Outcome,
    detail: impl Into<String>,
    evidence: Vec<String>,
) -> TestResult {
    TestResult {
        id: t.0.to_string(),
        version: t.1,
        title: t.2.to_string(),
        outcome,
        detail: detail.into(),
        evidence,
        environment: env.clone(),
    }
}

/// The lines of `text` that hold `needle`, at most three, for evidence.
fn lines_with(text: &str, needle: &str) -> Vec<String> {
    text.lines()
        .filter(|l| l.contains(needle))
        .take(3)
        .map(|l| l.trim_end().to_string())
        .collect()
}

/// The last few non-empty lines of `text`, for the evidence of a step that went wrong.
fn tail(text: &str, n: usize) -> Vec<String> {
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    lines[lines.len().saturating_sub(n)..]
        .iter()
        .map(|l| l.trim_end().to_string())
        .collect()
}

/// Run Stages 2 and 3. `env` names the environment on every result; the machine's serial log is left at
/// `cfg.work/serial.log`.
pub fn run(cfg: &Config, decl: &Declaration, s: &Subject, env: &Environment) -> Vec<TestResult> {
    let mut out = Vec::new();
    let skip_rest = |out: &mut Vec<TestResult>, from: &str, why: &str| {
        let order = ["S3.boot", "S3.setup", "S3.install", "S3.behaviour", "S3.running", "S2"];
        let start = order.iter().position(|o| *o == from).unwrap_or(0);
        for o in &order[start..] {
            match *o {
                "S3.boot" => out.push(result(BOOT, env, Outcome::NotRun, why, Vec::new())),
                "S3.setup" => out.push(result(SETUP, env, Outcome::NotRun, why, Vec::new())),
                "S3.install" => out.push(result(INSTALL, env, Outcome::NotRun, why, Vec::new())),
                "S3.behaviour" => out.push(result(BEHAVIOUR, env, Outcome::NotRun, why, Vec::new())),
                "S3.running" => out.push(result(RUNNING, env, Outcome::NotRun, why, Vec::new())),
                _ => {
                    out.push(result(PRIVILEGE, env, Outcome::NotRun, why, Vec::new()));
                    out.push(result(DMA, env, Outcome::NotRun, why, Vec::new()));
                }
            }
        }
    };

    let mut m = match Machine::boot(cfg) {
        Ok(m) => m,
        Err(e) => {
            out.push(result(
                BOOT,
                env,
                Outcome::Fail,
                format!("the machine did not start: {e}"),
                Vec::new(),
            ));
            skip_rest(&mut out, "S3.setup", "needs a machine that boots");
            push_attempts(&mut out, env);
            return out;
        }
    };

    // Boot: a prompt, and the supervisor saying it is ready.
    let booted = m.wait_for("gsh>", 180);
    let text = m.text();
    if booted.is_none() || !text.contains("supervisor: ready") {
        out.push(result(
            BOOT,
            env,
            Outcome::Fail,
            "no `gsh>` prompt within 180 s of power-on",
            tail(&text, 6),
        ));
        skip_rest(&mut out, "S3.setup", "needs a machine that reached its prompt");
        push_attempts(&mut out, env);
        finish(m, &mut out, env);
        return out;
    }
    out.push(result(
        BOOT,
        env,
        Outcome::Pass,
        "reached `gsh>`",
        lines_with(&text, "supervisor: ready"),
    ));

    // Setup: the operator's three steps. The network is usually up by the prompt; it is waited for, not required.
    let _ = m.seen_since(0, "echo reply (ping OK)", 60);
    let setup = setup(&mut m, s);
    match setup {
        Ok(evidence) => out.push(result(
            SETUP,
            env,
            Outcome::Pass,
            "formatted, downloaded, trusted",
            evidence,
        )),
        Err((why, evidence)) => {
            out.push(result(SETUP, env, Outcome::Fail, why, evidence));
            skip_rest(
                &mut out,
                "S3.install",
                "needs the package on the machine and its key trusted",
            );
            push_attempts(&mut out, env);
            finish(m, &mut out, env);
            return out;
        }
    }

    // Install. `yes` answers every question, and is what was approved: the device and, asked separately, DMA.
    let from = m.len();
    let installed = m.command(&format!("godspeed install {} yes", s.file_name), 300);
    let said = installed.clone().unwrap_or_default();
    let started = format!("supervisor: installed {} {} started", s.name, s.version);
    let log = m.text();
    if installed.is_none() {
        out.push(result(
            INSTALL,
            env,
            Outcome::Fail,
            "`godspeed install` did not return to the prompt within 300 s",
            tail(&log, 8),
        ));
    } else if !log[from.min(log.len())..].contains(&started) {
        let why = lines_with(&said, "refus")
            .into_iter()
            .chain(lines_with(&said, "not"))
            .take(3)
            .collect::<Vec<_>>();
        let evidence = if why.is_empty() { tail(&said, 8) } else { why };
        out.push(result(
            INSTALL,
            env,
            Outcome::Fail,
            format!("the supervisor never said `{started}`"),
            evidence,
        ));
    } else {
        out.push(result(
            INSTALL,
            env,
            Outcome::Pass,
            "the supervisor checked it from the disk and started it",
            lines_with(&log, &started),
        ));
    }
    let install_ok = out.last().is_some_and(|r| r.outcome == Outcome::Pass);

    // Behaviour: every declared line, each within its own time of the install.
    if !install_ok {
        out.push(result(
            BEHAVIOUR,
            env,
            Outcome::NotRun,
            "needs the driver installed and started",
            Vec::new(),
        ));
    } else if decl.behaviour.is_empty() {
        out.push(result(
            BEHAVIOUR,
            env,
            Outcome::NotRun,
            "the declaration names no behaviour, so this driver is a probe: a functional pass covers only what a \
             driver declares (gdcs.toml, [[behaviour]])",
            Vec::new(),
        ));
    } else {
        for b in &decl.behaviour {
            let id = format!("S3.behaviour.{}", b.id);
            let t = (id.as_str(), BEHAVIOUR.1, BEHAVIOUR.2);
            if m.seen_since(from, &b.expect, b.within()) {
                let text = m.text();
                out.push(result(
                    t,
                    env,
                    Outcome::Pass,
                    format!("seen within {} s", b.within()),
                    lines_with(&text[from.min(text.len())..], &b.expect),
                ));
            } else {
                let name = s.name.rsplit('/').next().unwrap_or(s.name);
                let text = m.text();
                let said = lines_with(&text[from.min(text.len())..], name);
                out.push(result(
                    t,
                    env,
                    Outcome::Fail,
                    format!("`{}` not seen within {} s of the install", b.expect, b.within()),
                    if said.is_empty() {
                        vec![format!("no line naming `{name}` after the install")]
                    } else {
                        said
                    },
                ));
            }
        }
    }

    // Still running, by the package manager's own list.
    if install_ok {
        let listed = m.command("godspeed list installed", 120).unwrap_or_default();
        // The list can have a driver's log line spliced into it, so the package and its state are looked for apart.
        let running = listed
            .lines()
            .any(|l| l.contains(s.name) && l.contains("running") && !l.contains("NOT running"))
            || (listed.contains(s.name) && listed.contains(" running") && !listed.contains("NOT running"));
        let evidence = lines_with(&listed, s.name);
        out.push(if running {
            result(
                RUNNING,
                env,
                Outcome::Pass,
                "`godspeed list installed` says running",
                evidence,
            )
        } else {
            result(
                RUNNING,
                env,
                Outcome::Fail,
                "`godspeed list installed` does not say it is running",
                tail(&listed, 6),
            )
        });
    } else {
        out.push(result(
            RUNNING,
            env,
            Outcome::NotRun,
            "needs the driver installed and started",
            Vec::new(),
        ));
    }

    // Stage 2, as far as this machine can show it without the probe.
    let log = m.text();
    let card = &said;
    if install_ok {
        let line = lines_with(card, "privilege");
        out.push(if line.iter().any(|l| l.contains("none")) {
            result(
                PRIVILEGE,
                env,
                Outcome::Pass,
                "the supervisor reports it was given no privilege",
                line,
            )
        } else {
            result(
                PRIVILEGE,
                env,
                Outcome::Fail,
                "the install card does not say `privilege none`",
                tail(card, 8),
            )
        });
    } else {
        out.push(result(
            PRIVILEGE,
            env,
            Outcome::NotRun,
            "needs the driver installed",
            Vec::new(),
        ));
    }
    out.push(dma(&log, s, env));
    push_attempts(&mut out, env);
    finish(m, &mut out, env);
    out
}

fn setup(m: &mut Machine, s: &Subject) -> Result<Vec<String>, (String, Vec<String>)> {
    let mut evidence = Vec::new();
    m.type_line("drives flash 0 data").map_err(|e| (e, Vec::new()))?;
    if m.wait_for("[y/N]", 60).is_none() {
        return Err((
            "`drives flash 0 data` asked nothing within 60 s".to_string(),
            tail(&m.text(), 6),
        ));
    }
    let flashed = m.command("y", 120).unwrap_or_default();
    if !flashed.contains("formatted as GSFS") {
        return Err((
            "`drives flash 0 data` did not format the data disk".to_string(),
            tail(&flashed, 6),
        ));
    }
    evidence.extend(lines_with(&flashed, "formatted as GSFS"));

    let server = FileServer::start(s.file_name, s.file.to_vec())
        .map_err(|e| (format!("the host's file server: {e}"), Vec::new()))?;
    let got = m
        .command(&format!("download {}", server.link()), 180)
        .unwrap_or_default();
    if !got.lines().any(|l| l.contains("saved") && l.contains(s.file_name)) {
        return Err((
            format!("`download {}` did not save the package", server.link()),
            tail(&got, 6),
        ));
    }
    evidence.extend(lines_with(&got, "saved"));

    m.type_line(&format!(
        "godspeed trust add {} {}",
        s.name.split('/').next().unwrap_or(""),
        s.key
    ))
    .map_err(|e| (e, Vec::new()))?;
    if m.wait_for("[y/N]", 120).is_none() {
        return Err((
            "`godspeed trust add` asked nothing within 120 s".to_string(),
            tail(&m.text(), 6),
        ));
    }
    let trusted = m.command("y", 120).unwrap_or_default();
    if !trusted.contains("trusted by this machine's operator") {
        return Err((
            "`godspeed trust add` did not trust the key".to_string(),
            tail(&trusted, 6),
        ));
    }
    evidence.extend(lines_with(&trusted, "trusted by this machine's operator"));
    Ok(evidence)
}

/// `S2.dma`, from a whole serial log.
pub fn dma(log: &str, s: &Subject, env: &Environment) -> TestResult {
    if !s.dma {
        return result(
            DMA,
            env,
            Outcome::Pass,
            "asks for no DMA arena, so its device has no DMA reach to confine",
            Vec::new(),
        );
    }
    let none = lines_with(log, "iommu: no IVRS table");
    if !none.is_empty() {
        return result(
            DMA,
            env,
            Outcome::NotEnforceable,
            "this machine has no IOMMU: a driver given a DMA arena can point its device at any address, and is \
             inside the trusted computing base by necessity (GodspeedOS CLAUDE.md 6.4)",
            none,
        );
    }
    let confined = lines_with(log, &format!("'{}'", s.name))
        .into_iter()
        .filter(|l| l.contains("confine"))
        .collect::<Vec<_>>();
    if !confined.is_empty() {
        return result(
            DMA,
            env,
            Outcome::NotShown,
            "the kernel confined the device, and QEMU's IOMMU maps but does not fault, so no refused access can be \
             shown here",
            confined,
        );
    }
    result(
        DMA,
        env,
        Outcome::NotRun,
        "the log says neither that there is no IOMMU nor that the device was confined",
        Vec::new(),
    )
}

/// Every Stage 2 and 3 result as NOT RUN, for a run that never booted the machine.
pub fn not_run(env: &Environment, why: &str) -> Vec<TestResult> {
    let mut out = Vec::new();
    for t in [BOOT, SETUP, INSTALL, BEHAVIOUR, RUNNING, PRIVILEGE, DMA] {
        out.push(result(t, env, Outcome::NotRun, why, Vec::new()));
    }
    for t in ATTEMPTS {
        out.push(result(*t, env, Outcome::NotRun, why, Vec::new()));
    }
    out.push(result(KERNEL, env, Outcome::NotRun, why, Vec::new()));
    out
}

fn push_attempts(out: &mut Vec<TestResult>, env: &Environment) {
    for t in ATTEMPTS {
        out.push(result(
            *t,
            env,
            Outcome::NotRun,
            "needs the GDCS isolation probe, which is not built yet: a refusal is shown only by an attempt",
            Vec::new(),
        ));
    }
}

/// Stop the machine and read its whole log once more for a kernel that stopped.
fn finish(m: Machine, out: &mut Vec<TestResult>, env: &Environment) {
    std::thread::sleep(std::time::Duration::from_secs(2));
    let log = m.text();
    m.stop();
    out.push(kernel(&log, env));
}

/// `S3.kernel`, from a whole serial log.
pub fn kernel(log: &str, env: &Environment) -> TestResult {
    let stops: Vec<String> = KERNEL_STOPS.iter().flat_map(|k| lines_with(log, k)).collect();
    if stops.is_empty() {
        result(
            KERNEL,
            env,
            Outcome::Pass,
            "no `KERNEL PANIC` or `LIVENESS WEDGE` in the serial log",
            Vec::new(),
        )
    } else {
        result(KERNEL, env, Outcome::Fail, "the kernel stopped", stops)
    }
}
