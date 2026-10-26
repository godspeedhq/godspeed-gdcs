<!-- SPDX-License-Identifier: Apache-2.0 -->
# examples/hda

A GodspeedOS driver package for an Intel High Definition Audio controller (QEMU's `intel-hda`, with an
`hda-output` codec on it), kept as a GDCS reference subject. It is the example of a PCI device driven through
**DMA rings and an interrupt**, which is the shape of most real controllers. Commands and their answers travel
through memory the device reads and writes by itself, and the device raises an interrupt to say an answer has
arrived.

On startup it claims the controller by PCI class (`hw_pci_class = "040300"`), taking it over from GodspeedOS's
`audio-driver` while it is installed. It is granted the controller's registers, its interrupt and an 8 KiB DMA
arena. It then:

- resets the controller and its link, and finds the codec that answered (`STATESTS`);
- lays the two command rings in the arena: the CORB, which holds commands the controller fetches, and the RIRB,
  which holds answers the controller writes back. Then it starts both engines;
- enables the controller interrupt and sends two codec verbs through the CORB, asking for the vendor ID and the
  revision ID. It takes each answer off the RIRB when the interrupt says it is there.

When it has both answers it logs one line and parks:

```
hda-gdcs: HD Audio 1.0, codec 0 is 1af4:0012 revision 0x00100101 - answered over the CORB/RIRB, by interrupt
```

It plays nothing: it is a conformance subject, not an audio driver. Its `gdcs.toml` declares exactly that
behaviour. On v0.32.0 it is CONFORMANT (simulated) on x86-64 and on riscv64, 28 of 29 tests passing. The 29th, `S2.dma`, reads NOT
ENFORCEABLE because QEMU's `pc` board has no IOMMU.

## Build and run

It builds for all four architectures GodspeedOS supports (x86_64, aarch64, armv7, riscv64), and CI checks
every binary with `gdcs check` on each change. It runs, under GDCS, in QEMU on x86-64.

```
cd examples/hda
godspeed check
godspeed package --local        # every architecture; GODSPEED_SIGNING_KEY=<a seed> to sign it
gdcs run hda-gdcs-0.1.0.gsp --platform godspeed-platform.toml \
    --image godspeed-os-v0.32.0-x86_64.img --source . --out gdcs-run
```

The declaration gives QEMU the controller and a codec: `devices = ["intel-hda",
"hda-output,audiodev=gdcs-audio"]`. `gdcs-audio` is the silent audio backend GDCS's machine always has, because
QEMU's codecs need one. Built packages (`*.gsp`) and `target/` are not committed.

The register offsets, bits and verbs come from the Intel HD Audio specification 1.0a, and were checked against
GodspeedOS's in-tree `audio-driver`. None of its code is copied here (GDCS `SPECIFICATION.md` 10).
