// SPDX-License-Identifier: Apache-2.0
//! `gdcs`: run the Godspeed Driver Conformance Suite on a driver package.

#![forbid(unsafe_code)]

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use gdcs::report::{Report, Target, Verdict, GDCS_VERSION, REPORT_FORMAT};

#[derive(Parser)]
#[command(name = "gdcs", version, about = "The Godspeed Driver Conformance Suite")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Stage 1, contract compliance: the package file alone, against a release's platform file. No machine.
    Check {
        /// The driver package, `<name>-<version>.gsp`.
        package: PathBuf,
        /// The release's `godspeed-platform.toml`, from its GitHub release.
        #[arg(long)]
        platform: PathBuf,
        /// The driver's project folder, for the two tests a binary cannot answer (no `unsafe`, SDK use).
        #[arg(long)]
        source: Option<PathBuf>,
        /// Also write the report as JSON to this file.
        #[arg(long)]
        json: Option<PathBuf>,
        /// Print every result's detail and evidence, passes included.
        #[arg(long, short)]
        verbose: bool,
    },
}

/// Exit codes: 0 conformant, 1 not conformant, 2 the run could not start, 3 incomplete (a required test did not run).
fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Command::Check {
            package,
            platform,
            source,
            json,
            verbose,
        } => {
            let platform_bytes = match std::fs::read(&platform) {
                Ok(b) => b,
                Err(e) => return fail(&format!("{}: {e}", platform.display())),
            };
            let platform = match gdcs::platform::read(&platform_bytes) {
                Ok(p) => p,
                Err(e) => return fail(&format!("{}: {e}", platform.display())),
            };
            let file = match std::fs::read(&package) {
                Ok(b) => b,
                Err(e) => return fail(&format!("{}: {e}", package.display())),
            };
            let name = package
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let (subject, results) = gdcs::stage1::check(&name, &file, &platform, source.as_deref());
            let report = Report {
                format: REPORT_FORMAT,
                gdcs: GDCS_VERSION,
                profile: "Stage 1, contract compliance".to_string(),
                subject,
                target: Target {
                    os: platform.os.clone(),
                    commit: platform.commit.clone(),
                    platform_sha256: platform.sha256.clone(),
                },
                results,
            };
            print!("{}", report.text(verbose));
            if let Some(path) = json {
                if let Err(e) = std::fs::write(&path, report.json()) {
                    return fail(&format!("{}: {e}", path.display()));
                }
            }
            match report.verdict() {
                Verdict::Conformant => ExitCode::SUCCESS,
                Verdict::NotConformant => ExitCode::from(1),
                Verdict::Incomplete => ExitCode::from(3),
            }
        }
    }
}

fn fail(msg: &str) -> ExitCode {
    eprintln!("gdcs: {msg}");
    ExitCode::from(2)
}
