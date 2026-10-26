<!-- SPDX-License-Identifier: Apache-2.0 -->
# isolation-probe

GDCS's Stage 2 fixture: an ordinary GodspeedOS service that, on startup, tries the things an isolated service
must not be able to do and logs whether each was refused. It is not a model driver and is not meant to be
installed by hand; GDCS installs it during `gdcs run` and reads its lines. See [`../README.md`](../README.md)
for what it is, how it was built and signed, and how to rebuild it for a new release.

The source was scaffolded with `godspeed new service isolation-probe` against GodspeedOS v0.31.0, then given its
attempts in `src/main.rs` and a dependency on `godspeed-sdk` (for `adversarial`, the raw-syscall and fault
primitives a probe needs and a model driver never would).
