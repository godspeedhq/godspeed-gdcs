<!-- SPDX-License-Identifier: Apache-2.0 -->
# examples/xhci

A GodspeedOS driver package that brings up an xHCI USB host controller (QEMU's `qemu-xhci`), kept as a GDCS
reference subject. It is the example of a **host controller**, and of a **registers-only** driver: it asks for
no interrupt and no DMA arena, so everything it learns comes from reading and writing the controller's
registers.

A full host driver goes on to build command and event rings in DMA memory and enumerate devices, as
GodspeedOS's own `xhci` does. This one stops where registers stop, which is where every host driver starts.

On startup it claims the controller by PCI class (`hw_pci_class = "0c0330"`), taking it over from GodspeedOS's
`xhci` while it is installed, and:

- reads the capability registers: the interface version, the offset of the operational registers, and the
  number of slots and ports;
- stops the controller (`USBCMD.RS` cleared, waiting for `USBSTS.HCH`), then resets it (`USBCMD.HCRST`),
  waiting for the reset to clear and the controller to be ready (`USBSTS.CNR`);
- reads every root port's status register (`PORTSC`) and reports which ports have a device connected, and at
  what speed.

Then it logs one line and parks:

```
xhci-gdcs: xHCI 1.00, 8 ports, 64 slots - reset OK, 2 port(s) with a device: port 5 high speed, port 6 high
speed - read by registers only
```

It drives nothing on its ports: it is a conformance subject, not a USB stack. On v0.32.0 it is CONFORMANT
(simulated), **29 of 29**. `S2.dma` passes because a registers-only driver has no DMA arena.

## Build and run

It builds for all four architectures GodspeedOS supports (x86_64, aarch64, armv7, riscv64), and CI checks
every binary with `gdcs check` on each change. It runs, under GDCS, in QEMU on x86-64.

```
cd examples/xhci
godspeed check
godspeed package --local        # every architecture; GODSPEED_SIGNING_KEY=<a seed> to sign it
gdcs run xhci-gdcs-0.1.0.gsp --platform godspeed-platform.toml \
    --image godspeed-os-v0.32.0-x86_64.img --source . --out gdcs-run
```

The declaration gives QEMU the controller and two devices for it to find: `devices = ["qemu-xhci,id=xhci",
"usb-kbd,bus=xhci.0", "usb-mouse,bus=xhci.0"]`. QEMU's xHCI puts its USB 2 ports after its USB 3 ones, so the
two devices appear on ports 5 and 6. Built packages (`*.gsp`) and `target/` are not committed.

The register offsets and bits come from the xHCI specification 1.2, and were checked against GodspeedOS's
in-tree `xhci`. None of its code is copied here (GDCS `SPECIFICATION.md` 10).
