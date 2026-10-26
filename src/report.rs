// SPDX-License-Identifier: Apache-2.0
//! What a run found, and the evidence for each part of it (SPECIFICATION.md 7.2).
//!
//! A result is one test, by identifier and version, with an outcome, the environment it ran in and the lines that
//! decided it. A result whose evidence is missing is not a pass: the code that builds one says `NotRun` instead.

use serde::Serialize;

/// The version of this suite, printed on every report.
pub const GDCS_VERSION: &str = env!("CARGO_PKG_VERSION");

/// The report's JSON format. Raised when a field changes meaning; `schema/report.schema.json` is its definition.
pub const REPORT_FORMAT: u32 = 1;

/// What one test showed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Outcome {
    /// The criterion was met, and the evidence says how.
    Pass,
    /// The criterion was not met.
    Fail,
    /// The test did not run, and the detail says why. Never counted as a pass.
    NotRun,
    /// The test ran, and this environment cannot show the property either way (QEMU's IOMMU does not fault).
    NotShown,
    /// The property cannot be enforced on this machine at all (DMA where no IOMMU confines the device).
    NotEnforceable,
    /// The driver does not claim it (live replacement, say). Not a failure.
    NotSupported,
    /// Worth knowing, not a failure: a portability or least-privilege note.
    Finding,
}

impl Outcome {
    /// The word a text report prints.
    pub fn word(self) -> &'static str {
        match self {
            Outcome::Pass => "PASS",
            Outcome::Fail => "FAIL",
            Outcome::NotRun => "NOT RUN",
            Outcome::NotShown => "NOT SHOWN",
            Outcome::NotEnforceable => "NOT ENFORCEABLE ON THIS MACHINE",
            Outcome::NotSupported => "NOT SUPPORTED",
            Outcome::Finding => "FINDING",
        }
    }
}

/// Where a test ran. Simulation and hardware are never merged into one result (SPECIFICATION.md 4).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Environment {
    /// On the package file alone: no machine involved.
    Offline,
    /// In an emulator. `detail` names it and the devices it was given.
    Simulated { detail: String },
    /// On a physical board. `detail` names the board, its firmware and the image.
    Hardware { detail: String },
}

impl Environment {
    /// One line for a text report.
    pub fn describe(&self) -> String {
        match self {
            Environment::Offline => "offline - the package file only".to_string(),
            Environment::Simulated { detail } => format!("simulated - {detail}"),
            Environment::Hardware { detail } => format!("hardware - {detail}"),
        }
    }
}

/// One test's result.
#[derive(Clone, Debug, Serialize)]
pub struct TestResult {
    /// Its identifier, `S1.signature`: the stage, then what it checks.
    pub id: &'static str,
    /// The version of its definition. A criterion that changes gets a new version, so two reports can be compared.
    pub version: u32,
    /// What it checks, in a few words.
    pub title: &'static str,
    /// What it showed.
    pub outcome: Outcome,
    /// One sentence: what was found, or why it did not run. Written to be acted on without the OS's source.
    pub detail: String,
    /// The lines that decided it: values read from the package, or lines from a serial log.
    pub evidence: Vec<String>,
    /// Where it ran.
    pub environment: Environment,
}

/// The package a report is about.
#[derive(Clone, Debug, Serialize)]
pub struct Subject {
    /// `publisher/name`, or the file name when the package could not be read.
    pub name: String,
    /// Its version, when the manifest could be read.
    pub version: Option<String>,
    /// The SHA-256 of the whole package file.
    pub sha256: String,
}

/// The release a run tested against.
#[derive(Clone, Debug, Serialize)]
pub struct Target {
    /// The GodspeedOS version the platform file names.
    pub os: String,
    /// The commit it names.
    pub commit: String,
    /// The SHA-256 of the platform file.
    pub platform_sha256: String,
}

/// A whole run.
#[derive(Clone, Debug, Serialize)]
pub struct Report {
    /// [`REPORT_FORMAT`].
    pub format: u32,
    /// [`GDCS_VERSION`].
    pub gdcs: &'static str,
    /// The profile and its version, `Basic 0.1`, or the stage when only one was asked for.
    pub profile: String,
    /// The package.
    pub subject: Subject,
    /// The release.
    pub target: Target,
    /// Every test, in the order it ran, those that did not run included.
    pub results: Vec<TestResult>,
}

/// The verdict a report ends with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Nothing failed and nothing required was left unrun.
    Conformant,
    /// Something failed.
    NotConformant,
    /// Nothing failed, but a required test did not run, so nothing can be claimed.
    Incomplete,
}

impl Report {
    /// The verdict: any failure fails, then any test left unrun makes the run incomplete. `NotShown`,
    /// `NotEnforceable`, `NotSupported` and findings are printed and never turn into a pass or a failure.
    pub fn verdict(&self) -> Verdict {
        if self.results.iter().any(|r| r.outcome == Outcome::Fail) {
            Verdict::NotConformant
        } else if self.results.iter().any(|r| r.outcome == Outcome::NotRun) {
            Verdict::Incomplete
        } else {
            Verdict::Conformant
        }
    }

    /// Where the results ran, as the verdict qualifies it: `offline`, `simulated`, `hardware`, or a mix.
    fn qualifier(&self) -> String {
        let mut kinds: Vec<&str> = Vec::new();
        for r in &self.results {
            let k = match r.environment {
                Environment::Offline => "offline",
                Environment::Simulated { .. } => "simulated",
                Environment::Hardware { .. } => "hardware",
            };
            if !kinds.contains(&k) {
                kinds.push(k);
            }
        }
        kinds.join(" + ")
    }

    /// The report as text, for a person.
    pub fn text(&self, verbose: bool) -> String {
        let mut s = String::new();
        s.push_str(&format!("Godspeed Driver Conformance Suite {}\n", self.gdcs));
        let version = self.subject.version.as_deref().unwrap_or("(unread)");
        s.push_str(&format!(
            "Driver:          {} {} (sha256 {})\n",
            self.subject.name,
            version,
            short(&self.subject.sha256)
        ));
        s.push_str(&format!(
            "Target:          GodspeedOS {} (platform file sha256 {})\n",
            self.target.os,
            short(&self.target.platform_sha256)
        ));
        s.push_str(&format!("Profile:         {}\n", self.profile));
        let mut envs: Vec<String> = Vec::new();
        for r in &self.results {
            let d = r.environment.describe();
            if !envs.contains(&d) {
                envs.push(d);
            }
        }
        for (i, e) in envs.iter().enumerate() {
            s.push_str(if i == 0 {
                "Environment:     "
            } else {
                "                 "
            });
            s.push_str(e);
            s.push('\n');
        }
        s.push('\n');
        let width = self.results.iter().map(|r| r.id.len()).max().unwrap_or(0);
        for r in &self.results {
            s.push_str(&format!(
                "{:width$}  {:<9}  {}\n",
                r.id,
                r.outcome.word(),
                r.title,
                width = width
            ));
            if r.outcome != Outcome::Pass || verbose {
                s.push_str(&format!("{:width$}             {}\n", "", r.detail, width = width));
            }
            if verbose {
                for e in &r.evidence {
                    s.push_str(&format!("{:width$}               | {}\n", "", e, width = width));
                }
            }
        }
        s.push('\n');
        let count = |o: Outcome| self.results.iter().filter(|r| r.outcome == o).count();
        s.push_str(&format!(
            "{} run: {} passed, {} failed, {} not run, {} not shown or not enforceable, {} findings\n",
            self.results.len(),
            count(Outcome::Pass),
            count(Outcome::Fail),
            count(Outcome::NotRun),
            count(Outcome::NotShown) + count(Outcome::NotEnforceable),
            count(Outcome::Finding)
        ));
        let physical = self
            .results
            .iter()
            .any(|r| matches!(r.environment, Environment::Hardware { .. }));
        s.push_str(&format!(
            "Physical hardware: {}\n",
            if physical { "tested" } else { "not tested" }
        ));
        let q = self.qualifier();
        s.push_str(&match self.verdict() {
            Verdict::Conformant => format!("RESULT: CONFORMANT ({q})\n"),
            Verdict::NotConformant => format!("RESULT: NOT CONFORMANT ({q})\n"),
            Verdict::Incomplete => format!("RESULT: INCOMPLETE ({q}) - a required test did not run\n"),
        });
        s
    }

    /// The report as JSON, for a tool. `schema/report.schema.json` describes it.
    pub fn json(&self) -> String {
        let mut v = serde_json::to_value(self).expect("a report always serialises");
        let verdict = match self.verdict() {
            Verdict::Conformant => "conformant",
            Verdict::NotConformant => "not-conformant",
            Verdict::Incomplete => "incomplete",
        };
        v["verdict"] = serde_json::Value::String(verdict.to_string());
        serde_json::to_string_pretty(&v).expect("a report always serialises") + "\n"
    }
}

/// The first 12 hex digits of a digest, then `...`, as a text report shows one.
fn short(hex: &str) -> String {
    if hex.len() > 12 {
        format!("{}...", &hex[..12])
    } else {
        hex.to_string()
    }
}

/// Lowercase hex of `bytes`.
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
