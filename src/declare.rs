// SPDX-License-Identifier: Apache-2.0
//! The driver's declaration, `gdcs.toml`: what it claims to do, and what it needs the machine to have.
//!
//! A functional pass covers only the behaviour a driver declares (SPECIFICATION.md 4, Stage 3), so the declaration
//! is part of what is tested, and a report shows it. It is the driver author's, kept beside the driver's source:
//!
//! ```toml
//! format = 1
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
    /// What QEMU is given.
    #[serde(default)]
    pub qemu: Qemu,
    /// What the driver claims to do.
    #[serde(default)]
    pub behaviour: Vec<Behaviour>,
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
