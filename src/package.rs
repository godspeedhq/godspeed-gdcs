// SPDX-License-Identifier: Apache-2.0
//! A `.gsp` package file, read whole: its entries by registry path, and where the one package in it lives.
//!
//! The framing rules are the package crate's (`godspeed_package::gsp`), so a file this accepts is one the CLI
//! and the OS accept. This adds only what a conformance run needs: every entry kept, in order, and the package's
//! own folder found.

use godspeed_package::gsp::{self, Line};

/// One document in the file.
#[derive(Clone, Debug)]
pub struct Entry {
    /// The registry path it is served at.
    pub path: String,
    /// Its bytes.
    pub bytes: Vec<u8>,
}

/// A package file, read.
#[derive(Clone, Debug)]
pub struct PackageFile {
    /// Every entry, in file order.
    pub entries: Vec<Entry>,
    /// The package's folder, `packages/<publisher>/<name>/<version>/`, with its trailing slash.
    pub dir: String,
}

impl PackageFile {
    /// The entry at `path`, if the file holds one.
    pub fn get(&self, path: &str) -> Option<&[u8]> {
        self.entries.iter().find(|e| e.path == path).map(|e| e.bytes.as_slice())
    }

    /// The entry `file` in the package's own folder.
    pub fn own(&self, file: &str) -> Option<&[u8]> {
        self.get(&format!("{}{}", self.dir, file))
    }
}

/// Read a package file. The error is one sentence saying what is wrong and where.
pub fn read(bytes: &[u8]) -> Result<PackageFile, String> {
    let mut at = 0usize;
    let next_line = |at: &mut usize| -> Result<String, String> {
        let rest = &bytes[*at..];
        let n = rest
            .iter()
            .take(gsp::LINE_MAX + 1)
            .position(|&b| b == b'\n')
            .ok_or_else(|| format!("at byte {}: a line with no end within {} bytes", *at, gsp::LINE_MAX))?;
        let line = std::str::from_utf8(&rest[..n]).map_err(|_| format!("at byte {}: a line that is not text", *at))?;
        *at += n + 1;
        Ok(line.to_string())
    };
    let header = next_line(&mut at).map_err(|_| "not a package file: no first line".to_string())?;
    gsp::header(&header).map_err(|_| {
        format!(
            "not a package file: the first line is `{header}`, not `{}`",
            gsp::HEADER
        )
    })?;
    let mut entries: Vec<Entry> = Vec::new();
    loop {
        let start = at;
        let line = next_line(&mut at)?;
        match gsp::line(&line).map_err(|e| format!("at byte {start}: `{line}`: {e}"))? {
            Line::End => break,
            Line::Entry { size, path } => {
                if entries.len() == gsp::ENTRIES_MAX {
                    return Err(format!("more than {} entries", gsp::ENTRIES_MAX));
                }
                if entries.iter().any(|e| e.path == path) {
                    return Err(format!("`{path}` appears twice"));
                }
                let size = usize::try_from(size).map_err(|_| format!("`{path}`: a size this machine cannot hold"))?;
                let end = at.checked_add(size).filter(|&e| e <= bytes.len()).ok_or_else(|| {
                    format!(
                        "`{path}` says {size} bytes, and the file ends {} bytes after its line",
                        bytes.len() - at
                    )
                })?;
                // Each entry's bytes are followed by a line break, so the next line starts on a line of its own.
                if bytes.get(end) != Some(&b'\n') {
                    return Err(format!("`{path}`: no line break after its {size} bytes"));
                }
                entries.push(Entry {
                    path: path.to_string(),
                    bytes: bytes[at..end].to_vec(),
                });
                at = end + 1;
            }
        }
    }
    if at != bytes.len() {
        return Err(format!("{} bytes after `end`", bytes.len() - at));
    }
    let manifests: Vec<&Entry> = entries
        .iter()
        .filter(|e| e.path.starts_with("packages/") && e.path.ends_with("/manifest.toml"))
        .collect();
    let dir = match manifests.as_slice() {
        [one] => one.path.trim_end_matches("manifest.toml").to_string(),
        [] => return Err("no `packages/<publisher>/<name>/<version>/manifest.toml`: it holds no package".to_string()),
        many => return Err(format!("{} manifests: a package file holds one package", many.len())),
    };
    if dir.split('/').filter(|s| !s.is_empty()).count() != 4 {
        return Err(format!(
            "the manifest is at `{dir}manifest.toml`, not `packages/<publisher>/<name>/<version>/`"
        ));
    }
    Ok(PackageFile { entries, dir })
}
