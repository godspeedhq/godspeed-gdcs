// SPDX-License-Identifier: Apache-2.0
//! The driver's declaration, `gdcs.toml`: what it claims to do, and what it needs the machine to have.
//!
//! A functional pass covers only the behaviour a driver declares (SPECIFICATION.md 4, Stage 3), so the declaration
//! is part of what is tested, and a report shows it. It is the driver author's, kept beside the driver's source:
//!
//! ```toml
//! format = 1
//!
//! [target]
//! os = "0.31.0"                     # the GodspeedOS release this driver is built and tested against
//!
//! [qemu]
//! devices = ["edu"]                 # what `-device` QEMU is given, beyond the disks and the network card
//!
//! [[behaviour]]
//! id = "identity-factorial-dma"     # lowercase, digits and hyphens
//! expect = "edu-driver: device id 0x010000ed, alive true, 5! = 120 by interrupt, 64 bytes through DMA and back intact"
//! within-secs = 30                  # after the install; 30 when left out
//! ```
//!
//! A driver that declares no behaviour is a probe, and its functional result says so rather than passing.
//!
//! `[target].os` is the release the driver is for. It is OPTIONAL and declarative: GDCS checks it against the
//! platform file and image it was given (and the bundled probe) and refuses to start on a mismatch, so a run
//! against the wrong release is caught before anything boots, not misread as a driver fault. It does not fetch
//! anything - the operator still supplies the platform file and image.

use serde::Deserialize;

/// The format this suite reads.
pub const FORMAT: u32 = 1;
/// How long a behaviour is waited for when its declaration does not say.
pub const DEFAULT_WITHIN_SECS: u64 = 30;
/// The longest a behaviour may ask to be waited for: a card is minutes, not hours.
pub const MAX_WITHIN_SECS: u64 = 600;

/// A declaration, read.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Declaration {
    /// [`FORMAT`].
    pub format: u32,
    /// The release this driver is built and tested against. Optional.
    #[serde(default)]
    pub target: Target,
    /// What QEMU is given.
    #[serde(default)]
    pub qemu: Qemu,
    /// What the driver claims to do.
    #[serde(default)]
    pub behaviour: Vec<Behaviour>,
}

/// The release the driver targets (`[target]`).
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Target {
    /// The GodspeedOS release, e.g. `0.31.0`. `None` when the declaration does not name one.
    #[serde(default)]
    pub os: Option<String>,
}

/// The emulated machine's devices.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Qemu {
    /// `-device` values, each one QEMU device with its options (`edu`, `e1000,netdev=n1`).
    #[serde(default)]
    pub devices: Vec<String>,
}

/// One claimed behaviour: a line the driver logs when it has done the thing.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Behaviour {
    /// Its name in the report, `S3.behaviour.<id>`.
    pub id: String,
    /// The text that shows it, found anywhere in the serial log after the install.
    pub expect: String,
    /// Seconds to wait for it after the install.
    #[serde(rename = "within-secs", default)]
    pub within_secs: Option<u64>,
}

impl Behaviour {
    /// How long to wait for it.
    pub fn within(&self) -> u64 {
        self.within_secs.unwrap_or(DEFAULT_WITHIN_SECS)
    }
}

/// Read a declaration. The error is one sentence saying what is wrong.
pub fn read(text: &str) -> Result<Declaration, String> {
    let d: Declaration = toml::from_str(text).map_err(|e| format!("gdcs.toml: {}", e.message()))?;
    if d.format != FORMAT {
        return Err(format!(
            "gdcs.toml: format {} - this suite reads format {FORMAT}",
            d.format
        ));
    }
    if let Some(os) = &d.target.os {
        // A release is digits and dots (`0.31.0`), no `v` prefix: the same spelling the platform file uses.
        let ok = !os.is_empty()
            && os.len() <= 32
            && os.bytes().all(|c| c.is_ascii_digit() || c == b'.')
            && os.bytes().any(|c| c.is_ascii_digit());
        if !ok {
            return Err(format!(
                "gdcs.toml: target.os `{os}` - a release is digits and dots, e.g. \"0.31.0\" (no `v`)"
            ));
        }
    }
    for (i, b) in d.behaviour.iter().enumerate() {
        let id_ok = !b.id.is_empty()
            && b.id.len() <= 40
            && b.id
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-');
        if !id_ok {
            return Err(format!(
                "gdcs.toml: behaviour id `{}` - use lowercase letters, digits and hyphens",
                b.id
            ));
        }
        if d.behaviour[..i].iter().any(|x| x.id == b.id) {
            return Err(format!("gdcs.toml: behaviour `{}` appears twice", b.id));
        }
        if b.expect.trim().is_empty() {
            return Err(format!("gdcs.toml: behaviour `{}` expects nothing", b.id));
        }
        if b.within() == 0 || b.within() > MAX_WITHIN_SECS {
            return Err(format!(
                "gdcs.toml: behaviour `{}`: within-secs is 1 to {MAX_WITHIN_SECS}",
                b.id
            ));
        }
    }
    for dev in &d.qemu.devices {
        if dev.is_empty() || dev.starts_with('-') || dev.chars().any(|c| c.is_whitespace()) {
            return Err(format!("gdcs.toml: `{dev}` is not one QEMU device"));
        }
    }
    Ok(d)
}

impl Declaration {
    /// If the declaration names a target release and it differs from the `platform_os` of the platform file
    /// GDCS was given, the one-sentence reason to refuse the run. `None` when they agree, or when the
    /// declaration names no target (then the platform file alone decides). A leading `v` on either side is
    /// ignored, so `v0.31.0` and `0.31.0` agree.
    pub fn target_mismatch(&self, platform_os: &str) -> Option<String> {
        let want = self.target.os.as_deref()?;
        let norm = |s: &str| s.trim_start_matches('v').to_string();
        if norm(want) == norm(platform_os) {
            None
        } else {
            Some(format!(
                "gdcs.toml says this driver targets GodspeedOS {want}, but the platform file is for {platform_os}: \
                 pass {want}'s godspeed-platform.toml and image, or change target.os"
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_is_optional_and_parses() {
        let none = read("format = 1\n").unwrap();
        assert_eq!(none.target.os, None);
        let some = read("format = 1\n\n[target]\nos = \"0.31.0\"\n").unwrap();
        assert_eq!(some.target.os.as_deref(), Some("0.31.0"));
    }

    #[test]
    fn a_target_version_with_a_v_or_letters_is_refused() {
        assert!(read("format = 1\n\n[target]\nos = \"v0.31.0\"\n").is_err());
        assert!(read("format = 1\n\n[target]\nos = \"latest\"\n").is_err());
        assert!(read("format = 1\n\n[target]\nos = \"\"\n").is_err());
    }

    #[test]
    fn mismatch_is_reported_only_when_they_differ() {
        let d = read("format = 1\n\n[target]\nos = \"0.31.0\"\n").unwrap();
        assert!(d.target_mismatch("0.31.0").is_none());
        // A `v` on the platform side is ignored, so a tag-shaped os still agrees.
        assert!(d.target_mismatch("v0.31.0").is_none());
        assert!(d.target_mismatch("0.30.0").is_some());
        // No target named: the platform file alone decides, so never a mismatch.
        let bare = read("format = 1\n").unwrap();
        assert!(bare.target_mismatch("0.30.0").is_none());
    }
}
