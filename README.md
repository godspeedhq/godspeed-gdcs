# godspeed-gdcs

**The Godspeed Driver Conformance Suite (GDCS)**: an independent way to test a GodspeedOS driver and to say,
in a report anyone can check, what was shown, where, and what was not.

> **Status: specification only.** Nothing here is built yet. [`SPECIFICATION.md`](SPECIFICATION.md) is the
> draft (0.1, 2026-10-24) and the place to start.

## Why it exists

GodspeedOS drivers are userspace services on a capability microkernel. A driver developer - a person or an AI
coding agent - should be able to write, port and fix a driver without knowing the kernel, and know whether it
is good enough without asking someone who does.

GDCS answers that question from the outside: against published interfaces, with the kernel and the hardware
enforcing what they enforce, and with every result tied to its evidence.

**Trust enforceable contracts and independently verified behaviour, not the method used to produce the code.**

## Where it sits

```
  godspeed-cli                 godspeed-gdcs                    godspeed-os
  (developer's tool)           (this repository)                (the system under test)

  godspeed new driver          profiles  +  test definitions    a released image
  godspeed package     ---->   harness   +  simulated devices   ----> QEMU, or a board
  godspeed driver test         fault injection, MAX CARNAGE
                               reports (text + JSON)  <-------- serial log, results
         ^                              |
         +------------ report ----------+
```

GDCS runs without the CLI too, in CI: given a driver package (`.gsp`) and a GodspeedOS image, it runs a
profile and writes a report.

## What a driver is tested against

Only what GodspeedOS publishes and versions - never its kernel's internals:

```
  +------------------------------------------------------------------+
  |  the driver package (.gsp): manifest, signature, contract,       |
  |  one binary per architecture                                     |
  +----------------------------+-------------------------------------+
                               | built on
  +----------------------------v-------------------------------------+
  |  gs::driver   registers  arena  irq  wait  delay  bus  ...       |  <- the compatibility promise
  |  (the SDK underneath: allowed, but outside the promise)          |
  +----------------------------+-------------------------------------+
                               | grants checked by
  +----------------------------v-------------------------------------+
  |  supervisor   one device per driver, approved by the operator;   |
  |               restarts bounded (5 a boot)                        |
  +----------------------------+-------------------------------------+
                               | enforced by
  +----------------------------v-------------------------------------+
  |  kernel (MISCIS)  memory isolation, IPC, scheduling,             |
  |                   capabilities, interrupts, SMP routing          |
  +------------------------------------------------------------------+
```

The published pieces are the platform file (`godspeed-platform.toml`), the contract schema, `gs::driver`, the
package format and the install grant. [SPECIFICATION.md 3.1](SPECIFICATION.md#31-the-interfaces-gdcs-tests-against)
lists them.

## The five stages

```
  1 Contract       2 Isolation      3 Function       4 MAX CARNAGE     5 Live
    compliance  ->   (as far as  ->   (declared   ->   kill, flood,  ->  replacement
    builds,          the machine      behaviour,       unplug, reset,    V1 -> V2 at a
    schema,          enforces it)     simulated and    bounded           safe boundary
    signature                         hardware apart)  recovery
```

| Profile | Stages |
|---|---|
| Basic | 1, 2, 3 |
| Recovery | Basic + 4 |
| Live Replacement | Recovery + 5 |
| Continuity | Live Replacement, with no observable interruption within the device's tolerances |

A driver does not fail Basic for lacking live replacement.

## The rules GDCS keeps about itself

- **Every check is seen to fail** against a driver built to fail it before it counts.
- **Simulation is not hardware.** Every result names its environment; the two are never merged into one PASS.
- **No result without evidence.** A log digest and the lines that decided it, or the test reads "not run".
- **Isolation is reported as the machine enforces it.** A DMA driver on a machine whose IOMMU does not confine
  its device is `NOT ENFORCEABLE ON THIS MACHINE`, never PASS.
- **Survival is not enough.** Floods of refused sends, input injected into the console, recovery that cannot
  work and order-dependent tests are failures or named outcomes, not passes.
- **No kernel change for testing.** A test uses only authority a driver or the operator already has.

## A report

```
Godspeed Driver Conformance Suite 0.1.0
Driver:          godspeed/e1000 0.1.0 (sha256 4f1c...)
Target:          GodspeedOS 0.31.0 (platform file sha256 9a2e...)
Profile:         network/pci, Recovery 1.0
Environment:     simulated - QEMU 9.1, q35, e1000 (8086:100e)

Contract compliance       PASS
Capability isolation      PASS  (MMIO, capabilities, interrupts)
DMA isolation             NOT SHOWN  (QEMU's amd-iommu maps but does not fault;
                                      page tables checked, no refused access)
Functional tests          PASS  12/12
Crash recovery            PASS  restarts bounded at 5
Refused sends while down  PASS  1 (bounded)
Device reset              PASS
MAX CARNAGE               PASS  100 rounds, 0 kernel faults
Live replacement          NOT SUPPORTED

Conformance level:        Recovery (simulated)
Physical hardware:        not tested
RESULT: CONFORMANT (simulated)
```

Illustrative: no part of this is built yet.

## AI-assisted drivers

Welcome, and held to the same bar as any other. GDCS is designed so that an agent can develop, test, repair and
submit a driver from published interfaces alone, with every failure saying what failed, why and where to look.
Passing GDCS is evidence, not proof.

## Licence

Apache-2.0 ([`LICENSE`](LICENSE)). Contributions under the Developer Certificate of Origin, AI-assisted ones
included. GodspeedOS's kernel, services and test harness are GPL-2.0-only: GDCS runs them, and does not copy
their code here.
