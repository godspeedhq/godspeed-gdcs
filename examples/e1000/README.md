<!-- SPDX-License-Identifier: Apache-2.0 -->
# examples/e1000

A real GodspeedOS driver package for Intel's 82540EM ("e1000", QEMU's default NIC, `8086:100e`), kept here as
a reference subject for GDCS: the first driver run through it that drives an actual network controller rather
than QEMU's teaching device `edu`.

On startup it claims the ethernet controller by PCI class (`hw_pci_class = "020000"`), and is granted its
registers, its interrupt and a 16 KiB DMA arena. It resets the controller, brings the link up, reads the MAC,
and transmits one broadcast frame by DMA, waiting for the descriptor-done bit. Then it logs one line and
parks:

```
e1000-gdcs: reset OK  link UP  MAC 52:54:00:12:34:56  sent a broadcast frame (TX done)
```

It is bring-up and one transmit, not a full NIC driver: no receive path and nothing `net-stack` can use. Its
`gdcs.toml` declares exactly that behaviour, so a functional pass covers that and no more.

## What it found

The first GDCS run of this driver turned up a gap in GDCS itself. Installing a driver that claims the network
device stops GodspeedOS's own `nic-driver`, so `net-stack` loses its link, and GDCS could no longer download
its Stage 2 probes. GDCS now fetches the probes before it installs the driver under test (PR #7). With that
fix this driver is CONFORMANT (simulated) on v0.31.0, 28 of 29 tests passing. The 29th, `S2.dma`, reads NOT
ENFORCEABLE because QEMU's `pc` board has no IOMMU.

## Build and run

It builds for all four architectures GodspeedOS supports (x86_64, aarch64, armv7, riscv64), and CI checks
every binary with `gdcs check` on each change. It runs, under GDCS, in QEMU on x86-64.

This needs the developer CLI (`godspeed`) and the release's platform file and image (see the top-level
README):

```
cd examples/e1000
godspeed check
godspeed package --local        # every architecture; GODSPEED_SIGNING_KEY=<a seed> to sign it
gdcs run e1000-gdcs-0.1.0.gsp --platform godspeed-platform.toml \
    --image godspeed-os-v0.31.0-x86_64.img --source . --out gdcs-run
```

QEMU needs no extra device: GDCS's machine always has an e1000, which this driver takes over from
`nic-driver` for as long as it is installed. Built packages (`*.gsp`) and `target/` are not committed.

The register offsets, bits and values come from the 82540EM's datasheet and were checked against
GodspeedOS's in-tree `nic-driver`. None of its code is copied here (GDCS `SPECIFICATION.md` 10).
