<!-- SPDX-License-Identifier: Apache-2.0 -->
# backlog/

Open items for GDCS that are **recorded rather than closed**, one file each, in the shape GodspeedOS's own
[`backlog/`](https://github.com/godspeedhq/godspeed-os/tree/main/backlog) uses. An item earns a file when it is
real and not being done right now, and every entry carries:

- a **status line** at the top, `**Status:` OPEN or CLOSED, with the date;
- **why it is open** - the evidence, or the thing it is waiting for;
- **what is ruled out**, so the next attempt does not re-derive it;
- **the next concrete step**, and what it costs.

An item is closed in place: its status line says CLOSED and the date, and what was learned goes at the end.

| # | Item | Status |
|---|---|---|
| [1](1-hardware-mode-waits-for-a-usb-serial-adapter.md) | Hardware mode: `gdcs run` on a real board | OPEN, waiting for a USB serial adapter |
