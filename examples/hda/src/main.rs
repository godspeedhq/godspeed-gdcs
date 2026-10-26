// SPDX-License-Identifier: Apache-2.0
//! hda-gdcs: a GodspeedOS driver for an Intel High Definition Audio controller (QEMU's `intel-hda`).
//!
//! A GDCS reference example of a PCI device driven through DMA RINGS and an interrupt: the shape of most real
//! controllers (network cards, storage, USB hosts), where commands and their answers travel through memory the
//! device reads and writes by itself, and the device says "there is an answer" by interrupt.
//!
//! On startup it:
//!
//!   - resets the controller and its link, and finds the codec that answered (`STATESTS`) - registers;
//!   - lays the command rings in its DMA arena - the CORB (commands the controller fetches) and the RIRB (answers
//!     the controller writes back) - and starts both engines - the DMA arena;
//!   - enables the controller's interrupt and sends two codec verbs (GET_PARAMETER vendor ID and revision ID)
//!     through the CORB, taking each answer off the RIRB when the interrupt says it has arrived - the interrupt.
//!
//! It is a conformance subject, not an audio driver: it plays nothing. Its gdcs.toml declares exactly this.
//! Register offsets, bits and verbs are facts from the Intel HD Audio specification 1.0a, cross-checked against
//! GodspeedOS's in-tree `audio-driver`, whose code is not copied here (GDCS SPECIFICATION.md 10).

#![deny(unsafe_code)]
#![no_std]
#![no_main]

use godspeed::driver::irq::{Irq, Woke};
use godspeed::driver::wait::Budget;
use godspeed::driver::Registers;
use godspeed::{self as gs, ServiceContext};

// Controller registers (HDA 1.0a section 3.3), byte offsets into BAR 0.
const VMIN: usize = 0x02; // u8: specification minor version
const VMAJ: usize = 0x03; // u8: specification major version
const GCTL: usize = 0x08; // u32: bit 0 CRST - 0 holds the controller and link in reset
const STATESTS: usize = 0x0E; // u16: bit n set = a codec answered at address n after reset
const INTCTL: usize = 0x20; // u32: 31 GIE (global), 30 CIE (controller: RIRB) interrupt enables
const CORBLBASE: usize = 0x40;
const CORBUBASE: usize = 0x44;
const CORBWP: usize = 0x48; // u16: the last entry software wrote
const CORBRP: usize = 0x4A; // u16: bit 15 resets the read pointer
const CORBCTL: usize = 0x4C; // u8: bit 1 runs the CORB engine
const CORBSIZE: usize = 0x4E; // u8: bits 1:0 the size chosen
const RIRBLBASE: usize = 0x50;
const RIRBUBASE: usize = 0x54;
const RIRBWP: usize = 0x58; // u16: the last entry the controller wrote; bit 15 resets it
const RINTCNT: usize = 0x5A; // u16: raise the response status every N responses
const RIRBCTL: usize = 0x5C; // u8: bit 0 RINTCTL (status/interrupt on response), bit 1 runs the RIRB engine
const RIRBSTS: usize = 0x5D; // u8: write-1-to-clear: bit 0 a response arrived, bit 2 overrun
const RIRBSIZE: usize = 0x5E; // u8

const GCTL_CRST: u32 = 1 << 0;
const INTCTL_GIE: u32 = 1 << 31;
const INTCTL_CIE: u32 = 1 << 30;
const RING_RUN: u8 = 1 << 1;
const RIRB_RINTCTL: u8 = 1 << 0;
const CORBRP_RST: u16 = 1 << 15;
const RIRBWP_RST: u16 = 1 << 15;
const RING_SIZE_256: u8 = 0x02;
const RIRB_UNSOLICITED: u32 = 1 << 4; // in a response's extended word: no verb asked for this one

// The command rings in the arena: 256 CORB entries of 4 bytes, then 256 RIRB entries of 8. Both bases must be
// 128-byte aligned; the arena is page aligned.
const CORB_AT: usize = 0;
const RIRB_AT: usize = 1024;

// Codec verbs (HDA 1.0a section 7.3).
const GET_PARAMETER: u32 = 0xF00;
const PARAM_VENDOR_ID: u32 = 0x00;
const PARAM_REVISION_ID: u32 = 0x02;

const CRST_WAIT: Budget = Budget::ms(100);
const RESET_HOLD: Budget = Budget::ms(1); // the spec's minimum is 100 us; Linux holds up to 1 ms
const CODEC_ENUM_HOLD: Budget = Budget::ms(2); // codecs announce themselves within 521 us of leaving reset
const RING_WAIT: Budget = Budget::ms(100);
const ANSWER_WAIT: Budget = Budget::ms(1000);

/// The command rings, once they run: the arena and the two ring pointers this driver keeps.
struct Rings {
    arena: gs::driver::Arena,
    corb_wp: u16,
    rirb_rp: u16,
}

#[allow(unsafe_code)]
#[no_mangle]
pub extern "C" fn service_main(ctx: ServiceContext) -> ! {
    let Some(regs) = gs::driver::registers(&ctx) else {
        ctx.log("hda-gdcs: no register window was granted - nothing to drive");
        gs::ipc::park(&ctx);
    };
    let irq = Irq::granted(&ctx);
    if !irq.routed() {
        ctx.log("hda-gdcs: no interrupt was routed to this driver - its contract asks for one (`hw_pci_irq`)");
        gs::ipc::park(&ctx);
    }
    let Some(arena) = gs::driver::arena(&ctx) else {
        ctx.log("hda-gdcs: no DMA arena was granted - its contract asks for one (`hw_dma`)");
        gs::ipc::park(&ctx);
    };

    let Some(codecs) = reset(&ctx, &regs) else { gs::ipc::park(&ctx) };
    if codecs == 0 {
        ctx.log("hda-gdcs: the controller came out of reset, and no codec answered on its link");
        gs::ipc::park(&ctx);
    }
    let cad = codecs.trailing_zeros(); // the first codec that answered

    let Some(mut rings) = start_rings(&ctx, &regs, arena) else { gs::ipc::park(&ctx) };
    // Interrupt on each response: RIRBCTL.RINTCTL (set in start_rings) raises RIRBSTS, and CIE carries it to the CPU.
    regs.write32(INTCTL, INTCTL_GIE | INTCTL_CIE);

    let vendor = verb(&ctx, &regs, &irq, &mut rings, cad, 0, GET_PARAMETER, PARAM_VENDOR_ID);
    let revision = verb(&ctx, &regs, &irq, &mut rings, cad, 0, GET_PARAMETER, PARAM_REVISION_ID);
    match (vendor, revision) {
        (Some(v), Some(r)) => ctx.log_fmt(format_args!(
            "hda-gdcs: HD Audio {}.{}, codec {} is {:04x}:{:04x} revision {:#010x} - answered over the CORB/RIRB, by interrupt",
            regs.read8(VMAJ), regs.read8(VMIN), cad, v >> 16, v & 0xFFFF, r)),
        _ => ctx.log("hda-gdcs: the codec did not answer both verbs by interrupt (see the line above)"),
    }
    gs::ipc::park(&ctx)
}

/// Reset the controller and the link; the codecs that answered (STATESTS), or `None`, said.
fn reset(ctx: &ServiceContext, regs: &Registers) -> Option<u16> {
    regs.write16(STATESTS, 0x7FFF); // write-1-to-clear: forget any codec seen before this reset
    regs.write32(GCTL, regs.read32(GCTL) & !GCTL_CRST);
    if gs::driver::wait::until(ctx, CRST_WAIT, || regs.read32(GCTL) & GCTL_CRST == 0).is_err() {
        ctx.log("hda-gdcs: the controller never entered reset (GCTL.CRST stayed 1)");
        return None;
    }
    gs::driver::delay::hold(ctx, RESET_HOLD);
    regs.write32(GCTL, regs.read32(GCTL) | GCTL_CRST);
    if gs::driver::wait::until(ctx, CRST_WAIT, || regs.read32(GCTL) & GCTL_CRST != 0).is_err() {
        ctx.log("hda-gdcs: the controller never left reset (GCTL.CRST stayed 0)");
        return None;
    }
    gs::driver::delay::hold(ctx, CODEC_ENUM_HOLD);
    Some(regs.read16(STATESTS) & 0x7FFF)
}

/// Lay the CORB and RIRB in the arena and start both engines; `None`, said, if they would not run.
fn start_rings(ctx: &ServiceContext, regs: &Registers, arena: gs::driver::Arena) -> Option<Rings> {
    regs.write8(CORBCTL, 0);
    regs.write8(RIRBCTL, 0);
    if gs::driver::wait::until(ctx, RING_WAIT, || regs.read8(CORBCTL) & RING_RUN == 0 && regs.read8(RIRBCTL) & RING_RUN == 0).is_err() {
        ctx.log("hda-gdcs: the command rings would not stop");
        return None;
    }
    arena.zero();
    let corb = arena.phys_at(CORB_AT);
    let rirb = arena.phys_at(RIRB_AT);
    regs.write32(CORBLBASE, corb as u32);
    regs.write32(CORBUBASE, (corb >> 32) as u32);
    regs.write8(CORBSIZE, RING_SIZE_256);
    regs.write16(CORBWP, 0);
    // Reset the CORB read pointer: set bit 15, see it set, clear it, see it clear (HDA 1.0a 3.3.21).
    regs.write16(CORBRP, CORBRP_RST);
    let _ = gs::driver::wait::until(ctx, RING_WAIT, || regs.read16(CORBRP) & CORBRP_RST != 0);
    regs.write16(CORBRP, 0);
    if gs::driver::wait::until(ctx, RING_WAIT, || regs.read16(CORBRP) & CORBRP_RST == 0).is_err() {
        ctx.log("hda-gdcs: the CORB read pointer stayed in reset");
        return None;
    }
    regs.write32(RIRBLBASE, rirb as u32);
    regs.write32(RIRBUBASE, (rirb >> 32) as u32);
    regs.write8(RIRBSIZE, RING_SIZE_256);
    regs.write16(RIRBWP, RIRBWP_RST);
    regs.write16(RINTCNT, 1);
    regs.write8(RIRBSTS, 0x05);
    regs.write8(RIRBCTL, RING_RUN | RIRB_RINTCTL);
    regs.write8(CORBCTL, RING_RUN);
    if gs::driver::wait::until(ctx, RING_WAIT, || regs.read8(CORBCTL) & RING_RUN != 0 && regs.read8(RIRBCTL) & RING_RUN != 0).is_err() {
        ctx.log("hda-gdcs: the command rings would not start");
        return None;
    }
    Some(Rings { arena, corb_wp: 0, rirb_rp: 0 })
}

/// One codec verb: put it on the CORB, then wait for the interrupt that says the RIRB has an answer, and take it.
/// An unsolicited response (a codec reporting a jack) is passed over: it answers nobody's verb.
#[allow(clippy::too_many_arguments)]
fn verb(ctx: &ServiceContext, regs: &Registers, irq: &Irq, r: &mut Rings, cad: u32, nid: u32, cmd: u32, payload: u32) -> Option<u32> {
    let word = (cad & 0xF) << 28 | (nid & 0x7F) << 20 | (cmd & 0xFFF) << 8 | (payload & 0xFF);
    let wp = (r.corb_wp + 1) % 256;
    r.arena.write32(CORB_AT + wp as usize * 4, word);
    r.corb_wp = wp;
    regs.write16(CORBWP, wp);
    loop {
        match irq.wait(ctx, ANSWER_WAIT) {
            Woke::Interrupt => {
                regs.write8(RIRBSTS, 0x05); // acknowledge the response (and any overrun)
                irq.rearm(ctx);
                while regs.read16(RIRBWP) & 0xFF != r.rirb_rp {
                    r.rirb_rp = (r.rirb_rp + 1) % 256;
                    let at = RIRB_AT + r.rirb_rp as usize * 8;
                    let (resp, ext) = (r.arena.read32(at), r.arena.read32(at + 4));
                    if ext & RIRB_UNSOLICITED == 0 {
                        return Some(resp);
                    }
                }
            }
            // Nothing sends this driver requests; one that arrives is not what is being waited for.
            Woke::Request(_) => continue,
            Woke::Timeout => {
                ctx.log_fmt(format_args!(
                    "hda-gdcs: verb {:#010x} - no interrupt within {} ms (CORBRP={:#06x} RIRBWP={:#06x} RIRBSTS={:#04x})",
                    word, ANSWER_WAIT.as_us() / 1000, regs.read16(CORBRP), regs.read16(RIRBWP), regs.read8(RIRBSTS)));
                return None;
            }
        }
    }
}
