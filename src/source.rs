// SPDX-License-Identifier: Apache-2.0
//! The two Stage 1 tests a binary cannot answer, asked of the driver's source.
//!
//! The source given is not proven to be the package's: nothing in a package binds it to its source. A report says
//! so on each of these results, and a reviewer who needs the binding builds the package from that source.
//!
//! The scan is lexical - comments and string literals removed, then words - which is enough for a crate that is
//! meant to hold no `unsafe` at all, and is stated as such rather than claimed to be a parser.

use std::path::{Path, PathBuf};

use crate::report::{Environment, Outcome, TestResult};
use crate::stage1::TestDef;

const CAVEAT: &str = "(from the source given, which nothing binds to the package)";

fn result(def: TestDef, outcome: Outcome, detail: String, evidence: Vec<String>) -> TestResult {
    TestResult {
        id: def.id.to_string(),
        version: def.version,
        title: def.title.to_string(),
        outcome,
        detail,
        evidence,
        environment: Environment::Offline,
    }
}

/// Every `.rs` file under `dir/src`, in a stable order.
fn rust_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.join("src")];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x == "rs") {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

/// `text` with its comments and string and character literals blanked to spaces, line breaks kept, so a word
/// found in what is left is code and its line number is the file's.
pub fn code_only(text: &str) -> String {
    let b = text.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    let blank = |out: &mut Vec<u8>, c: u8| out.push(if c == b'\n' { b'\n' } else { b' ' });
    while i < b.len() {
        let c = b[i];
        if c == b'/' && b.get(i + 1) == Some(&b'/') {
            while i < b.len() && b[i] != b'\n' {
                out.push(b' ');
                i += 1;
            }
        } else if c == b'/' && b.get(i + 1) == Some(&b'*') {
            let mut depth = 0;
            while i < b.len() {
                if b[i] == b'/' && b.get(i + 1) == Some(&b'*') {
                    depth += 1;
                    out.extend_from_slice(b"  ");
                    i += 2;
                } else if b[i] == b'*' && b.get(i + 1) == Some(&b'/') {
                    depth -= 1;
                    out.extend_from_slice(b"  ");
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    blank(&mut out, b[i]);
                    i += 1;
                }
            }
        } else if c == b'r' && (b.get(i + 1) == Some(&b'"') || b.get(i + 1) == Some(&b'#')) && !prev_is_ident(b, i) {
            // A raw string: r"...", r#"..."#.
            let mut j = i + 1;
            let mut hashes = 0;
            while b.get(j) == Some(&b'#') {
                hashes += 1;
                j += 1;
            }
            if b.get(j) != Some(&b'"') {
                out.push(c);
                i += 1;
                continue;
            }
            j += 1;
            loop {
                if j >= b.len() {
                    break;
                }
                if b[j] == b'"' && b[j + 1..].iter().take(hashes).filter(|&&h| h == b'#').count() == hashes {
                    j += 1 + hashes;
                    break;
                }
                j += 1;
            }
            for &x in &b[i..j.min(b.len())] {
                blank(&mut out, x);
            }
            i = j.min(b.len());
        } else if c == b'"' {
            out.push(b' ');
            i += 1;
            while i < b.len() && b[i] != b'"' {
                if b[i] == b'\\' && i + 1 < b.len() {
                    blank(&mut out, b[i]);
                    i += 1;
                }
                blank(&mut out, b[i]);
                i += 1;
            }
            if i < b.len() {
                out.push(b' ');
                i += 1;
            }
        } else if c == b'\'' {
            // A character literal ('a', '\n', '\''), or a lifetime ('a), which is left alone.
            let end = if b.get(i + 1) == Some(&b'\\') {
                b[i + 2..].iter().position(|&x| x == b'\'').map(|p| i + 2 + p)
            } else if b.get(i + 2) == Some(&b'\'') {
                Some(i + 2)
            } else {
                None
            };
            match end {
                Some(e) => {
                    for &x in &b[i..=e] {
                        blank(&mut out, x);
                    }
                    i = e + 1;
                }
                None => {
                    out.push(c);
                    i += 1;
                }
            }
        } else {
            out.push(c);
            i += 1;
        }
    }
    String::from_utf8(out).unwrap_or_default()
}

fn prev_is_ident(b: &[u8], i: usize) -> bool {
    i > 0 && (b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'_')
}

/// The words of `code`, each with its 1-based line number.
fn words(code: &str) -> Vec<(usize, &str)> {
    let mut out = Vec::new();
    for (n, line) in code.lines().enumerate() {
        for w in line.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')) {
            if !w.is_empty() {
                out.push((n + 1, w));
            }
        }
    }
    out
}

/// `S1.unsafe`.
pub fn unsafe_code(def: TestDef, dir: &Path) -> TestResult {
    let files = rust_files(dir);
    if files.is_empty() {
        return result(
            def,
            Outcome::NotRun,
            format!("no Rust source under {}", dir.join("src").display()),
            Vec::new(),
        );
    }
    let root = ["main.rs", "lib.rs"]
        .iter()
        .map(|f| dir.join("src").join(f))
        .find(|p| p.exists());
    let mut bad = Vec::new();
    let mut evidence = Vec::new();
    let mut entry_allowances = 0;
    match &root {
        None => bad.push("no src/main.rs or src/lib.rs, so no crate root to deny `unsafe_code`".to_string()),
        Some(r) => {
            let code = code_only(&std::fs::read_to_string(r).unwrap_or_default());
            let squashed: String = code.chars().filter(|c| !c.is_whitespace()).collect();
            if squashed.contains("#![deny(unsafe_code)]") || squashed.contains("#![forbid(unsafe_code)]") {
                evidence.push(format!("{}: denies unsafe_code", rel(dir, r)));
            } else {
                bad.push(format!("{} does not say `#![deny(unsafe_code)]`", rel(dir, r)));
            }
        }
    }
    for f in &files {
        let text = std::fs::read_to_string(f).unwrap_or_default();
        let code = code_only(&text);
        let w = words(&code);
        for (i, (line, word)) in w.iter().enumerate() {
            if *word == "unsafe" {
                let next = w.get(i + 1).map(|x| x.1).unwrap_or("");
                let rest = code.lines().nth(line - 1).unwrap_or("");
                let block = rest
                    .split("unsafe")
                    .nth(1)
                    .is_some_and(|after| after.trim_start().starts_with('{'));
                if block || matches!(next, "fn" | "impl" | "trait" | "extern") {
                    bad.push(format!("{}:{line}: `unsafe`", rel(dir, f)));
                }
            }
        }
        // One allowance is the house rule, not a hole (GodspeedOS CLAUDE.md 18.2): `#[no_mangle]` on the entry symbol
        // is itself covered by the lint, so the entry carries `#[allow(unsafe_code)]` directly above it. Any other
        // allowance - a second one, or one on anything but `#[no_mangle]` - is refused.
        let squashed: String = code.chars().filter(|c| !c.is_whitespace()).collect();
        let mut rest = squashed.as_str();
        while let Some(at) = rest.find("allow(unsafe_code)") {
            let after = &rest[at + "allow(unsafe_code)".len()..];
            let on_entry = rest[..at].ends_with("#[") && after.starts_with("]#[no_mangle]");
            if on_entry && entry_allowances == 0 {
                entry_allowances += 1;
                evidence.push(format!(
                    "{}: allows `unsafe_code` on its `#[no_mangle]` entry, the one exception",
                    rel(dir, f)
                ));
            } else if on_entry {
                bad.push(format!(
                    "{}: a second `#[no_mangle]` allowed `unsafe_code`; a crate has one entry",
                    rel(dir, f)
                ));
            } else {
                bad.push(format!(
                    "{}: allows `unsafe_code` on something other than its `#[no_mangle]` entry",
                    rel(dir, f)
                ));
            }
            rest = after;
        }
    }
    evidence.push(format!("{} source file(s) scanned", files.len()));
    if bad.is_empty() {
        result(def, Outcome::Pass, format!("no unsafe code {CAVEAT}"), evidence)
    } else {
        result(def, Outcome::Fail, format!("{} {CAVEAT}", bad.join("; ")), evidence)
    }
}

/// `S1.sdk`.
pub fn sdk_use(def: TestDef, dir: &Path) -> TestResult {
    let files = rust_files(dir);
    if files.is_empty() {
        return result(
            def,
            Outcome::NotRun,
            format!("no Rust source under {}", dir.join("src").display()),
            Vec::new(),
        );
    }
    let mut uses = Vec::new();
    if let Ok(cargo) = std::fs::read_to_string(dir.join("Cargo.toml")) {
        for (n, line) in cargo.lines().enumerate() {
            if line.trim_start().starts_with("godspeed-sdk") {
                uses.push(format!("Cargo.toml:{}: depends on godspeed-sdk", n + 1));
            }
        }
    }
    for f in &files {
        let code = code_only(&std::fs::read_to_string(f).unwrap_or_default());
        for (line, word) in words(&code) {
            if word == "godspeed_sdk" {
                uses.push(format!("{}:{line}: godspeed_sdk", rel(dir, f)));
            }
        }
    }
    if uses.is_empty() {
        result(
            def,
            Outcome::Pass,
            format!("no direct use of the SDK {CAVEAT}"),
            vec![format!("{} source file(s) scanned", files.len())],
        )
    } else {
        let n = uses.len();
        result(
            def,
            Outcome::Finding,
            format!(
                "{n} direct use(s) of `godspeed_sdk`, which may change between releases; `gs::driver` is what the \
                 compatibility promise covers {CAVEAT}"
            ),
            uses,
        )
    }
}

fn rel(dir: &Path, f: &Path) -> String {
    f.strip_prefix(dir).unwrap_or(f).to_string_lossy().replace('\\', "/")
}
