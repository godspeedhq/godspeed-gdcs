// SPDX-License-Identifier: Apache-2.0
//! xhci-gdcs: a GodspeedOS driver that brings up an xHCI USB host controller (QEMU's `qemu-xhci`) by its registers.
//!
//! A GDCS reference example of a HOST CONTROLLER, and of a REGISTERS-ONLY driver: it asks for no interrupt and no
//! DMA arena, so everything it learns it learns by reading and writing the controller's registers. A full host
//! driver goes on to build command and event rings in DMA memory and enumerate devices (GodspeedOS's own `xhci`
//! does); this one stops where registers stop, which is where every host driver starts.
//!
//! On startup it:
//!
//!   - reads the capability registers: the interface version, the operational registers' offset, slots and ports;
//!   - stops the controller (USBCMD.RS = 0, waiting for USBSTS.HCH) and resets it (USBCMD.HCRST), waiting for the
//!     reset to self-clear and for the controller to be ready (USBSTS.CNR = 0);
//!   - reads every root port's status register (PORTSC) and reports which have a device connected, and at what speed.
//!
//! It drives nothing on the ports: it is a conformance subject, not a USB stack. Its gdcs.toml declares exactly this.
//! Register offsets and bits are facts from the xHCI specification 1.2, checked against GodspeedOS's in-tree `xhci`,
//! whose code is not copied here (GDCS SPECIFICATION.md 10).

#![deny(unsafe_code)]
#![no_std]
#![no_main]

use godspeed::driver::wait::Budget;
use godspeed::driver::Registers;
use godspeed::{self as gs, ServiceContext};

// Capability registers (xHCI 1.2 section 5.3), from BAR 0.
const CAP_CAPLENGTH_HCIVERSION: usize = 0x00; // u32: bits 7:0 CAPLENGTH, bits 31:16 HCIVERSION
const CAP_HCSPARAMS1: usize = 0x04; // u32: 7:0 MaxSlots, 18:8 MaxIntrs, 31:24 MaxPorts

// Operational registers (section 5.4), from BAR 0 + CAPLENGTH.
const OP_USBCMD: usize = 0x00;
const OP_USBSTS: usize = 0x04;
const OP_PORTSC: usize = 0x400; // PORTSC for port n (from 1) at 0x400 + 0x10 * (n - 1)
const PORT_STRIDE: usize = 0x10;

const CMD_RS: u32 = 1 << 0; // Run/Stop
const CMD_HCRST: u32 = 1 << 1; // Host Controller Reset; the controller clears it when done
const STS_HCH: u32 = 1 << 0; // Host Controller Halted
const STS_CNR: u32 = 1 << 11; // Controller Not Ready

const PORT_CCS: u32 = 1 << 0; // Current Connect Status: a device is attached
const PORT_SPEED_SHIFT: u32 = 10; // bits 13:10, the default Protocol Speed IDs (section 7.2.2.1.1)

const HALT_WAIT: Budget = Budget::ms(100); // the spec gives the controller 16 ms to halt
const RESET_WAIT: Budget = Budget::ms(1000);

/// The most root ports this driver reports on one line (xHCI allows 255; QEMU's has 8).
const PORTS_SHOWN: usize = 16;

#[allow(unsafe_code)]
#[no_mangle]
pub extern "C" fn service_main(ctx: ServiceContext) -> ! {
    let Some(regs) = gs::driver::registers(&ctx) else {
        ctx.log("xhci-gdcs: no register window was granted - nothing to drive");
        gs::ipc::park(&ctx);
    };

    let cap = regs.read32(CAP_CAPLENGTH_HCIVERSION);
    let op = (cap & 0xFF) as usize;
    let version = (cap >> 16) as u16;
    let hcs1 = regs.read32(CAP_HCSPARAMS1);
    let slots = hcs1 & 0xFF;
    let ports = ((hcs1 >> 24) & 0xFF) as usize;

    if !reset(&ctx, &regs, op) {
        gs::ipc::park(&ctx);
    }

    // Every root port: is a device attached, and at what speed.
    let mut found = 0usize;
    let mut list = Line::new();
    for p in 1..=ports.min(PORTS_SHOWN) {
        let sc = regs.read32(op + OP_PORTSC + PORT_STRIDE * (p - 1));
        if sc & PORT_CCS != 0 {
            found += 1;
            let speed = match (sc >> PORT_SPEED_SHIFT) & 0xF {
                1 => "full speed",
                2 => "low speed",
                3 => "high speed",
                4 => "SuperSpeed",
                5 => "SuperSpeed+",
                _ => "an unknown speed",
            };
            let _ = core::fmt::Write::write_fmt(&mut list, format_args!("{}port {} {}", if found > 1 { ", " } else { "" }, p, speed));
        }
    }

    ctx.log_fmt(format_args!(
        "xhci-gdcs: xHCI {}.{:02x}, {} ports, {} slots - reset OK, {} port(s) with a device: {} - read by registers only",
        version >> 8,
        version & 0xFF,
        ports,
        slots,
        found,
        if found == 0 { "none" } else { list.as_str() }
    ));
    gs::ipc::park(&ctx)
}

/// Stop the controller, then reset it; false, said, if it would not.
fn reset(ctx: &ServiceContext, regs: &Registers, op: usize) -> bool {
    regs.write32(op + OP_USBCMD, regs.read32(op + OP_USBCMD) & !CMD_RS);
    if gs::driver::wait::until(ctx, HALT_WAIT, || regs.read32(op + OP_USBSTS) & STS_HCH != 0).is_err() {
        ctx.log("xhci-gdcs: the controller did not halt (USBSTS.HCH stayed 0) - not resetting a running controller");
        return false;
    }
    regs.write32(op + OP_USBCMD, CMD_HCRST);
    let done = gs::driver::wait::until(ctx, RESET_WAIT, || {
        regs.read32(op + OP_USBCMD) & CMD_HCRST == 0 && regs.read32(op + OP_USBSTS) & STS_CNR == 0
    });
    if done.is_err() {
        ctx.log("xhci-gdcs: the controller did not finish its reset (USBCMD.HCRST or USBSTS.CNR stayed set)");
        return false;
    }
    true
}

/// A short fixed buffer for the port list: no heap, and a list that would not fit is cut, not overrun.
struct Line {
    b: [u8; 192],
    n: usize,
}

impl Line {
    fn new() -> Self {
        Line { b: [0; 192], n: 0 }
    }
    fn as_str(&self) -> &str {
        core::str::from_utf8(&self.b[..self.n]).unwrap_or("")
    }
}

impl core::fmt::Write for Line {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let take = s.len().min(self.b.len() - self.n);
        self.b[self.n..self.n + take].copy_from_slice(&s.as_bytes()[..take]);
        self.n += take;
        Ok(())
    }
}
