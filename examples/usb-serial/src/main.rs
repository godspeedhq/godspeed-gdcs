// SPDX-License-Identifier: Apache-2.0
//! usb-serial-gdcs: a GodspeedOS driver for an FTDI FT232 USB serial adapter (QEMU's `usb-serial`, 0403:6001).
//!
//! A GDCS reference example of a USB DEVICE driver. A USB device has no register window, interrupt or DMA arena a
//! driver could be granted - its host controller owns all of them - so the driver is the host's client: the host
//! (`xhci` here) binds the device, the supervisor starts this driver with the host as its peer, and every transfer
//! is a request to the host through `gs::driver::bus`. The driver never touches hardware itself.
//!
//! On startup it:
//!
//!   - asks the host who the bound device is (vendor, product) and where its endpoints are;
//!   - reads the device descriptor with a standard control transfer IN (GET_DESCRIPTOR);
//!   - resets the adapter and sets 115200 baud, 8N1, with the FTDI's vendor control transfers OUT;
//!   - sends one line on the adapter's bulk OUT endpoint, which QEMU writes to the serial port behind it.
//!
//! It receives nothing: it is a conformance subject, not a serial driver. Its gdcs.toml declares exactly this.
//! The FTDI requests (reset, baud divisor, data format) are facts about the FT232, as Linux's `ftdi_sio` documents
//! them; no code is copied (GDCS SPECIFICATION.md 10).

#![deny(unsafe_code)]
#![no_std]
#![no_main]

use godspeed::driver::bus::UsbDevice;
use godspeed::{self as gs, ServiceContext};

/// A standard GET_DESCRIPTOR (device) request: device-to-host, standard, device; descriptor type 1, index 0, 18 bytes.
const GET_DEVICE_DESCRIPTOR: [u8; 8] = [0x80, 0x06, 0x00, 0x01, 0x00, 0x00, 18, 0];
/// FTDI vendor requests: host-to-device, vendor, device (bmRequestType 0x40).
const FTDI_OUT: u8 = 0x40;
const FTDI_RESET: u8 = 0x00; // wValue 0: reset the adapter
const FTDI_SET_BAUD: u8 = 0x03; // wValue: the baud divisor; 0x001A is 115200 on the FT232's 3 MHz clock
const FTDI_SET_DATA: u8 = 0x04; // wValue: data bits (8), parity none, one stop bit
const BAUD_115200: u16 = 0x001A;
const DATA_8N1: u16 = 0x0008;

/// The line sent on the bulk OUT endpoint.
const LINE: &[u8] = b"hello from usb-serial-gdcs\r\n";

/// A vendor control request OUT with no data stage.
fn ftdi(request: u8, value: u16) -> [u8; 8] {
    [FTDI_OUT, request, value as u8, (value >> 8) as u8, 0, 0, 0, 0]
}

#[allow(unsafe_code)]
#[no_mangle]
pub extern "C" fn service_main(ctx: ServiceContext) -> ! {
    let Some(dev) = UsbDevice::granted(&ctx) else {
        ctx.log("usb-serial-gdcs: no USB host was given to this driver - it was not started for a USB device");
        gs::ipc::park(&ctx);
    };
    let bound = match dev.info() {
        Ok(b) => b,
        Err(e) => {
            ctx.log_fmt(format_args!("usb-serial-gdcs: {} did not say who the device is: {}", dev.host(), e.as_str()));
            gs::ipc::park(&ctx);
        }
    };

    // The device descriptor, by a standard control transfer IN: its USB version and its own vendor and product.
    let mut d = [0u8; 18];
    let read = dev.control_in(GET_DEVICE_DESCRIPTOR, &mut d);
    let (usb_major, usb_minor, desc_vid, desc_pid) = match read {
        Ok(n) if n >= 12 => (d[3], d[2], u16::from_le_bytes([d[8], d[9]]), u16::from_le_bytes([d[10], d[11]])),
        Ok(n) => {
            ctx.log_fmt(format_args!("usb-serial-gdcs: the device descriptor was {n} bytes, short of 12"));
            gs::ipc::park(&ctx);
        }
        Err(e) => {
            ctx.log_fmt(format_args!("usb-serial-gdcs: GET_DESCRIPTOR failed: {}", e.as_str()));
            gs::ipc::park(&ctx);
        }
    };

    // Reset the adapter, then set 115200 8N1, by the FTDI's vendor control transfers OUT.
    for (what, setup) in [
        ("reset", ftdi(FTDI_RESET, 0)),
        ("set the baud rate", ftdi(FTDI_SET_BAUD, BAUD_115200)),
        ("set 8N1", ftdi(FTDI_SET_DATA, DATA_8N1)),
    ] {
        if let Err(e) = dev.control_out(setup, &[]) {
            ctx.log_fmt(format_args!("usb-serial-gdcs: the vendor request to {what} failed: {}", e.as_str()));
            gs::ipc::park(&ctx);
        }
    }

    // One line on the first bulk OUT endpoint.
    if let Err(e) = dev.bulk_out(0, LINE) {
        ctx.log_fmt(format_args!("usb-serial-gdcs: bulk OUT failed: {}", e.as_str()));
        gs::ipc::park(&ctx);
    }

    ctx.log_fmt(format_args!(
        "usb-serial-gdcs: FTDI {:04x}:{:04x} (descriptor {:04x}:{:04x}, USB {:x}.{:02x}) through {} - reset and 115200 8N1 \
         set by control transfer, and a line sent by bulk OUT",
        bound.vid, bound.pid, desc_vid, desc_pid, usb_major, usb_minor, dev.host()
    ));
    gs::ipc::park(&ctx)
}
