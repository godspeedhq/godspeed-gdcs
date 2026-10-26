// SPDX-License-Identifier: Apache-2.0
//! Every Stage 1 test, seen to fail (SPECIFICATION.md 4: a check that has never fired is not evidence).
//!
//! A good package is built here from parts - a contract, a minimal ELF, a manifest naming both, signed by a
//! throwaway key - and passes every test. Then, for each test in `stage1::TESTS`, one part is broken in the one way
//! that test exists to catch, and the test is asserted to FAIL (or, for `S1.sdk`, to report its finding). The
//! table is checked against `TESTS`, so a test added without a failing package fails this file.

use std::path::{Path, PathBuf};

use gdcs::platform::{self, Platform};
use gdcs::report::Outcome;
use gdcs::stage1::{self, TESTS};
use godspeed_package::{gsp, ManifestSignature, Sha256Digest};

/// A THROWAWAY key for these tests, in plain sight on purpose. Nothing trusts it.
const SEED: [u8; 32] = [0x9d; 32];
const RELEASE: &str = "0.31.0";

/// A contract schema written for these tests: enough of the release's shape that `S1.schema` can be seen to fail.
/// A real run reads the release's own schema from its platform file.
const SCHEMA: &str = r#"{
  "type": "object",
  "required": ["name", "version", "resources", "capabilities"],
  "additionalProperties": false,
  "properties": {
    "name": { "type": "string" },
    "version": { "type": "string" },
    "resources": { "type": "object" },
    "capabilities": { "type": "object" },
    "placement": { "type": "object" }
  }
}"#;

fn test_platform() -> Platform {
    let toml = format!(
        "format = 1\nos = \"{RELEASE}\"\ncommit = \"0000000000000000000000000000000000000000\"\n\n[files]\n\"service.schema.json\" = {}\n",
        toml::Value::String(SCHEMA.to_string())
    );
    platform::read(toml.as_bytes()).expect("the test platform file reads")
}

/// The parts of a package, each changeable before it is put together.
#[derive(Clone)]
struct Parts {
    contract: String,
    elf: Vec<u8>,
    kind: &'static str,
    release: &'static str,
    /// The contract's version, when it should differ from the manifest's.
    contract_version: &'static str,
    /// Applied to the manifest's text AFTER it is signed.
    after_signing: Option<fn(&mut String)>,
    /// Applied to the binary AFTER the manifest names its digest.
    after_digest: Option<fn(&mut Vec<u8>)>,
}

fn good_contract(version: &str, extra: &str) -> String {
    format!(
        "name = \"gdcs-fixture\"\nversion = \"{version}\"\n\n[resources.memory]\nrequest = \"16MiB\"\nlimit = \"16MiB\"\n\n\
         [capabilities]\nlog_write = true\nhw_pci_class = \"00ff00\"\nhw_pci_bar = 0\nhw_pci_irq = true\nhw_dma = \"8KiB\"\n{extra}"
    )
}

/// A 64-byte ELF64 header: an x86-64 executable with an entry point. Enough for `S1.binaries`; nothing runs it.
fn elf(machine: u16) -> Vec<u8> {
    let mut b = vec![0u8; 64];
    b[..4].copy_from_slice(b"\x7fELF");
    b[4] = 2; // 64-bit
    b[5] = 1; // little-endian
    b[6] = 1; // ELF version
    b[16..18].copy_from_slice(&2u16.to_le_bytes()); // executable
    b[18..20].copy_from_slice(&machine.to_le_bytes());
    b[20..24].copy_from_slice(&1u32.to_le_bytes());
    b[24..32].copy_from_slice(&0x40_0000u64.to_le_bytes());
    b
}

fn good() -> Parts {
    Parts {
        contract: good_contract("0.1.0", ""),
        elf: elf(62),
        kind: "driver",
        release: RELEASE,
        contract_version: "0.1.0",
        after_signing: None,
        after_digest: None,
    }
}

fn hexs(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn build(p: &Parts) -> Vec<u8> {
    let contract = if p.contract_version == "0.1.0" {
        p.contract.clone()
    } else {
        p.contract.replacen("0.1.0", p.contract_version, 1)
    };
    let manifest = format!(
        "format = 1\n\n[package]\nname = \"gdcs/gdcs-fixture\"\nversion = \"0.1.0\"\nkind = \"{}\"\ndescription = \"A GDCS fixture\"\n\n\
         [os]\nrelease = \"{}\"\n\n[contract]\ndigest = \"sha256:{}\"\n\n[[artifact]]\narch = \"x86_64\"\nsize = {}\ndigest = \"sha256:{}\"\n",
        p.kind,
        p.release,
        hexs(&Sha256Digest::of(contract.as_bytes()).0),
        p.elf.len(),
        hexs(&Sha256Digest::of(&p.elf).0)
    );
    let sig = ManifestSignature::sign(&SEED, manifest.as_bytes());
    let mut sig_text = String::new();
    sig.write(&mut sig_text).unwrap();
    let mut manifest = manifest;
    if let Some(f) = p.after_signing {
        f(&mut manifest);
    }
    let mut elf = p.elf.clone();
    if let Some(f) = p.after_digest {
        f(&mut elf);
    }
    let dir = "packages/gdcs/gdcs-fixture/0.1.0/";
    let entries: Vec<(String, Vec<u8>)> = vec![
        (format!("{dir}manifest.toml"), manifest.into_bytes()),
        (format!("{dir}manifest.sig"), sig_text.into_bytes()),
        (format!("{dir}contract.toml"), contract.into_bytes()),
        (format!("{dir}x86_64.elf"), elf),
    ];
    let refs: Vec<(&str, &[u8])> = entries.iter().map(|(p, b)| (p.as_str(), b.as_slice())).collect();
    let mut out = Vec::new();
    gsp::write(&mut out, &refs).unwrap();
    out
}

const GOOD_MAIN: &str = "#![deny(unsafe_code)]\n#![no_std]\n#![no_main]\n\nuse godspeed::ServiceContext;\n\n\
                         // `unsafe` in a comment is not code.\n#[allow(unsafe_code)]\n#[no_mangle]\n\
                         pub extern \"C\" fn service_main(ctx: ServiceContext) -> ! {\n    let _ = \"unsafe { in a string }\";\n    loop {}\n}\n";

/// A project folder holding `main` as its `src/main.rs`.
fn source(tag: &str, main: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("gdcs-test-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("src/main.rs"), main).unwrap();
    std::fs::write(
        dir.join("Cargo.toml"),
        "[package]\nname = \"gdcs-fixture\"\n\n[dependencies]\ngodspeed = \"0.31\"\n",
    )
    .unwrap();
    dir
}

fn run(file: &[u8], src: &Path) -> Vec<gdcs::report::TestResult> {
    stage1::check("fixture.gsp", file, &test_platform(), Some(src)).1
}

fn outcome(results: &[gdcs::report::TestResult], id: &str) -> Outcome {
    results
        .iter()
        .find(|r| r.id == id)
        .unwrap_or_else(|| panic!("no result for {id}"))
        .outcome
}

#[test]
fn a_good_package_passes_every_test() {
    let src = source("good", GOOD_MAIN);
    let results = run(&build(&good()), &src);
    for r in &results {
        assert_eq!(
            r.outcome,
            Outcome::Pass,
            "{}: {} - {}",
            r.id,
            r.outcome.word(),
            r.detail
        );
    }
    assert_eq!(results.len(), TESTS.len());
}

/// Each Stage 1 test and the package (and source) built to fail it.
fn failing() -> Vec<(&'static str, Vec<u8>, String, Outcome)> {
    let main = GOOD_MAIN.to_string();
    let mut v: Vec<(&'static str, Vec<u8>, String, Outcome)> = Vec::new();
    let mut add = |id: &'static str, parts: Parts, main: &str, expect: Outcome| {
        v.push((id, build(&parts), main.to_string(), expect))
    };

    // S1.container is broken on the bytes, below.
    add(
        stage1::SIGNATURE.id,
        Parts {
            after_signing: Some(|m| *m = m.replace("A GDCS fixture", "A GDCS fixturf")),
            ..good()
        },
        &main,
        Outcome::Fail,
    );
    add(
        stage1::MANIFEST.id,
        Parts {
            contract_version: "0.2.0",
            ..good()
        },
        &main,
        Outcome::Fail,
    );
    add(
        stage1::DIGESTS.id,
        Parts {
            after_digest: Some(|b| *b.last_mut().unwrap() ^= 1),
            ..good()
        },
        &main,
        Outcome::Fail,
    );
    add(
        stage1::KIND.id,
        Parts {
            kind: "service",
            ..good()
        },
        &main,
        Outcome::Fail,
    );
    add(
        stage1::TARGET.id,
        Parts {
            release: "0.30.0",
            ..good()
        },
        &main,
        Outcome::Fail,
    );
    add(
        stage1::BINARIES.id,
        Parts {
            elf: elf(183),
            ..good()
        },
        &main,
        Outcome::Fail,
    );
    add(
        stage1::SCHEMA.id,
        Parts {
            contract: good_contract("0.1.0", "").replacen("version", "colour = \"red\"\nversion", 1),
            ..good()
        },
        &main,
        Outcome::Fail,
    );
    add(
        stage1::DEVICE.id,
        Parts {
            contract: good_contract("0.1.0", "").replace("hw_pci_class = \"00ff00\"\n", ""),
            ..good()
        },
        &main,
        Outcome::Fail,
    );
    add(
        stage1::DMA.id,
        Parts {
            contract: good_contract("0.1.0", "").replace("8KiB", "4MiB"),
            ..good()
        },
        &main,
        Outcome::Fail,
    );
    add(
        stage1::AUTHORITY.id,
        Parts {
            contract: good_contract("0.1.0", "service_control = true\n"),
            ..good()
        },
        &main,
        Outcome::Fail,
    );
    add(
        stage1::PEERS.id,
        Parts {
            contract: good_contract("0.1.0", "ipc_send = [\"supervisor\"]\n"),
            ..good()
        },
        &main,
        Outcome::Fail,
    );
    add(
        stage1::UNSAFE.id,
        good(),
        &main.replace("    loop {}", "    unsafe { core::hint::unreachable_unchecked() }"),
        Outcome::Fail,
    );
    add(
        stage1::SDK.id,
        good(),
        &main.replace(
            "use godspeed::ServiceContext;",
            "use godspeed_sdk::service_context::ServiceContext;",
        ),
        Outcome::Finding,
    );

    let mut truncated = build(&good());
    truncated.truncate(truncated.len() - 10);
    v.push((stage1::CONTAINER.id, truncated, main, Outcome::Fail));
    v
}

#[test]
fn every_test_is_seen_to_fail() {
    let cases = failing();
    for def in TESTS {
        assert!(
            cases.iter().any(|c| c.0 == def.id),
            "{} has no package built to fail it",
            def.id
        );
    }
    for (i, (id, file, main, expect)) in cases.iter().enumerate() {
        let src = source(&format!("fail{i}"), main);
        let results = run(file, &src);
        let got = outcome(&results, id);
        let detail = &results.iter().find(|r| r.id == *id).unwrap().detail;
        assert_eq!(
            got,
            *expect,
            "{id}: expected {}, got {} - {detail}",
            expect.word(),
            got.word()
        );
        // And it is THAT test that catches it: a package broken one way fails one test, so a failure names its cause.
        // The broken container is the exception, since nothing after it can run.
        if *id != stage1::CONTAINER.id {
            let others: Vec<String> = results
                .iter()
                .filter(|r| r.id != *id && r.outcome != Outcome::Pass)
                .map(|r| format!("{} {} - {}", r.id, r.outcome.word(), r.detail))
                .collect();
            assert!(others.is_empty(), "{id}'s package also tripped: {others:?}");
        }
    }
}

/// The one-allowance rule, each way it can be broken.
#[test]
fn unsafe_code_allowance_is_the_entry_only() {
    for (tag, main) in [
        ("no-deny", GOOD_MAIN.replace("#![deny(unsafe_code)]\n", "")),
        (
            "allow-elsewhere",
            GOOD_MAIN.replace("#[allow(unsafe_code)]\n#[no_mangle]\n", "#[no_mangle]\n")
                + "#[allow(unsafe_code)]\nfn x() {}\n",
        ),
        ("unsafe-fn", GOOD_MAIN.to_string() + "unsafe fn y() {}\n"),
        (
            "unsafe-impl",
            GOOD_MAIN.to_string() + "struct S;\nunsafe impl Send for S {}\n",
        ),
    ] {
        let src = source(tag, &main);
        let results = run(&build(&good()), &src);
        assert_eq!(outcome(&results, stage1::UNSAFE.id), Outcome::Fail, "{tag}");
    }
}

#[test]
fn without_source_the_source_tests_do_not_run_and_the_run_is_incomplete() {
    let results = stage1::check("fixture.gsp", &build(&good()), &test_platform(), None).1;
    assert_eq!(outcome(&results, stage1::UNSAFE.id), Outcome::NotRun);
    assert_eq!(outcome(&results, stage1::SDK.id), Outcome::NotRun);
}

#[test]
fn a_package_that_does_not_read_runs_nothing_else() {
    let results = stage1::check("x.gsp", b"not a package\n", &test_platform(), None).1;
    assert_eq!(outcome(&results, stage1::CONTAINER.id), Outcome::Fail);
    assert!(results.iter().skip(1).all(|r| r.outcome == Outcome::NotRun));
    assert_eq!(results.len(), TESTS.len());
}

#[test]
fn the_json_report_keeps_its_published_schema() {
    let schema: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string("schema/report.schema.json").unwrap()).unwrap();
    let compiled = jsonschema::JSONSchema::compile(&schema).unwrap();
    for file in [build(&good()), b"nonsense".to_vec()] {
        let (subject, results) = stage1::check("fixture.gsp", &file, &test_platform(), None);
        let p = test_platform();
        let report = gdcs::report::Report {
            format: gdcs::report::REPORT_FORMAT,
            gdcs: gdcs::report::GDCS_VERSION,
            profile: "Stage 1".into(),
            subject,
            target: gdcs::report::Target {
                os: p.os,
                commit: p.commit,
                platform_sha256: p.sha256,
            },
            results,
        };
        let v: serde_json::Value = serde_json::from_str(&report.json()).unwrap();
        if let Err(errs) = compiled.validate(&v) {
            let all: Vec<String> = errs.map(|e| format!("{}: {e}", e.instance_path)).collect();
            panic!("the report breaks its schema: {all:?}");
        };
    }
}
