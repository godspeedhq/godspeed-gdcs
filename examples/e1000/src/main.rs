// SPDX-License-Identifier: Apache-2.0
//! e1000-gdcs: a standalone GodspeedOS driver for Intel's 82540EM ("e1000", QEMU's dev NIC, 8086:100e).
//!
//! Written to dogfood the Godspeed Driver Conformance Suite against a real device driver, not the teaching
//! `edu`: it claims the ethernet controller by PCI class, is granted its register window, its interrupt and a
//! DMA arena, and on startup brings the controller up and transmits one frame by DMA, proving all three work:
//!
//!   - resets the controller and waits for the reset to self-clear (registers);
//!   - sets the link up and reads the MAC from the receive-address registers (registers);
//!   - lays a TX descriptor ring and one broadcast frame in the arena, hands the ring to the NIC, and waits
//!     for the descriptor-done bit (the DMA arena, and the device reading it).
//!
//! The register offsets, bits and values are facts about the Intel 82540EM (its datasheet), cross-checked
//! against GodspeedOS's in-tree `nic-driver`, whose code is not copied here (GDCS SPECIFICATION.md 10). This driver does only bring-up and one transmit -
//! it is a conformance subject, not a full NIC - so its declared behaviour (gdcs.toml) is exactly that, and
//! GDCS reports a functional pass for that and no more. Everything goes through `gs::driver`; no `unsafe`.

#![deny(unsafe_code)]
#![no_std]
#![no_main]

use godspeed::driver::wait::Budget;
use godspeed::{self as gs, ServiceContext};

// Intel 82540EM register offsets (bytes into BAR 0).
const REG_CTRL: usize = 0x0000; // Device Control
const REG_STATUS: usize = 0x0008; // Device Status; bit 1 (LU) = link up
const REG_TCTL: usize = 0x0400; // Transmit Control
const REG_TIPG: usize = 0x0410; // Transmit Inter-Packet Gap
const REG_TDBAL: usize = 0x3800; // TX descriptor base, low
const REG_TDBAH: usize = 0x3804; // TX descriptor base, high
const REG_TDLEN: usize = 0x3808; // TX descriptor ring length (bytes)
const REG_TDH: usize = 0x3810; // TX descriptor head
const REG_TDT: usize = 0x3818; // TX descriptor tail
const REG_RAL0: usize = 0x5400; // Receive Address Low 0 (MAC bytes 0..4, EEPROM-loaded)
const REG_RAH0: usize = 0x5404; // Receive Address High 0 (MAC bytes 4..6)

const CTRL_RST: u32 = 1 << 26; // global reset; the NIC self-clears it when done
const CTRL_SLU: u32 = 1 << 6; // set link up
const CTRL_ASDE: u32 = 1 << 5; // auto-speed detection enable
const TCTL_VALUE: u32 = (1 << 1) | (1 << 3) | (0x0F << 4) | (0x40 << 12); // EN | PSP | CT=15 | COLD=0x40
const TIPG_VALUE: u32 = 10 | (8 << 10) | (6 << 20); // 82540EM copper

const TXD_CMD_EOP: u8 = 1 << 0; // end of packet
const TXD_CMD_IFCS: u8 = 1 << 1; // insert FCS (the NIC appends the CRC)
const TXD_CMD_RS: u8 = 1 << 3; // report status: the NIC sets DD when the frame is sent
const TXD_STA_DD: u8 = 1 << 0; // descriptor done

/// The arena holds a small TX ring and one frame buffer. A TX descriptor is 16 bytes; the ring length in
/// bytes must be a multiple of 128, so a short ring is given 128.
const TX_RING_BYTES: u32 = 128;
const TX_BUF: usize = 128; // the frame buffer starts after the ring
const FRAME_LEN: usize = 60; // a minimum-length ethernet frame (the NIC pads and appends the CRC)

const RESET_WAIT: Budget = Budget::ms(100);
const TX_WAIT: Budget = Budget::ms(500);

#[allow(unsafe_code)]
#[no_mangle]
pub extern "C" fn service_main(ctx: ServiceContext) -> ! {
    let Some(regs) = gs::driver::registers(&ctx) else {
        ctx.log("e1000-gdcs: no register window was granted - nothing to drive");
        gs::ipc::park(&ctx);
    };
    let Some(arena) = gs::driver::arena(&ctx) else {
        ctx.log("e1000-gdcs: no DMA arena was granted - its contract asks for one (`hw_dma`)");
        gs::ipc::park(&ctx);
    };

    // Reset to a known state, and wait for the reset bit to self-clear.
    regs.write32(REG_CTRL, regs.read32(REG_CTRL) | CTRL_RST);
    let reset_ok = gs::driver::wait::until(&ctx, RESET_WAIT, || regs.read32(REG_CTRL) & CTRL_RST == 0).is_ok();

    // Bring the link up, then read link state and the MAC.
    regs.write32(REG_CTRL, regs.read32(REG_CTRL) | CTRL_SLU | CTRL_ASDE);
    let link_up = (regs.read32(REG_STATUS) >> 1) & 1 == 1;
    let ral = regs.read32(REG_RAL0);
    let rah = regs.read32(REG_RAH0);
    let mac = [
        (ral & 0xff) as u8,
        ((ral >> 8) & 0xff) as u8,
        ((ral >> 16) & 0xff) as u8,
        ((ral >> 24) & 0xff) as u8,
        (rah & 0xff) as u8,
        ((rah >> 8) & 0xff) as u8,
    ];

    // Lay a one-frame broadcast in the arena: destination ff:ff:ff:ff:ff:ff, our MAC as source, a harmless
    // ethertype, the rest zero. The NIC appends the CRC (IFCS).
    arena.zero();
    for i in 0..6 {
        arena.write8(TX_BUF + i, 0xff);
        arena.write8(TX_BUF + 6 + i, mac[i]);
    }
    arena.write8(TX_BUF + 12, 0x88); // ethertype 0x88b5 (IEEE experimental), nothing routable
    arena.write8(TX_BUF + 13, 0xb5);

    // One TX descriptor (16 bytes) at ring offset 0: buffer address, length, then the command byte.
    arena.write64(0, arena.phys_at(TX_BUF));
    arena.write8(8, FRAME_LEN as u8); // length, low byte
    arena.write8(9, (FRAME_LEN >> 8) as u8); // length, high byte
    arena.write8(11, TXD_CMD_EOP | TXD_CMD_IFCS | TXD_CMD_RS); // command byte
    arena.write8(12, 0); // status: clear DD before handing it over

    // Point the NIC at the ring and set transmit going. The NIC transmits [head, tail).
    let ring = arena.phys_at(0);
    regs.write32(REG_TDBAL, (ring & 0xffff_ffff) as u32);
    regs.write32(REG_TDBAH, (ring >> 32) as u32);
    regs.write32(REG_TDLEN, TX_RING_BYTES);
    regs.write32(REG_TDH, 0);
    regs.write32(REG_TIPG, TIPG_VALUE);
    regs.write32(REG_TCTL, TCTL_VALUE);
    regs.write32(REG_TDT, 1);

    let tx_done = gs::driver::wait::until(&ctx, TX_WAIT, || arena.read8(12) & TXD_STA_DD != 0).is_ok();

    ctx.log_fmt(format_args!(
        "e1000-gdcs: reset {}  link {}  MAC {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}  {}",
        if reset_ok { "OK" } else { "TIMEOUT" },
        if link_up { "UP" } else { "down" },
        mac[0],
        mac[1],
        mac[2],
        mac[3],
        mac[4],
        mac[5],
        if tx_done { "sent a broadcast frame (TX done)" } else { "TX did not complete" },
    ));

    gs::ipc::park(&ctx)
}
