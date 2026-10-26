<!-- SPDX-License-Identifier: Apache-2.0 -->
# 1. Hardware mode: `gdcs run` on a real board

**Status: OPEN, 2026-10-26 - deferred by the operator until a USB serial adapter is on hand ("Put this in the
backlog till I get that usb to serial thing").**

## What it is

`gdcs run` today makes its machine in QEMU (x86-64 `pc`, riscv64 `virt`). Hardware mode would run the same
profiles against a real board (SPECIFICATION.md 7.3):
- **the board:** booted from a release image by the operator;
- **its shell:** driven over the board's serial line;
- **the package:** served from the host over the LAN;
- **the report:** naming the board, keeping its serial log, and reporting the results as physical, never
  merged with simulated ones (7.2).

## Why it waits

There is no reference driver a board in this project can exercise without one. The four examples were chosen
for devices QEMU emulates:

| Example | On a board |
|---|---|
| `e1000` | none of the boards has an Intel 82540EM |
| `hda` | the T630 and Wyse have HD Audio, but `audio-driver` owns it, and a takeover there tests the takeover more than the driver |
| `xhci` | would take over the USB host the VisionFive's and Pi 4's disk is on |
| `usb-serial` | **runs on any board with a free USB port, given an FTDI FT232-family adapter (0403:6001)** - the one the operator does not have yet |

So the adapter is what makes hardware mode testable with a known-good subject, rather than with a new driver
and a new harness at once.

## What is ruled out

- **QEMU's Pi machines instead.** `raspi2b` has no PCI and no network `dwc2` drives; `raspi4b` emulates neither
  the Pi 4's PCIe nor its GENET. Neither can fetch a package (checked 2026-10-26).
- **Formatting the board's disk as QEMU's run does.** `drives flash 0 data` on a board is the operator's real
  disk. The operator has not chosen yet among: use the disk as it is and format only with an explicit flag, a
  dedicated test stick, or never format. Decide this before building.

## The next concrete step

1. **The operator decides the disk question** above.
2. **The serial transport.** The QEMU machine talks to a TCP socket. A board's serial port is a COM port or a
   `/dev/tty*` at 115200 8N1, so it needs a serial transport behind `machine::Machine` beside the socket.
3. **The network.** The package server listens on the host's LAN address instead of QEMU's 10.0.2.2, and the
   board needs a link: cable or WiFi, each board's own.
4. **First run:** `usb-serial` on the VisionFive 2 Lite with the adapter plugged in. That run would also be
   the hardware card v0.32.0 still owes on that board (its PLIC set up at boot).
