// SPDX-License-Identifier: Apache-2.0
//! Stage 1, contract compliance (SPECIFICATION.md 4): what can be shown from the package file, the release's
//! platform file and, when given, the driver's source - with no machine involved.
//!
//! Every rule here is one the system enforces somewhere: the package format's own, the contract schema the release
//! publishes, or a refusal the supervisor makes at install (`docs/developer-services-design.md` 13.1 in GodspeedOS).
//! Checking them here is the point: a package that passes every offline check and is refused only on the machine
//! is a GDCS defect (SPECIFICATION.md 4, Stage 1).
//!
//! Each test is listed in [`TESTS`], and `tests/stage1.rs` holds, for every one, a package built to fail it: a
//! check is not counted until it has been seen to fail.

use std::path::Path;

use godspeed_package::{Arch, Kind, ManifestSignature, OwnedManifest, Sha256Digest};

use crate::package::{self, PackageFile};
use crate::platform::Platform;
use crate::report::{hex, Environment, Outcome, Subject, TestResult};

/// One test's definition: identifier, version, and what it checks.
#[derive(Clone, Copy, Debug)]
pub struct TestDef {
    /// `S1.<what>`.
    pub id: &'static str,
    /// Raised when its criterion changes.
    pub version: u32,
    /// What it checks.
    pub title: &'static str,
    /// The acceptance criterion, stated before the test is run.
    pub criterion: &'static str,
}

/// The file is a package file holding exactly one package.
pub const CONTAINER: TestDef = TestDef {
    id: "S1.container",
    version: 1,
    title: "a package file holding one package",
    criterion: "the file is framed as a .gsp (header, entries, `end`, nothing after) and holds exactly one manifest",
};
/// The manifest is signed, and the signature covers it.
pub const SIGNATURE: TestDef = TestDef {
    id: "S1.signature",
    version: 1,
    title: "manifest signature",
    criterion: "manifest.sig is in its canonical form and verifies, strictly, over the manifest's exact bytes",
};
/// The manifest reads and keeps the format's rules, and agrees with its contract.
pub const MANIFEST: TestDef = TestDef {
    id: "S1.manifest",
    version: 1,
    title: "manifest",
    criterion: "the manifest parses, breaks no rule of the format, and names the contract's own name and version",
};
/// Every byte the manifest names is in the file, and nothing else is.
pub const DIGESTS: TestDef = TestDef {
    id: "S1.digests",
    version: 1,
    title: "contract and binaries match the manifest",
    criterion: "the contract and every binary are present with the size and SHA-256 the manifest names, and the \
                package's folder holds nothing the manifest does not name",
};
/// It is a driver.
pub const KIND: TestDef = TestDef {
    id: "S1.kind",
    version: 1,
    title: "package kind",
    criterion: "the manifest's kind is `driver`",
};
/// It was built for the release this run targets.
pub const TARGET: TestDef = TestDef {
    id: "S1.target",
    version: 1,
    title: "built for the target release",
    criterion: "the manifest's `[os] release` is the release the platform file names",
};
/// Its binaries are programs for the architectures they claim.
pub const BINARIES: TestDef = TestDef {
    id: "S1.binaries",
    version: 1,
    title: "a program for each architecture claimed",
    criterion: "each binary is an executable ELF of the class and machine its architecture names, with an entry point",
};
/// The contract keeps the release's schema.
pub const SCHEMA: TestDef = TestDef {
    id: "S1.schema",
    version: 1,
    title: "contract schema",
    criterion: "the contract is TOML the release's published service contract schema accepts",
};
/// One device, with only the keys its bus allows.
pub const DEVICE: TestDef = TestDef {
    id: "S1.device",
    version: 1,
    title: "one device, named by class",
    criterion: "the contract names exactly one device - `hw_pci_class` or `hw_usb` - and a USB driver asks for no \
                BAR, interrupt or DMA arena of its own (they are its host's)",
};
/// The DMA arena is within what a package is given.
pub const DMA: TestDef = TestDef {
    id: "S1.dma",
    version: 1,
    title: "DMA arena within 2 MiB",
    criterion: "`hw_dma`, when present, is a size in KiB or MiB of at most 2 MiB",
};
/// It asks for no authority an installed package cannot be given.
pub const AUTHORITY: TestDef = TestDef {
    id: "S1.authority",
    version: 1,
    title: "no privilege requested",
    criterion: "the contract's capabilities hold only `log_write`, `ipc_send`, `ipc_receive` and the device keys; \
                any other request is one no installed package is given",
};
/// Its send peers are ones the supervisor can grant.
pub const PEERS: TestDef = TestDef {
    id: "S1.peers",
    version: 1,
    title: "send peers",
    criterion: "at most 8 send peers, never `supervisor`, and every package peer (`publisher/name`) required by the \
                manifest; each peer is listed so a reviewer can ask why",
};
/// No `unsafe` of its own.
pub const UNSAFE: TestDef = TestDef {
    id: "S1.unsafe",
    version: 1,
    title: "no unsafe code of its own",
    criterion: "the crate root denies or forbids `unsafe_code`, no source file holds an `unsafe` block, function, \
                impl or trait, and `unsafe_code` is allowed only on the one `#[no_mangle]` entry",
};
/// Whether it reaches past `gs::driver` to the SDK.
pub const SDK: TestDef = TestDef {
    id: "S1.sdk",
    version: 1,
    title: "only the promised interfaces",
    criterion: "a finding, never a failure: the source's direct uses of `godspeed_sdk`, which the compatibility \
                promise does not cover",
};

/// Every Stage 1 test, in the order they run.
pub const TESTS: &[TestDef] = &[
    CONTAINER, SIGNATURE, MANIFEST, DIGESTS, KIND, TARGET, BINARIES, SCHEMA, DEVICE, DMA, AUTHORITY, PEERS, UNSAFE, SDK,
];

/// The largest DMA arena an installed driver is given.
pub const DMA_MAX: u64 = 2 * 1024 * 1024;
/// The most send peers an installed package may name.
pub const PEERS_MAX: usize = 8;
/// The capability keys a driver package may ask for. Anything else under `[capabilities]` is authority an
/// installed package is never given, so asking for it is refused at install or is a request that cannot be met.
pub const DRIVER_KEYS: &[&str] = &[
    "log_write",
    "ipc_send",
    "ipc_receive",
    "hw_pci_class",
    "hw_pci_bar",
    "hw_pci_irq",
    "hw_usb",
    "hw_dma",
];

fn result(def: TestDef, outcome: Outcome, detail: impl Into<String>, evidence: Vec<String>) -> TestResult {
    TestResult {
        id: def.id,
        version: def.version,
        title: def.title,
        outcome,
        detail: detail.into(),
        evidence,
        environment: Environment::Offline,
    }
}

fn not_run(def: TestDef, why: &str) -> TestResult {
    result(def, Outcome::NotRun, why, Vec::new())
}

/// Run Stage 1 on `file` (the package file's bytes, named `file_name`) against `platform`; `source` is the
/// driver's project folder, when given, for the two tests a binary cannot answer.
pub fn check(file_name: &str, file: &[u8], platform: &Platform, source: Option<&Path>) -> (Subject, Vec<TestResult>) {
    let mut subject = Subject {
        name: file_name.to_string(),
        version: None,
        sha256: hex(&Sha256Digest::of(file).0),
    };
    let mut out = Vec::new();

    let pkg = match package::read(file) {
        Ok(p) => {
            let evidence = p
                .entries
                .iter()
                .map(|e| format!("entry {} {}", e.bytes.len(), e.path))
                .collect();
            out.push(result(
                CONTAINER,
                Outcome::Pass,
                format!("{} entries; the package is `{}`", p.entries.len(), p.dir),
                evidence,
            ));
            p
        }
        Err(e) => {
            out.push(result(CONTAINER, Outcome::Fail, e, Vec::new()));
            for def in &TESTS[1..] {
                out.push(source_or_not_run(*def, source, "needs a readable package file"));
            }
            return (subject, out);
        }
    };

    out.push(signature(&pkg));

    // The manifest. The tests after it need it read, though not valid: a rule it breaks is its own failure.
    let manifest_bytes = pkg.own("manifest.toml").unwrap_or_default();
    let owned = std::str::from_utf8(manifest_bytes)
        .map_err(|_| "the manifest is not UTF-8 text".to_string())
        .and_then(|t| godspeed_package::parse(t).map_err(|e| format!("the manifest does not read: {e}")));
    let contract_text = pkg
        .own(godspeed_package::CONTRACT_FILE)
        .and_then(|b| std::str::from_utf8(b).ok());
    let contract: Option<toml::Table> = contract_text.and_then(|t| t.parse().ok());
    let owned = match owned {
        Ok(m) => m,
        Err(e) => {
            out.push(result(MANIFEST, Outcome::Fail, e, Vec::new()));
            for def in &TESTS[3..] {
                out.push(source_or_not_run(*def, source, "needs a manifest that reads"));
            }
            return (subject, out);
        }
    };
    let m = owned.manifest();
    subject.name = m.name.to_string();
    subject.version = Some(m.version.to_string());
    out.push(manifest(&owned, contract.as_ref()));
    out.push(digests(&pkg, &owned));

    out.push(if m.kind == Kind::Driver {
        result(
            KIND,
            Outcome::Pass,
            "a driver",
            vec![format!("kind = \"{}\"", m.kind.as_str())],
        )
    } else {
        result(
            KIND,
            Outcome::Fail,
            format!(
                "a `{}` package: GDCS tests drivers (`godspeed new driver` makes one)",
                m.kind.as_str()
            ),
            vec![format!("kind = \"{}\"", m.kind.as_str())],
        )
    });

    let built = m.os_release.to_string();
    let evidence = vec![
        format!("[os] release = \"{built}\""),
        format!("platform file: os = \"{}\"", platform.os),
    ];
    out.push(if built == platform.os {
        result(TARGET, Outcome::Pass, format!("built for {built}"), evidence)
    } else {
        result(
            TARGET,
            Outcome::Fail,
            format!(
                "built for {built}, and this run targets {}: build it against {}'s platform file, or test it \
                 against the release it was built for",
                platform.os, platform.os
            ),
            evidence,
        )
    });

    out.push(binaries(&pkg, &owned));

    match (contract_text, contract.as_ref()) {
        (Some(_), Some(c)) => {
            out.push(schema(c, platform));
            let caps = c
                .get("capabilities")
                .and_then(|v| v.as_table())
                .cloned()
                .unwrap_or_default();
            out.push(device(&caps));
            out.push(dma(&caps));
            out.push(authority(&caps));
            out.push(peers(&caps, &owned));
        }
        (None, _) => {
            for def in [SCHEMA, DEVICE, DMA, AUTHORITY, PEERS] {
                out.push(if def.id == SCHEMA.id {
                    result(
                        SCHEMA,
                        Outcome::Fail,
                        "the package holds no contract.toml, or it is not UTF-8 text",
                        Vec::new(),
                    )
                } else {
                    not_run(def, "needs the contract")
                });
            }
        }
        (Some(t), None) => {
            let why = match t.parse::<toml::Table>() {
                Err(e) => format!("the contract is not TOML: {}", e.message()),
                Ok(_) => "the contract is not TOML".to_string(),
            };
            out.push(result(SCHEMA, Outcome::Fail, why, Vec::new()));
            for def in [DEVICE, DMA, AUTHORITY, PEERS] {
                out.push(not_run(def, "needs a contract that reads"));
            }
        }
    }

    out.push(match source {
        Some(dir) => crate::source::unsafe_code(UNSAFE, dir),
        None => not_run(
            UNSAFE,
            "needs the driver's source (--source <project folder>): a package carries binaries only",
        ),
    });
    out.push(match source {
        Some(dir) => crate::source::sdk_use(SDK, dir),
        None => not_run(
            SDK,
            "needs the driver's source (--source <project folder>): a package carries binaries only",
        ),
    });
    (subject, out)
}

/// A test that does not run because the package could not be read - except the two that read the source, which
/// do not need the package.
fn source_or_not_run(def: TestDef, source: Option<&Path>, why: &str) -> TestResult {
    match (def.id, source) {
        (id, Some(dir)) if id == UNSAFE.id => crate::source::unsafe_code(UNSAFE, dir),
        (id, Some(dir)) if id == SDK.id => crate::source::sdk_use(SDK, dir),
        (id, None) if id == UNSAFE.id || id == SDK.id => not_run(
            def,
            "needs the driver's source (--source <project folder>): a package carries binaries only",
        ),
        _ => not_run(def, why),
    }
}

fn signature(pkg: &PackageFile) -> TestResult {
    let Some(manifest) = pkg.own("manifest.toml") else {
        return result(SIGNATURE, Outcome::Fail, "no manifest.toml", Vec::new());
    };
    let Some(sig) = pkg.own(godspeed_package::SIGNATURE_FILE) else {
        return result(
            SIGNATURE,
            Outcome::Fail,
            "no manifest.sig: the package is unsigned",
            Vec::new(),
        );
    };
    let parsed = std::str::from_utf8(sig)
        .map_err(|_| "manifest.sig is not text".to_string())
        .and_then(|t| ManifestSignature::parse(t).map_err(|e| format!("manifest.sig does not read: {e}")));
    let sig = match parsed {
        Ok(s) => s,
        Err(e) => return result(SIGNATURE, Outcome::Fail, e, Vec::new()),
    };
    let evidence = vec![format!("key = \"{}\"", sig.key)];
    match sig.verify(manifest) {
        Ok(()) => result(
            SIGNATURE,
            Outcome::Pass,
            format!(
                "signed by {}; whether this machine trusts that key is the install's question, not this one",
                sig.key
            ),
            evidence,
        ),
        Err(e) => result(
            SIGNATURE,
            Outcome::Fail,
            format!("the signature does not cover this manifest: {e}"),
            evidence,
        ),
    }
}

fn manifest(owned: &OwnedManifest, contract: Option<&toml::Table>) -> TestResult {
    let m = owned.manifest();
    let mut problems = Vec::new();
    m.validate(|p| problems.push(p.to_string()));
    match contract {
        Some(c) => {
            let name = c.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let version = c.get("version").and_then(|v| v.as_str()).unwrap_or("");
            m.validate_against_contract(name, version, |p| problems.push(p.to_string()));
        }
        None => problems
            .push("the contract is missing or does not read, so its name and version were not compared".to_string()),
    }
    let evidence = vec![
        format!("name = \"{}\"", m.name),
        format!("version = \"{}\"", m.version),
        format!("format = {}", m.format),
    ];
    if problems.is_empty() {
        result(MANIFEST, Outcome::Pass, format!("{} {}", m.name, m.version), evidence)
    } else {
        result(MANIFEST, Outcome::Fail, problems.join("; "), evidence)
    }
}

fn digests(pkg: &PackageFile, owned: &OwnedManifest) -> TestResult {
    let m = owned.manifest();
    let mut bad = Vec::new();
    let mut evidence = Vec::new();
    match pkg.own(godspeed_package::CONTRACT_FILE) {
        None => bad.push("no contract.toml".to_string()),
        Some(c) => {
            let d = Sha256Digest::of(c);
            evidence.push(format!("contract.toml sha256 {}", hex(&d.0)));
            if d != m.contract {
                bad.push(format!(
                    "contract.toml is sha256 {}, and the manifest names {}",
                    hex(&d.0),
                    hex(&m.contract.0)
                ));
            }
        }
    }
    let mut named = vec![
        "manifest.toml".to_string(),
        "manifest.sig".to_string(),
        godspeed_package::CONTRACT_FILE.to_string(),
    ];
    for a in m.artifacts {
        let file = a.arch.artifact_file();
        named.push(file.to_string());
        match pkg.own(file) {
            None => bad.push(format!(
                "the manifest names a {} binary, and there is no {file}",
                a.arch.as_str()
            )),
            Some(b) => {
                evidence.push(format!(
                    "{file} {} bytes sha256 {}",
                    b.len(),
                    hex(&Sha256Digest::of(b).0)
                ));
                if let Err(e) = godspeed_package::ArtifactVerifier::verify(*a, b) {
                    bad.push(format!("{file}: {e}"));
                }
            }
        }
    }
    for e in &pkg.entries {
        if let Some(rest) = e.path.strip_prefix(&pkg.dir) {
            if !named.iter().any(|n| n == rest) {
                bad.push(format!(
                    "{} is in the package's folder and the manifest does not name it",
                    e.path
                ));
            }
        } else if !(e.path.starts_with("trust/") || e.path.starts_with("index/")) {
            bad.push(format!(
                "{} is neither the package's, its index nor the registry's trust chain",
                e.path
            ));
        }
    }
    if bad.is_empty() {
        result(
            DIGESTS,
            Outcome::Pass,
            format!("the contract and {} binary(s) match", m.artifacts.len()),
            evidence,
        )
    } else {
        result(DIGESTS, Outcome::Fail, bad.join("; "), evidence)
    }
}

/// ELF class (1 = 32-bit, 2 = 64-bit) and machine for each architecture.
fn elf_shape(arch: Arch) -> (u8, u16) {
    match arch {
        Arch::X86_64 => (2, 62),
        Arch::Aarch64 => (2, 183),
        Arch::Armv7 => (1, 40),
        Arch::Riscv64 => (2, 243),
    }
}

fn binaries(pkg: &PackageFile, owned: &OwnedManifest) -> TestResult {
    let m = owned.manifest();
    let mut bad = Vec::new();
    let mut evidence = Vec::new();
    for a in m.artifacts {
        let file = a.arch.artifact_file();
        let Some(b) = pkg.own(file) else {
            bad.push(format!("no {file}"));
            continue;
        };
        let (class, machine) = elf_shape(a.arch);
        if b.len() < 52 || &b[..4] != b"\x7fELF" {
            bad.push(format!("{file} is not an ELF file"));
            continue;
        }
        let little = b[5] == 1;
        let u16_at = |o: usize| {
            if little {
                u16::from_le_bytes([b[o], b[o + 1]])
            } else {
                u16::from_be_bytes([b[o], b[o + 1]])
            }
        };
        let e_type = u16_at(16);
        let e_machine = u16_at(18);
        let entry: u64 = if b[4] == 2 && b.len() >= 32 {
            let mut x = [0u8; 8];
            x.copy_from_slice(&b[24..32]);
            if little {
                u64::from_le_bytes(x)
            } else {
                u64::from_be_bytes(x)
            }
        } else {
            let mut x = [0u8; 4];
            x.copy_from_slice(&b[24..28]);
            (if little {
                u32::from_le_bytes(x)
            } else {
                u32::from_be_bytes(x)
            }) as u64
        };
        evidence.push(format!(
            "{file}: class {} machine {} type {} entry {:#x}",
            b[4], e_machine, e_type, entry
        ));
        if b[4] != class {
            bad.push(format!(
                "{file} is {}-bit, and {} is {}-bit",
                if b[4] == 1 { 32 } else { 64 },
                a.arch.as_str(),
                if class == 1 { 32 } else { 64 }
            ));
        }
        if e_machine != machine {
            bad.push(format!(
                "{file} is for ELF machine {e_machine}, and {} is machine {machine}",
                a.arch.as_str()
            ));
        }
        if e_type != 2 {
            bad.push(format!("{file} is ELF type {e_type}, not an executable (2)"));
        }
        if entry == 0 {
            bad.push(format!("{file} has no entry point"));
        }
    }
    if bad.is_empty() {
        let archs: Vec<&str> = m.artifacts.iter().map(|a| a.arch.as_str()).collect();
        result(BINARIES, Outcome::Pass, archs.join(", "), evidence)
    } else {
        result(BINARIES, Outcome::Fail, bad.join("; "), evidence)
    }
}

fn schema(contract: &toml::Table, platform: &Platform) -> TestResult {
    let compiled = match jsonschema::JSONSchema::compile(&platform.schema) {
        Ok(s) => s,
        Err(e) => {
            return result(
                SCHEMA,
                Outcome::NotRun,
                format!("the platform file's schema does not compile: {e}"),
                Vec::new(),
            )
        }
    };
    let instance = match serde_json::to_value(contract) {
        Ok(v) => v,
        Err(e) => {
            return result(
                SCHEMA,
                Outcome::Fail,
                format!("the contract has a value JSON cannot hold: {e}"),
                Vec::new(),
            )
        }
    };
    let evidence = vec![format!("schema from the platform file of {}", platform.os)];
    let errors: Vec<String> = match compiled.validate(&instance) {
        Ok(()) => Vec::new(),
        Err(errs) => errs
            .map(|e| {
                let at = e.instance_path.to_string();
                format!("{}: {e}", if at.is_empty() { "the contract".to_string() } else { at })
            })
            .collect(),
    };
    if errors.is_empty() {
        result(
            SCHEMA,
            Outcome::Pass,
            format!("accepted by {}'s schema", platform.os),
            evidence,
        )
    } else {
        result(SCHEMA, Outcome::Fail, errors.join("; "), evidence)
    }
}

fn shown(caps: &toml::Table, key: &str) -> Option<String> {
    caps.get(key).map(|v| format!("{key} = {v}"))
}

fn device(caps: &toml::Table) -> TestResult {
    let evidence: Vec<String> = ["hw_pci_class", "hw_pci_bar", "hw_pci_irq", "hw_dma", "hw_usb"]
        .iter()
        .filter_map(|k| shown(caps, k))
        .collect();
    let pci = caps.contains_key("hw_pci_class");
    let usb = caps.contains_key("hw_usb");
    match (pci, usb) {
        (false, false) => result(
            DEVICE,
            Outcome::Fail,
            "the contract names no device: a driver names one, `hw_pci_class = \"00ff00\"` or `hw_usb = \"0403:6001\"`",
            evidence,
        ),
        (true, true) => result(
            DEVICE,
            Outcome::Fail,
            "the contract names a PCI device and a USB device: a driver drives one",
            evidence,
        ),
        (true, false) => {
            let class = caps.get("hw_pci_class").and_then(|v| v.as_str()).unwrap_or("?");
            result(DEVICE, Outcome::Pass, format!("PCI class {class}"), evidence)
        }
        (false, true) => {
            let host_owned: Vec<&str> = ["hw_pci_bar", "hw_pci_irq", "hw_dma"]
                .into_iter()
                .filter(|k| caps.contains_key(*k))
                .collect();
            if host_owned.is_empty() {
                let id = caps.get("hw_usb").and_then(|v| v.as_str()).unwrap_or("?");
                result(
                    DEVICE,
                    Outcome::Pass,
                    format!("USB device {id}, through its host"),
                    evidence,
                )
            } else {
                result(
                    DEVICE,
                    Outcome::Fail,
                    format!(
                        "a USB driver asks for {}: a USB device's registers, interrupt and DMA are its host's, and the \
                         driver reaches them through `gs::driver::bus`",
                        host_owned.join(", ")
                    ),
                    evidence,
                )
            }
        }
    }
}

/// A size as the contract writes one, `64KiB` or `1MiB`, in bytes.
pub fn size_of(s: &str) -> Option<u64> {
    let (n, unit) = match (s.strip_suffix("KiB"), s.strip_suffix("MiB")) {
        (Some(n), _) => (n, 1024u64),
        (_, Some(n)) => (n, 1024 * 1024),
        _ => return None,
    };
    if n.is_empty() || !n.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    n.parse::<u64>().ok()?.checked_mul(unit)
}

fn dma(caps: &toml::Table) -> TestResult {
    let Some(v) = caps.get("hw_dma") else {
        return result(DMA, Outcome::Pass, "asks for no DMA arena", Vec::new());
    };
    let evidence = vec![format!("hw_dma = {v}")];
    let Some(s) = v.as_str() else {
        return result(
            DMA,
            Outcome::Fail,
            "hw_dma is not a string: write a size, `\"64KiB\"`",
            evidence,
        );
    };
    match size_of(s) {
        None => result(
            DMA,
            Outcome::Fail,
            format!("`{s}` is not a size: write KiB or MiB, `\"64KiB\"`"),
            evidence,
        ),
        Some(b) if b > DMA_MAX => result(
            DMA,
            Outcome::Fail,
            format!(
                "asks for {} KiB, and an installed driver is given at most {} KiB",
                b / 1024,
                DMA_MAX / 1024
            ),
            evidence,
        ),
        Some(b) => result(
            DMA,
            Outcome::Pass,
            format!("{} KiB, approved by the operator at install", b / 1024),
            evidence,
        ),
    }
}

fn authority(caps: &toml::Table) -> TestResult {
    let mut asked = Vec::new();
    let mut evidence = Vec::new();
    for (k, v) in caps {
        if DRIVER_KEYS.contains(&k.as_str()) {
            continue;
        }
        // A boolean set to false asks for nothing.
        if v.as_bool() == Some(false) {
            evidence.push(format!("{k} = false (asks for nothing)"));
            continue;
        }
        evidence.push(format!("{k} = {v}"));
        let why = match k.as_str() {
            "handover" => "a driver cannot hand over: two drivers would program one device",
            "hw_mmio" | "hw_interrupt" | "hw_pio" => "a raw address or vector, which no spawn request can carry",
            "hw_device" => "a fixed device kind, which only in-tree drivers are given",
            "spawn" => "a dead key",
            _ => "a privilege, and an installed package is given none",
        };
        asked.push(format!("`{k}` ({why})"));
    }
    if asked.is_empty() {
        result(
            AUTHORITY,
            Outcome::Pass,
            "no privilege, no raw address, no vector",
            evidence,
        )
    } else {
        result(
            AUTHORITY,
            Outcome::Fail,
            format!("asks for {}", asked.join(", ")),
            evidence,
        )
    }
}

fn peers(caps: &toml::Table, owned: &OwnedManifest) -> TestResult {
    let m = owned.manifest();
    let list: Vec<String> = caps
        .get("ipc_send")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    let evidence: Vec<String> = list.iter().map(|p| format!("ipc_send: {p}")).collect();
    let mut bad = Vec::new();
    if list.len() > PEERS_MAX {
        bad.push(format!(
            "{} send peers, and an installed package may name at most {PEERS_MAX}",
            list.len()
        ));
    }
    if list.iter().any(|p| p == "supervisor") {
        bad.push("names `supervisor`, which no installed package is given".to_string());
    }
    for p in list.iter().filter(|p| p.contains('/')) {
        if !m.requires.iter().any(|r| r.name().to_string() == *p) {
            bad.push(format!(
                "sends to the package `{p}`, which the manifest does not require"
            ));
        }
    }
    if !bad.is_empty() {
        result(PEERS, Outcome::Fail, bad.join("; "), evidence)
    } else if list.is_empty() {
        result(PEERS, Outcome::Pass, "sends to no one", evidence)
    } else {
        result(
            PEERS,
            Outcome::Pass,
            format!("sends to {}; the operator approves each at install", list.join(", ")),
            evidence,
        )
    }
}
