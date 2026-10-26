<!-- SPDX-License-Identifier: Apache-2.0 -->
# examples/usb-serial

A GodspeedOS driver package for an FTDI FT232 USB serial adapter (QEMU's `usb-serial`, `0403:6001`), kept as a
GDCS reference subject. It is the example of a **USB device driver**.

A USB device has no register window, interrupt or DMA arena of its own; its host controller owns all three. So
the driver is the host's client:

- the host (`xhci` here) binds the device;
- the supervisor starts this driver with that host as its peer;
- every transfer is a request to the host through `gs::driver::bus`.

The driver never touches hardware itself. Its contract asks for one thing: the device, by vendor and product
(`hw_usb = "0403:6001"`).

On startup it:

- asks the host who the bound device is and where its endpoints are;
- reads the device descriptor with a standard control transfer IN (`GET_DESCRIPTOR`);
- resets the adapter and sets 115200 baud, 8N1, with the FTDI's vendor control transfers OUT;
- sends one line on the bulk OUT endpoint, which QEMU writes to the serial port behind the adapter.

Then it logs one line and parks:

```
usb-serial-gdcs: FTDI 0403:6001 (descriptor 0403:6001, USB 2.00) through xhci - reset and 115200 8N1 set by
control transfer, and a line sent by bulk OUT
```

The line it sent reaches the serial port. GDCS's machine connects the adapter to a file, and the run keeps that
file beside the report as `gdcs-serial.out`, holding `hello from usb-serial-gdcs`.

It receives nothing: it is a conformance subject, not a serial driver. Bulk IN needs something on the far side
of the port to send, and a file sends nothing. On v0.31.0 it is CONFORMANT (simulated), **29 of 29**. `S2.dma`
passes here because a USB driver has no DMA arena, so its device has no DMA reach to confine.

## Build and run

It builds for all four architectures GodspeedOS supports (x86_64, aarch64, armv7, riscv64), and CI checks
every binary with `gdcs check` on each change. It runs, under GDCS, in QEMU on x86-64.

```
cd examples/usb-serial
godspeed check
godspeed package --local        # every architecture; GODSPEED_SIGNING_KEY=<a seed> to sign it
gdcs run usb-serial-gdcs-0.1.0.gsp --platform godspeed-platform.toml \
    --image godspeed-os-v0.31.0-x86_64.img --source . --out gdcs-run
```

The declaration gives QEMU a USB host and the adapter on it: `devices = ["qemu-xhci,id=xhci",
"usb-serial,bus=xhci.0,chardev=gdcs-serial"]`. `gdcs-serial` is the file chardev GDCS's machine always has.
Built packages (`*.gsp`) and `target/` are not committed.

The FTDI requests (reset, baud divisor, data format) are facts about the FT232, as Linux's `ftdi_sio` documents
them. None of its code is copied here (GDCS `SPECIFICATION.md` 10).
