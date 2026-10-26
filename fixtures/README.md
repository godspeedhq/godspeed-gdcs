<!-- SPDX-License-Identifier: Apache-2.0 -->
# fixtures/

Packages GDCS ships to run against a release. Each says how it was made, so it can be made again.

## `isolation-probe/` and `isolation-probe-<release>.gsp`

GDCS's Stage 2 fixture (SPECIFICATION.md 4, Stage 2). `isolation-probe/` is its source - an ordinary service
that holds nothing but `log_write` and, on startup, tries the things an isolated service must not be able to do
(use a fabricated capability, hand the kernel a kernel-space pointer, make a gated query, reach the machine's
power, mint a send capability to a non-peer, guess past the syscall surface), logging whether each was refused.
`gdcs run` installs it like any package and reads those lines. The `.gsp` is the built, signed package, bundled
into the `gdcs` binary (`include_bytes!`) so a run needs no build step; one per release it is built for.

## `mmu-fault-probe/` and `mmu-fault-probe-<release>.gsp`

GDCS's other Stage 2 fixture: the attempt that KILLS the caller. `mmu-fault-probe/` is a service that, on
startup, announces and then reads address `0x0` - an address it was never granted, like any unmapped MMIO or
cross-service address - which takes a page fault. A service cannot log that it was refused after it is dead, so
the DEATH is the evidence: the kernel kills the faulting task and keeps running, the supervisor reports `died -
memory fault` and restarts it up to the installed-package bound (`RESTARTS_MAX`, 5) before leaving it down.
`gdcs run` reads that from the serial log (`S2.unmapped-read`). Built and signed the same way, same key.

**The signing key is a THROWAWAY, in plain sight on purpose.** It signs both probes. It is a fixture: a machine trusts it only if its
operator runs `godspeed trust add gdcs <public key>`, which GDCS's run does and then leaves (it is a disposable
QEMU machine). It vouches for nothing.

```
seed (secret):  1111111111111111111111111111111111111111111111111111111111111111
public key:     ed25519:d04ab232742bb4ab3a1368bd4615e4e6d0224ab71a016baf8520a332c9778737
```

Built and signed with the developer CLI, for every architecture, against the release named in the file. Leave
`GODSPEED_OS` unset: a probe depends on the SDK by the release's tag as well as on `godspeed`, and a local OS checkout
replaces only the second, which links two SDKs.

```
cd isolation-probe
GODSPEED_SIGNING_KEY=<the seed> godspeed package --local
# then rename isolation-probe-0.1.0.gsp to ../isolation-probe-<release>.gsp
```

**Rebuild it for a new release.** The bundled package is built for one GodspeedOS release (its name is in the file
name and in its manifest's `[os] release`). Against a different release, `gdcs run` reports the Stage 2 attempts
NOT RUN and says to rebuild this against that release. Create the project fresh with the CLI's current `service`
template for that release (`godspeed new service isolation-probe --os-release <release> --publisher gdcs`), copy
`src/main.rs` and the `godspeed-sdk` dependency line onto it, build, sign and commit the new `.gsp`.
