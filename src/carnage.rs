// SPDX-License-Identifier: Apache-2.0
//! Stage 4, MAX CARNAGE, as far as a register-only driver and QEMU allow (SPECIFICATION.md 4, Stage 4).
//!
//! The required outcome of Stage 4: the driver may fail; the kernel and unrelated services may not; the
//! supervisor detects each failure and applies recovery, and recovery is bounded. This runs that for a driver
//! package on its own fresh machine: install it, then kill it over and over while it is doing its work, and
//! measure what happens.
//!
//! - **S4.recover** - each kill is recovered: the supervisor restarts the driver and it runs again.
//! - **S4.bounded** - recovery is bounded, not endless: after the installed-package restart cap the supervisor
//!   leaves it down and says so, loudly. An unbounded restart, or a silent give-up, fails.
//! - **S4.cleanup** - every death reclaims the task's frames (the kernel's `freed N frames`), so a restart
//!   storm does not leak.
//! - **S4.bystanders** - the kernel and unrelated services are untouched: after the storm the shell answers, a
//!   file round-trips through `fs`, and `net-stack` still pings. The driver itself being down is allowed.
//! - **S4.kernel** - no `KERNEL PANIC` or `LIVENESS WEDGE` anywhere in the run.
//!
//! What is NOT built here, recorded rather than implied (CLAUDE.md 26.7): the device-specific carnage the spec
//! also describes - a flood of refused sends from a client while the driver is down (a register-only driver has
//! no client to flood it), device disconnect/reconnect and a device-reset failure (its device is PCI `edu`,
//! with no hot-plug in QEMU), and input integrity across a hot-plug - need a driver with a client peer or a
//! hot-pluggable device. And the lateness rule (rerun a failed run once, report a flake) is not automated: a
//! LIVENESS WEDGE here is reported as what it is, for a human to rerun. These are named in the report's detail.

use crate::basic::Subject;
use crate::machine::{Config, FileServer, Machine};
use crate::report::{Environment, Outcome, TestResult};

/// The installed-package restart cap (GodspeedOS `installed.rs` `RESTARTS_MAX`): after this many restarts a
/// package is left down. A kill one past it must be the one left down.
pub const RESTARTS_MAX: u32 = 5;

const RECOVER: (&str, u32, &str) = ("S4.recover", 1, "each kill is recovered by the supervisor");
const BOUNDED: (&str, u32, &str) = ("S4.bounded", 1, "recovery is bounded, then left down loudly");
const CLEANUP: (&str, u32, &str) = ("S4.cleanup", 1, "every death reclaims the task's frames");
const BYSTANDERS: (&str, u32, &str) = ("S4.bystanders", 1, "the kernel and unrelated services survive");
const KERNEL: (&str, u32, &str) = ("S4.kernel", 1, "no kernel panic or wedge under carnage");

const TESTS: &[(&str, u32, &str)] = &[RECOVER, BOUNDED, CLEANUP, BYSTANDERS, KERNEL];

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

/// Every Stage 4 test as NOT RUN, with one reason.
pub fn not_run(env: &Environment, why: &str) -> Vec<TestResult> {
    TESTS
        .iter()
        .map(|t| result(*t, env, Outcome::NotRun, why, Vec::new()))
        .collect()
}

fn lines_with(text: &str, needle: &str) -> Vec<String> {
    text.lines()
        .filter(|l| l.contains(needle))
        .map(|l| l.trim_end().to_string())
        .collect()
}

fn tail(text: &str, n: usize) -> Vec<String> {
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    lines[lines.len().saturating_sub(n)..]
        .iter()
        .map(|l| l.trim_end().to_string())
        .collect()
}

/// The recovery, bounded and cleanup results, read from the slice of the serial log that the storm produced.
/// `task` is the driver's task name (`publisher/name`). Independent of the machine, so it can be tested on
/// written logs. `bystanders` is how the still-alive check came out (driven live by `run`).
pub fn from_log(output: &str, task: &str, bystanders: TestResult, env: &Environment) -> Vec<TestResult> {
    let mut out = Vec::new();

    // Each death the supervisor reported for this package, as a fault or a kill.
    let died = lines_with(output, &format!("installed {task} died"));
    let restarting: Vec<&String> = died.iter().filter(|l| l.contains("restarting (")).collect();
    let left_down = died.iter().find(|l| l.contains("left down"));

    // Recover: the supervisor restarted it, and it started again (a fresh `started` line after a kill).
    let restarted = output
        .lines()
        .filter(|l| l.contains(&format!("installed {task}")) && l.contains("started"))
        .count();
    out.push(if !restarting.is_empty() && restarted > 0 {
        result(
            RECOVER,
            env,
            Outcome::Pass,
            format!(
                "{} kill(s) recovered; the driver restarted and ran again",
                restarting.len()
            ),
            restarting.iter().take(2).map(|l| l.to_string()).collect(),
        )
    } else if restarting.is_empty() {
        result(
            RECOVER,
            env,
            Outcome::Fail,
            "the supervisor never reported restarting the driver after a kill",
            tail(output, 6),
        )
    } else {
        result(
            RECOVER,
            env,
            Outcome::Fail,
            "the supervisor said it was restarting, but the driver never started again",
            tail(output, 6),
        )
    });

    // Bounded: the restarts stopped at the cap, and the package was left down with a loud line.
    out.push(match left_down {
        Some(line) if restarting.len() as u32 == RESTARTS_MAX => result(
            BOUNDED,
            env,
            Outcome::Pass,
            format!("restarted {RESTARTS_MAX} times, then left down"),
            vec![line.to_string()],
        ),
        Some(line) => result(
            BOUNDED,
            env,
            Outcome::Fail,
            format!(
                "left down after {} restarts, not the cap of {RESTARTS_MAX}",
                restarting.len()
            ),
            vec![line.to_string()],
        ),
        None if restarting.len() as u32 > RESTARTS_MAX => result(
            BOUNDED,
            env,
            Outcome::Fail,
            format!(
                "restarted {} times and was never left down (unbounded)",
                restarting.len()
            ),
            tail(output, 6),
        ),
        None => result(
            BOUNDED,
            env,
            Outcome::NotRun,
            "the storm did not reach the restart cap, so the bound was not observed",
            tail(output, 4),
        ),
    });

    // Cleanup: every death reclaimed the task's frames (`freed N frames`, N > 0).
    let freed: Vec<&str> = output
        .lines()
        .filter(|l| l.contains("kill_task") && l.contains(&format!("'{task}'")) && l.contains("freed"))
        .collect();
    let frames = |l: &str| {
        l.split("freed")
            .nth(1)
            .and_then(|r| r.split_whitespace().next())
            .and_then(|n| n.parse::<u64>().ok())
            .unwrap_or(0)
    };
    let zero = freed.iter().filter(|l| frames(l) == 0).count();
    out.push(if freed.len() >= died.len() && !freed.is_empty() && zero == 0 {
        result(
            CLEANUP,
            env,
            Outcome::Pass,
            format!("{} death(s) each reclaimed the task's frames", freed.len()),
            freed.iter().take(2).map(|l| l.trim().to_string()).collect(),
        )
    } else if freed.is_empty() {
        result(
            CLEANUP,
            env,
            Outcome::Fail,
            "no `freed N frames` line for the driver's deaths: the kernel did not report reclaiming its memory",
            tail(output, 6),
        )
    } else {
        result(
            CLEANUP,
            env,
            Outcome::Fail,
            format!(
                "{zero} death(s) reclaimed 0 frames, or fewer reclaim lines than deaths ({} vs {})",
                freed.len(),
                died.len()
            ),
            freed.iter().take(3).map(|l| l.trim().to_string()).collect(),
        )
    });

    out.push(bystanders);
    out.push(kernel(output, env));
    out
}

/// `S4.kernel`, from a whole serial log.
pub fn kernel(output: &str, env: &Environment) -> TestResult {
    let stops: Vec<String> = ["KERNEL PANIC", "LIVENESS WEDGE"]
        .iter()
        .flat_map(|k| lines_with(output, k))
        .collect();
    if stops.is_empty() {
        result(
            KERNEL,
            env,
            Outcome::Pass,
            "no `KERNEL PANIC` or `LIVENESS WEDGE` under the storm",
            Vec::new(),
        )
    } else {
        let wedge = stops.iter().any(|l| l.contains("LIVENESS WEDGE"));
        let note = if wedge {
            " (a LIVENESS WEDGE is lateness, not a driver fault: rerun once before trusting it, CLAUDE.md backlog/49)"
        } else {
            ""
        };
        result(
            KERNEL,
            env,
            Outcome::Fail,
            format!("the kernel stopped under the storm{note}"),
            stops,
        )
    }
}

/// Run Stage 4 on its own fresh machine: install the subject, storm it, and measure. `env` names the
/// environment on every result; the serial log is left at `cfg.work/serial.log`.
pub fn run(
    cfg: &Config,
    s: &Subject,
    behaviour: Option<&str>,
    target_release: &str,
    env: &Environment,
) -> Vec<TestResult> {
    let mut m = match Machine::boot(cfg) {
        Ok(m) => m,
        Err(e) => return not_run(env, &format!("the Stage 4 machine did not start: {e}")),
    };
    if m.wait_for("gsh>", 180).is_none() || !m.text().contains("supervisor: ready") {
        let log = finish(m, cfg);
        let mut out = not_run(env, "the Stage 4 machine did not reach a ready prompt");
        *out.last_mut().unwrap() = kernel(&log, env);
        return out;
    }
    let _ = m.seen_since(0, "echo reply (ping OK)", 60);

    // The operator's install sequence (as Stage 3 does it), on this fresh machine.
    if let Err(why) = install(&mut m, s, target_release) {
        let log = finish(m, cfg);
        let mut out = not_run(env, &why);
        *out.last_mut().unwrap() = kernel(&log, env);
        return out;
    }
    let task = s.name.to_string();
    // Let it do its work once before the storm, so a kill lands on a running driver.
    if let Some(b) = behaviour {
        m.seen_since(0, b, 30);
    }

    // The storm: kill it one more time than the cap, waiting for each recovery, so the bound is reached.
    let storm_from = m.len();
    for round in 1..=RESTARTS_MAX + 1 {
        if m.type_line(&format!("kill {task}")).is_err() {
            break;
        }
        // Wait for the supervisor to report this death (a kill), then for the recovery or the left-down.
        if !m.seen_since(storm_from, &format!("installed {task} died"), 30) {
            break;
        }
        if round <= RESTARTS_MAX {
            // It should restart and come up again; give it a moment to, before the next kill.
            m.seen_since(m.len().saturating_sub(4096), &format!("installed {task}"), 20);
            std::thread::sleep(std::time::Duration::from_millis(500));
        } else {
            m.seen_since(storm_from, "left down", 20);
        }
    }
    let storm_log = m.text_from(storm_from);

    // The storm leaves a stack of stale `gsh>` prompts in the buffer; step past them so the bystander
    // commands match their own output, not a prompt printed mid-storm.
    m.drain();
    // Bystanders: the driver is down now; the rest of the machine must still work.
    let bystanders = bystanders(&mut m, env);
    let log = finish(m, cfg);
    // S4.kernel is derived from the WHOLE log, not just the storm slice.
    let mut out = from_log(&storm_log, &task, bystanders, env);
    *out.last_mut().unwrap() = kernel(&log, env);
    out
}

/// The operator's install sequence on a fresh machine: format the disk, fetch the package, trust its key,
/// install it, and see the supervisor start it. Returns where it stopped, if it did.
fn install(m: &mut Machine, s: &Subject, target_release: &str) -> Result<(), String> {
    let _ = target_release;
    m.type_line("drives flash 0 data").map_err(|e| e.to_string())?;
    if m.wait_for("[y/N]", 60).is_none() {
        return Err("`drives flash 0 data` asked nothing".to_string());
    }
    if !m
        .command("y", 120)
        .map(|o| o.contains("formatted as GSFS"))
        .unwrap_or(false)
    {
        return Err("the data disk was not formatted".to_string());
    }
    let server = FileServer::start(s.file_name, s.file.to_vec()).map_err(|e| e.to_string())?;
    if !m
        .command(&format!("download {}", server.link()), 180)
        .map(|o| o.contains("saved"))
        .unwrap_or(false)
    {
        return Err("the package did not download".to_string());
    }
    m.type_line(&format!(
        "godspeed trust add {} {}",
        s.name.split('/').next().unwrap_or(""),
        s.key
    ))
    .map_err(|e| e.to_string())?;
    if m.wait_for("[y/N]", 60).is_none() {
        return Err("`godspeed trust add` asked nothing".to_string());
    }
    if !m
        .command("y", 60)
        .map(|o| o.contains("trusted by this machine's operator"))
        .unwrap_or(false)
    {
        return Err("the key was not trusted".to_string());
    }
    let from = m.len();
    if m.command(&format!("godspeed install {} yes", s.file_name), 300)
        .is_none()
    {
        return Err("`godspeed install` did not return to the prompt".to_string());
    }
    if !m.seen_since(
        from,
        &format!("supervisor: installed {} {} started", s.name, s.version),
        60,
    ) {
        return Err("the supervisor did not start the driver".to_string());
    }
    Ok(())
}

/// After the storm: the shell answers, a file round-trips through `fs`, and `net-stack` still pings. The driver
/// being down is fine; this is about everything else.
fn bystanders(m: &mut Machine, env: &Environment) -> TestResult {
    let mut ok = Vec::new();
    let mut bad = Vec::new();
    let version = m.command("version", 60).unwrap_or_default();
    if version.contains("GodspeedOS") {
        ok.push("the shell answers `version`".to_string());
    } else {
        bad.push("the shell did not answer `version`".to_string());
    }
    let _ = m.command("write /s4.txt carnage-survived", 60);
    let read = m.command("read /s4.txt", 60).unwrap_or_default();
    if read.contains("carnage-survived") {
        ok.push("a file round-tripped through fs".to_string());
    } else {
        bad.push("a file did not round-trip through fs".to_string());
    }
    // `ping` runs until `q`, so wait for a reply rather than a prompt, then stop it and step past its output.
    let ping_from = m.len();
    let replied = m.type_line("ping 10.0.2.2").is_ok() && m.seen_since(ping_from, "Reply from 10.0.2.2", 30);
    let _ = m.type_line("q");
    m.wait_for("gsh>", 20);
    m.drain();
    if replied {
        ok.push("net-stack still pings the gateway".to_string());
    } else {
        bad.push("net-stack did not answer a ping".to_string());
    }
    if bad.is_empty() {
        result(
            BYSTANDERS,
            env,
            Outcome::Pass,
            "the shell, fs and net-stack all survived the storm",
            ok,
        )
    } else {
        result(BYSTANDERS, env, Outcome::Fail, bad.join("; "), ok)
    }
}

/// Stop the machine and return its whole serial log for the final kernel scan.
fn finish(m: Machine, _cfg: &Config) -> String {
    std::thread::sleep(std::time::Duration::from_secs(2));
    let log = m.text();
    m.stop();
    log
}
