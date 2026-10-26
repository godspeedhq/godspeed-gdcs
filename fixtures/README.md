<!-- SPDX-License-Identifier: Apache-2.0 -->
# fixtures/

Packages GDCS ships to run against a release. Each says how it was made, so it can be made again.

## `isolation-probe/` and `isolation-probe-<release>-x86_64.gsp`

GDCS's Stage 2 fixture (SPECIFICATION.md 4, Stage 2). `isolation-probe/` is its source - an ordinary service
that holds nothing but `log_write` and, on startup, tries the things an isolated service must not be able to do
(use a fabricated capability, hand the kernel a kernel-space pointer, make a gated query, reach the machine's
power, mint a send capability to a non-peer, guess past the syscall surface), logging whether each was refused.
`gdcs run` installs it like any package and reads those lines. The `.gsp` is the built, signed package, bundled
into the `gdcs` binary (`include_bytes!`) so a run needs no build step; one per release it is built for.

**The signing key is a THROWAWAY, in plain sight on purpose.** It is a fixture: a machine trusts it only if its
operator runs `godspeed trust add gdcs <public key>`, which GDCS's run does and then leaves (it is a disposable
QEMU machine). It vouches for nothing.

```
seed (secret):  1111111111111111111111111111111111111111111111111111111111111111
public key:     ed25519:d04ab232742bb4ab3a1368bd4615e4e6d0224ab71a016baf8520a332c9778737
```

Built and signed with the developer CLI, against the release named in the file:

```
cd isolation-probe
GODSPEED_SIGNING_KEY=<the seed> godspeed package --arch x86_64 --local
# then rename target/package's .gsp to isolation-probe-<release>-x86_64.gsp here
```

**Rebuild it for a new release.** The bundled package is built for one GodspeedOS release (its name is in the file
name and in its manifest's `[os] release`). Against a different release, `gdcs run` reports the Stage 2 attempts
NOT RUN and says to rebuild this against that release. Create the project fresh with the CLI's current `service`
template for that release (`godspeed new service isolation-probe --os-release <release> --publisher gdcs`), copy
`src/main.rs` and the `godspeed-sdk` dependency line onto it, build, sign and commit the new `.gsp`.
