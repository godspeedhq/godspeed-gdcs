// SPDX-License-Identifier: Apache-2.0
//! `gdcs`: run the Godspeed Driver Conformance Suite on a driver package.

#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use gdcs::report::{hex, Environment, LogFile, Outcome, Report, Target, Verdict, GDCS_VERSION, REPORT_FORMAT};
use godspeed_package::{Arch, ManifestSignature, Sha256Digest};

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
    /// The Basic profile: Stage 1, then the driver installed on a release image in QEMU and watched (Stages 2, 3).
    /// With --recovery, also Stage 4 (MAX CARNAGE) on a second machine. x86-64 only, simulated.
    Run {
        /// The driver package, `<name>-<version>.gsp`.
        package: PathBuf,
        /// The release's `godspeed-platform.toml`.
        #[arg(long)]
        platform: PathBuf,
        /// The release's x86-64 image, unpacked (`godspeed-os-<tag>-x86_64.img`). It is copied, never written.
        #[arg(long)]
        image: PathBuf,
        /// The driver's project folder: its source for Stage 1, and its `gdcs.toml` unless --declare names another.
        #[arg(long)]
        source: Option<PathBuf>,
        /// The driver's declaration: what it claims to do, and the QEMU devices it needs.
        #[arg(long)]
        declare: Option<PathBuf>,
        /// Where the report, its JSON and the serial log are written.
        #[arg(long, default_value = "gdcs-run")]
        out: PathBuf,
        /// `qemu-system-x86_64`, if it is not on PATH.
        #[arg(long, default_value = "qemu-system-x86_64")]
        qemu: PathBuf,
        /// QEMU's x86-64 UEFI firmware, if it is not beside QEMU (`share/edk2-x86_64-code.fd`).
        #[arg(long)]
        firmware: Option<PathBuf>,
        /// Cores.
        #[arg(long, default_value_t = 2)]
        smp: u32,
        /// Also run Stage 4 (MAX CARNAGE) on a second machine: the Recovery profile. Adds a few minutes.
        #[arg(long)]
        recovery: bool,
        /// Print every result's detail and evidence, passes included.
        #[arg(long, short)]
        verbose: bool,
    },
}

/// Exit codes: 0 conformant, 1 not conformant, 2 the run could not start, 3 incomplete (a required test did not run).
fn main() -> ExitCode {
    match Cli::parse().command {
        Command::Check {
            package,
            platform,
            source,
            json,
            verbose,
        } => {
            let (platform, file, name) = match inputs(&platform, &package) {
                Ok(x) => x,
                Err(e) => return fail(&e),
            };
            // If the driver's gdcs.toml names a target release, it must match the platform file given.
            if let Some(dir) = &source {
                match load_declaration(&None, &Some(dir.clone())) {
                    Err(e) => return fail(&e),
                    Ok(decl) => {
                        if let Some(why) = decl.target_mismatch(&platform.os) {
                            return fail(&why);
                        }
                    }
                }
            }
            let (subject, results) = gdcs::stage1::check(&name, &file, &platform, source.as_deref());
            let report = Report {
                format: REPORT_FORMAT,
                gdcs: GDCS_VERSION,
                profile: "Stage 1, contract compliance".to_string(),
                subject,
                target: target(&platform),
                results,
                logs: Vec::new(),
            };
            print!("{}", report.text(verbose));
            if let Some(path) = json {
                if let Err(e) = std::fs::write(&path, report.json()) {
                    return fail(&format!("{}: {e}", path.display()));
                }
            }
            exit(&report)
        }
        Command::Run {
            package,
            platform,
            image,
            source,
            declare,
            out,
            qemu,
            firmware,
            smp,
            recovery,
            verbose,
        } => {
            let (platform, file, name) = match inputs(&platform, &package) {
                Ok(x) => x,
                Err(e) => return fail(&e),
            };
            let decl_path = declare
                .clone()
                .or_else(|| source.as_ref().map(|s| s.join("gdcs.toml")))
                .filter(|p| p.exists());
            let decl = match load_declaration(&declare, &source) {
                Ok(d) => d,
                Err(e) => return fail(&e),
            };
            // If the driver's gdcs.toml names a target release, it must match the platform file given, before boot.
            if let Some(why) = decl.target_mismatch(&platform.os) {
                return fail(&why);
            }
            let Some(firmware) = firmware.or_else(|| gdcs::machine::default_firmware(&qemu)) else {
                return fail(
                    "no UEFI firmware found beside QEMU (share/edk2-x86_64-code.fd): name one with --firmware",
                );
            };
            if let Err(e) = std::fs::create_dir_all(&out) {
                return fail(&format!("{}: {e}", out.display()));
            }
            let image_bytes_sha = match std::fs::read(&image) {
                Ok(b) => hex(&Sha256Digest::of(&b).0),
                Err(e) => return fail(&format!("{}: {e}", image.display())),
            };

            let (subject, mut results) = gdcs::stage1::check(&name, &file, &platform, source.as_deref());
            let devices = if decl.qemu.devices.is_empty() {
                "none".to_string()
            } else {
                decl.qemu.devices.join(", ")
            };
            let env = Environment::Simulated {
                detail: format!(
                    "{}, pc board, UEFI, {smp} cores, e1000, AHCI data disk; devices: {devices}; image {} (sha256 {}...)",
                    gdcs::machine::qemu_version(&qemu),
                    image.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                    &image_bytes_sha[..12]
                ),
            };
            let stage1_failed = results.iter().any(|r| r.outcome == Outcome::Fail);
            let mut logs = Vec::new();
            if stage1_failed {
                results.extend(gdcs::basic::not_run(
                    &env,
                    "Stage 1 failed: a package refused offline is not booted",
                ));
                if recovery {
                    results.extend(gdcs::carnage::not_run(
                        &env,
                        "Stage 1 failed: a package refused offline is not booted",
                    ));
                }
            } else {
                match basic_subject(&file) {
                    Err(why) => {
                        results.extend(gdcs::basic::not_run(&env, &why));
                        if recovery {
                            results.extend(gdcs::carnage::not_run(&env, &why));
                        }
                    }
                    Ok((key, dma)) => {
                        let s = gdcs::basic::Subject {
                            name: &subject.name,
                            version: subject.version.as_deref().unwrap_or(""),
                            key: &key,
                            file: &file,
                            file_name: &name,
                            dma,
                        };
                        let cfg = gdcs::machine::Config {
                            qemu: qemu.clone(),
                            firmware,
                            image: image.clone(),
                            work: out.join("machine"),
                            devices: decl.qemu.devices.clone(),
                            smp,
                        };
                        eprintln!(
                            "gdcs: Stage 1 passed; booting {} in QEMU (a few minutes)",
                            image.display()
                        );
                        let probes = gdcs::isolation::bundled_probes();
                        results.extend(gdcs::basic::run(&cfg, &decl, &s, Some(&probes), &platform.os, &env));
                        let serial = cfg.work.join("serial.log");
                        if let Ok(b) = std::fs::read(&serial) {
                            let kept = out.join("serial.log");
                            if std::fs::write(&kept, &b).is_ok() {
                                logs.push(LogFile {
                                    name: "serial.log".to_string(),
                                    sha256: hex(&Sha256Digest::of(&b).0),
                                });
                            }
                        }
                        if recovery {
                            // Stage 4 on its OWN fresh machine, so the restart cap starts at zero and the
                            // storm interferes with nothing else. No extra QEMU devices: the carnage is kills,
                            // not the device, so the subject does its one-shot work and then is stormed.
                            let carnage_cfg = gdcs::machine::Config {
                                work: out.join("carnage"),
                                devices: decl.qemu.devices.clone(),
                                ..cfg.clone()
                            };
                            let behaviour = decl.behaviour.first().map(|b| b.expect.as_str());
                            eprintln!("gdcs: --recovery; booting a second machine for Stage 4 (MAX CARNAGE)");
                            results.extend(gdcs::carnage::run(&carnage_cfg, &s, behaviour, &platform.os, &env));
                            if let Ok(b) = std::fs::read(carnage_cfg.work.join("serial.log")) {
                                if std::fs::write(out.join("serial-carnage.log"), &b).is_ok() {
                                    logs.push(LogFile {
                                        name: "serial-carnage.log".to_string(),
                                        sha256: hex(&Sha256Digest::of(&b).0),
                                    });
                                }
                            }
                        }
                    }
                }
            }
            if let Some(p) = decl_path.filter(|p| p.exists()) {
                let b = std::fs::read(&p).unwrap_or_default();
                if std::fs::write(out.join("gdcs.toml"), &b).is_ok() {
                    logs.push(LogFile {
                        name: "gdcs.toml".to_string(),
                        sha256: hex(&Sha256Digest::of(&b).0),
                    });
                }
            }
            let report = Report {
                format: REPORT_FORMAT,
                gdcs: GDCS_VERSION,
                profile: if recovery {
                    "Recovery 0.1 (Stages 1-4)".to_string()
                } else {
                    "Basic 0.1 (Stages 1-3)".to_string()
                },
                subject,
                target: target(&platform),
                results,
                logs,
            };
            let text = report.text(verbose);
            print!("{text}");
            for (f, body) in [("report.txt", report.text(true)), ("report.json", report.json())] {
                if let Err(e) = std::fs::write(out.join(f), body) {
                    return fail(&format!("{}: {e}", out.join(f).display()));
                }
            }
            eprintln!("gdcs: report, JSON and logs in {}", out.display());
            exit(&report)
        }
    }
}

/// The platform file and the package file, read.
fn inputs(platform: &Path, package: &Path) -> Result<(gdcs::platform::Platform, Vec<u8>, String), String> {
    let pb = std::fs::read(platform).map_err(|e| format!("{}: {e}", platform.display()))?;
    let p = gdcs::platform::read(&pb).map_err(|e| format!("{}: {e}", platform.display()))?;
    let file = std::fs::read(package).map_err(|e| format!("{}: {e}", package.display()))?;
    let name = package
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    Ok((p, file, name))
}

/// The driver's declaration: `--declare`, else `<source>/gdcs.toml`, else an empty declaration. An error is a
/// gdcs.toml that is present but does not read.
fn load_declaration(declare: &Option<PathBuf>, source: &Option<PathBuf>) -> Result<gdcs::declare::Declaration, String> {
    let path = declare.clone().or_else(|| source.as_ref().map(|s| s.join("gdcs.toml")));
    match path {
        Some(p) if p.exists() => std::fs::read_to_string(&p)
            .map_err(|e| format!("{}: {e}", p.display()))
            .and_then(|t| gdcs::declare::read(&t).map_err(|e| format!("{}: {e}", p.display()))),
        _ => Ok(gdcs::declare::Declaration {
            format: gdcs::declare::FORMAT,
            ..Default::default()
        }),
    }
}

fn target(p: &gdcs::platform::Platform) -> Target {
    Target {
        os: p.os.clone(),
        commit: p.commit.clone(),
        platform_sha256: p.sha256.clone(),
    }
}

/// What the machine run needs from a package Stage 1 passed: the key that signed it, and whether it asks for DMA.
/// An error is why the run cannot go ahead.
fn basic_subject(file: &[u8]) -> Result<(String, bool), String> {
    let pkg = gdcs::package::read(file)?;
    let sig = pkg
        .own(godspeed_package::SIGNATURE_FILE)
        .and_then(|b| std::str::from_utf8(b).ok())
        .unwrap_or("");
    let key = ManifestSignature::parse(sig)
        .map_err(|e| e.to_string())?
        .key
        .to_string();
    if pkg.own(Arch::X86_64.artifact_file()).is_none() {
        return Err("the package has no x86_64 binary, and this run's machine is x86-64".to_string());
    }
    let contract: toml::Table = pkg
        .own(godspeed_package::CONTRACT_FILE)
        .and_then(|b| std::str::from_utf8(b).ok())
        .and_then(|t| t.parse().ok())
        .unwrap_or_default();
    let dma = contract.get("capabilities").and_then(|c| c.get("hw_dma")).is_some();
    Ok((key, dma))
}

fn exit(report: &Report) -> ExitCode {
    match report.verdict() {
        Verdict::Conformant => ExitCode::SUCCESS,
        Verdict::NotConformant => ExitCode::from(1),
        Verdict::Incomplete => ExitCode::from(3),
    }
}

fn fail(msg: &str) -> ExitCode {
    eprintln!("gdcs: {msg}");
    ExitCode::from(2)
}
