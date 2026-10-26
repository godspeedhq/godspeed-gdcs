// SPDX-License-Identifier: Apache-2.0
//! The Godspeed Driver Conformance Suite (SPECIFICATION.md).
//!
//! GDCS tests a GodspeedOS driver package against what GodspeedOS publishes - the platform file, the contract
//! schema, the package format, the install grant - and reports what it showed, where, and what it did not.

#![forbid(unsafe_code)]

pub mod basic;
pub mod declare;
pub mod machine;
pub mod package;
pub mod platform;
pub mod report;
pub mod source;
pub mod stage1;
