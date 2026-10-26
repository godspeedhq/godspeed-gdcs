// SPDX-License-Identifier: Apache-2.0
//! The platform file, `godspeed-platform.toml`: what one GodspeedOS release publishes for building against it.
//!
//! GDCS reads three things from it - the release, its commit and the contract schema it carries - and nothing
//! else. The file is the release's, so the schema a contract is checked against is exactly the one that release
//! validates with: GDCS keeps no copy of its own to drift.

/// A platform file, read.
#[derive(Clone, Debug)]
pub struct Platform {
    /// The release, `0.31.0`.
    pub os: String,
    /// The commit it was built from.
    pub commit: String,
    /// The service contract schema it carries (`[files]."service.schema.json"`).
    pub schema: serde_json::Value,
    /// The SHA-256 of the file, as a report names it.
    pub sha256: String,
}

/// Read a platform file. The error is one sentence saying what is missing.
pub fn read(bytes: &[u8]) -> Result<Platform, String> {
    let text = std::str::from_utf8(bytes).map_err(|_| "the platform file is not text".to_string())?;
    let v: toml::Value = text
        .parse()
        .map_err(|e| format!("the platform file is not TOML: {e}"))?;
    let field = |key: &str| -> Result<String, String> {
        v.get(key)
            .and_then(|x| x.as_str())
            .map(str::to_string)
            .ok_or_else(|| format!("the platform file has no `{key}`"))
    };
    let os = field("os")?;
    let commit = field("commit")?;
    let schema_text = v
        .get("files")
        .and_then(|f| f.get("service.schema.json"))
        .and_then(|s| s.as_str())
        .ok_or("the platform file carries no `[files].\"service.schema.json\"`: it is older than this suite reads")?;
    let schema = serde_json::from_str(schema_text)
        .map_err(|e| format!("the platform file's contract schema is not JSON: {e}"))?;
    let sha256 = crate::report::hex(&godspeed_package::Sha256Digest::of(bytes).0);
    Ok(Platform {
        os,
        commit,
        schema,
        sha256,
    })
}
