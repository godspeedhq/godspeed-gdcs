# Godspeed Driver Conformance Suite (GDCS) - Specification

**Status:** draft 0.1, 2026-10-24. Where this document names something GodspeedOS already has, it says so and
names it; everything else is a requirement on GDCS, not a description of it.

**What is built (2026-10-26), and what is not.** Stage 1 in full, offline (`gdcs check`), and the Basic profile on
x86-64 in QEMU (`gdcs run`): Stage 3 for the behaviour a driver declares, and Stage 2 by two probes - one that
attempts and is refused the things an isolated service must not do (a fabricated capability, a kernel-space
pointer to a syscall, a gated query, the machine's power, a send capability to a non-peer, a guess past the
syscall surface), and one that reads an address it was never granted and is killed by the kernel while the
machine keeps running - plus the grant and whether DMA can be confined. And, with `gdcs run --recovery`, the
Recovery profile: Stage 4 (MAX CARNAGE) on a second machine, as far as a register-only driver allows - killed
past the restart cap, recovered then bounded, frames reclaimed, the kernel and other services untouched. Each
check has been seen to fail. **Not built:** the device-specific carnage (flood, hot-plug, device-reset) and the
lateness auto-rerun (Stage 4 below), the other architectures, physical hardware, Stage 5, and the CLI commands
of section 7. The README says how to run what is built.

## 1. Purpose

The Godspeed Driver Conformance Suite (GDCS) is an independent framework for testing drivers written for
GodspeedOS, and for saying, in a report anyone can check, what was shown and how.

It exists so that a driver can be written, ported, tested and maintained - by a person or by an AI coding
agent - without knowing GodspeedOS's kernel internals. GDCS checks a driver against published, versioned
contracts, against the authority it was granted, against the recovery the system promises, and against the
device it drives.

**Core principle:** trust enforceable contracts and independently verified behaviour, not the method used to
produce the code.

**And its corollary, which is most of what this document is about:** a result is only as good as the evidence
behind it. GDCS says what it ran, where, and what it did not run. A PASS in a simulator is not a PASS on
silicon, a check that has never been seen to fail is not a check, and a driver that was never exercised is
"built, not seen" - never "conformant".

## 2. Architectural Philosophy

GodspeedOS is a capability-based microkernel. The kernel's whole job is MISCIS:

- Memory isolation
- IPC
- Scheduling
- Capability enforcement
- Interrupt routing
- SMP routing

Drivers are isolated userspace services. The supervisor starts and restarts them; it alone decides which
driver a device gets. A driver developer never needs to understand or change the kernel, and GDCS must never
require it to.

A driver reaches GodspeedOS through stable, documented interfaces only (section 3.1). A buggy driver must not
be able to compromise the kernel or unrelated services, and its failure should be recoverable without
restarting the machine wherever that is technically possible.

**Where isolation stops, said plainly.** Isolation is enforced by the kernel and the hardware, never by the
driver's good behaviour - and it is not uniform. A driver granted a DMA arena on a machine whose IOMMU does not
confine its device can point the device at any address in memory: it is kernel-equivalent, inside the trusted
computing base by necessity (GodspeedOS `CLAUDE.md` 6.4). On the boards GodspeedOS supports today, only AMD
x86 machines confine DMA at all; every Raspberry Pi, the VisionFive and Intel PCs do not. GDCS reports this as
a property of the test machine, per run, and never reports a DMA driver's isolation as PASS on a machine that
cannot enforce it (section 4, Stage 2).

## 3. Repositories and Interfaces

Organisation: GodspeedHQ.

### `godspeed-os` (GPL-2.0-only, with Apache-2.0 libraries)

The kernel, the supervisor, hardware enumeration (`hw-enumerator`), the USB hosts (`xhci`, `ehci`, `dwc2`),
the in-tree drivers and core services, and the OS's own test suites (`osdev`). Its standard library (`gs`,
`stdlib/rust`) and SDK (`sdk/rust`) are Apache-2.0.

### `godspeed-cli` (GPL-2.0-only, with an Apache-2.0 OR MIT package crate)

The developer's interface: `godspeed new driver`, `check`, `package`, `verify`, `inspect`, and later the
GDCS commands of section 7. The CLI invokes GDCS and shows its results; it does not contain the test engine.

Its package format crate, `crates/godspeed-package` - the manifest, its signature, the digests and the `.gsp` file
- is Apache-2.0 OR MIT, so GDCS reads packages with the same code the CLI and the OS use (godspeed-cli `1676e4d`).
The CLI itself is GPL-2.0-only. *(This section said the CLI was Apache-2.0; it never was.)*

### `godspeed-gdcs` (Apache-2.0)

This repository: conformance profiles, test definitions, the harness, simulated devices and fixtures,
capability isolation tests, fault injection, crash and recovery tests, MAX CARNAGE, live replacement tests,
and the report format. GDCS runs on its own - in CI, without the CLI - given a driver package and a GodspeedOS
image.

### 3.1 The interfaces GDCS tests against

GDCS tests a driver only against what GodspeedOS publishes and versions:

| Interface | What it is | Where it lives |
|---|---|---|
| The platform file | `godspeed-platform.toml`, published with each release: OS version, standard library version, toolchain and targets, entry symbol, contract schema | each `godspeed-os` release |
| The contract schema | what a driver may request: one device by PCI class and BAR (`hw_pci_class`, `hw_pci_bar`), its interrupt (`hw_pci_irq`), a DMA arena by size (`hw_dma`, at most 2 MiB), or a USB device by vendor and product (`hw_usb`) | `contracts/schema/service.schema.json` |
| `gs::driver` | the driver library: `registers`, `arena`, `irq`, `wait`, `delay`, `bus` (a USB device through its host), `input`, `power`, `device` | `stdlib/rust/src/driver/` |
| The package format | the signed `.gsp` file and manifest, one binary per architecture | `godspeed-cli`, `docs/package-format.md` |
| The install grant | what the operator approves at `godspeed install`: the device, and DMA memory by a second question | `utilities/61_godspeed.md` |

`gs::driver` is the interface GodspeedOS's compatibility promise covers. A driver may also call the SDK
(`godspeed_sdk`) directly - nothing refuses it and it grants nothing extra - but the SDK may change between
releases. GDCS reports SDK use as a portability finding, not a failure (section 4, Stage 1).

GDCS uses no kernel interface a driver could not use, and adds no kernel responsibility to make testing
easier (section 11).

## 4. Conformance Stages

Every test in every stage has:

- **an identifier and a version**, so a report says exactly what ran;
- **a stated acceptance criterion**, measurable, written before the test is run;
- **an environment** it ran in (section 7.2): simulated, or which physical board;
- **a demonstrated failure**: each test is run at least once against a driver or image built to fail it, and
  is not counted until it has been seen to fail. A check that has never fired is not evidence of anything.

### Stage 1 - Contract Compliance

The driver:

- builds against the published platform file for its target release;
- has a contract the schema accepts, naming one device and only the keys its kind allows (a USB driver asks
  for no interrupt and no DMA of its own: they are its host's);
- requests no DMA beyond 2 MiB, no privilege, and no send peer it cannot justify;
- matches its signed manifest: contract and binaries by digest, a binary for each architecture it claims;
- uses only supported IPC interfaces, within protocol limits and message sizes (4 KiB);
- contains no `unsafe` of its own (`#![deny(unsafe_code)]`, as every GodspeedOS service does); hardware access
  goes through `gs::driver`'s audited accessors.

Reported, not failed: direct SDK calls (portability), and a contract that asks for more than the driver
exercises (least privilege).

GDCS must enforce at `check` time what the supervisor refuses at install (a 2 MiB bound, the USB key
exclusions). A package that passes every offline check and is refused only on the machine is a GDCS defect.

### Stage 2 - Capability Isolation

The driver cannot:

- reach an MMIO region it was not granted;
- acquire a capability it was not given;
- reach another service's memory;
- receive an interrupt that is not its device's;
- reach memory outside its DMA arena, **where the machine confines its device**;
- widen its authority through malformed requests to the supervisor, its host or any peer.

Each is shown by the driver, or a probe in its place, attempting it and the system refusing it.

**As built (2026-10-26): two probe packages GDCS ships, each a service holding nothing but `log_write`,
installed like any package, using only the published SDK (no kernel interface a driver could not use, section
11).** `fixtures/isolation-probe` makes the attempts a running service can make and SURVIVE to report, and logs
whether each was refused: a fabricated capability (`CapNotHeld`), a kernel-space or null pointer handed to a
syscall (rejected at the user-pointer check, the caller surviving), a gated query without `INTROSPECT`,
`Reboot`/`PowerOff` without the capability, `AcquireSendCap` to a non-peer without `ACQUIRE_ANY`, and thousands
of out-of-range syscall numbers (each `UnknownSyscall`, no crash). `fixtures/mmu-fault-probe` makes the attempt
that KILLS the caller: it reads an address it was never granted - an unmapped MMIO window, another service's
memory, or any unmapped address, all unmapped for it and so all the same page fault. A service cannot report its
own death, so the death is the evidence: the kernel kills the faulting task at the bad access, the supervisor
reports `died - memory fault` (a fault, not a kill) and restarts it a bounded number of times before leaving it
down, and the kernel never panics. The installed-package restart bound is what makes a deliberate fault safe to
run (it was once recorded here as not built for want of that). GDCS reads both from the serial log.

**DMA, honestly.** On a machine whose IOMMU does not confine the device, "cannot reach memory outside its arena"
cannot be enforced and is not tested: the report says `NOT ENFORCEABLE ON THIS MACHINE` and names the machine.
A driver with registers only, or a USB driver, carries no such reach and is tested in full everywhere.

**An isolation result is claimed only where the environment exercised it.** Confinement that exists is not
confinement shown: a result says DMA isolation PASSED only when a DMA outside the arena was attempted and the
environment refused it. QEMU's emulated `amd-iommu` installs the tables but does not fault on an unmapped
access (GodspeedOS `CLAUDE.md` 22, Test 12), so in QEMU confinement can be checked only structurally - the
device's I/O page table walked - and is reported as that, on its own line, never folded into a PASS.

### Stage 3 - Functional Correctness

The driver does what its contract and documentation say it does, shown with:

- simulated devices (QEMU's devices where they exist, GDCS's own where they do not);
- device protocol tests, boundary conditions, invalid input and timeouts;
- physical hardware, where available.

**What a driver claims to do is part of the result.** A driver that identifies a device and stops is a probe,
and is reported as one; a functional pass covers only the behaviour the driver declares. Simulation results and
hardware results are reported separately and never merged into one PASS.

### Stage 4 - MAX CARNAGE

The driver under adversarial conditions:

- killed during active operations, repeatedly;
- its IPC queue flooded; malformed messages injected;
- memory pressure;
- its device disconnected and reconnected, during initialisation and after;
- forced timeouts; its host, the supervisor and its dependants restarted under it;
- device reset failures;
- resource cleanup checked after every termination.

**Required outcome:** the driver may fail; the kernel and unrelated services may not. The supervisor detects
each failure and applies the recovery policy, and recovery is bounded: today an installed driver package is
restarted at most five times a boot, then left down and said so.

**Measured, not only survived.** Lessons GodspeedOS learned the hard way, each a GDCS requirement:

- **Floods.** A peer that keeps sending to a dead driver is a fault even when everything survives. GDCS counts
  refused sends (the kernel's stale-capability refusals) while a driver is down, and fails a run whose count
  grows with time rather than staying bounded.
- **Input integrity.** Nothing a driver does may answer a prompt or type into the console on its own (a USB
  host's plug notice once arrived as an Enter and answered an open `[y/N]`). GDCS holds a question open and a
  line half-typed across every hot-plug it injects.
- **Recovery that cannot work.** Some devices recover only by losing power (a USB WiFi chip that stops answering
  after a failed firmware block); no software reset revives them. GDCS reports such a run as
  `MANUAL INTERVENTION REQUIRED`, with the evidence, rather than as a pass or a hang.
- **Lateness is not failure.** A watchdog that halts on a late core proves lateness, not a fault. GDCS runs
  under a declared host load, reruns a failure once on the same build, and reports a failure that does not
  recur as a flake with both results - never silently as a pass.
- **No order dependence.** A test that kills a service (the clock, say) must not make the next test fail for a
  reason of its own. Each test states its preconditions and waits for them.

**As built (2026-10-26), for a register-only driver on its own fresh machine (`gdcs run --recovery`,
`src/carnage.rs`).** The core required outcome is run and measured: the driver is killed one past the
installed-package restart cap, and GDCS checks it was recovered each time (`S4.recover`), that recovery is
bounded - restarted exactly `RESTARTS_MAX` times then left down, loudly (`S4.bounded`) - that every death
reclaimed the task's frames (`S4.cleanup`, the kernel's `freed N frames`, none zero), that the kernel and
unrelated services are untouched (`S4.bystanders`: the shell answers, a file round-trips through `fs`,
`net-stack` still pings), and that nothing panicked or wedged (`S4.kernel`). The storm runs on a SECOND machine
so the restart cap starts at zero and nothing else is disturbed.

**Recorded, not yet built (CLAUDE.md 26.7).** The device-specific carnage needs a driver this register-only
subject is not: the **flood** (refused sends counted while down) needs a client peer, and **device
disconnect/reconnect** and a **device-reset failure** need a hot-pluggable device (its device is PCI `edu`,
which QEMU does not hot-plug). **Input integrity** across a hot-plug likewise needs the hot-plug. **Memory
pressure** is not injected. And the **lateness** rule is not automated: a `LIVENESS WEDGE` under the storm is
reported as what it is, with a note to rerun once (backlog/49), rather than GDCS rerunning on its own. These
are named in the report's detail where they would otherwise look like silent gaps.

### Stage 5 - Live Replacement

A driver replaced while GodspeedOS runs:

1. V1 is running and owns its device.
2. V2 is deployed, checked (Stage 1) and started beside it.
3. State is handed over, or reconstructed, where V2 supports it.
4. Device ownership moves at a safe boundary; the two never both own the device.
5. Clients are redirected to V2.
6. V1 ends, and its resources are reclaimed.

**What GodspeedOS has today, so GDCS can be honest about it.** A service can be replaced live
(`gs::handover`, opt-in, one message of state, `CLAUDE.md` 2.5 amendment 2026-10-17). A driver package can take
a device over from an in-tree driver and give it back, stopping one before starting the other. Live replacement
of a driver while it owns a device is not built. GDCS reports what was actually achieved - full continuity, a
measured gap, or `NOT SUPPORTED` - and never infers one from another. Not every device can be handed over
without interruption; that is a property of the device, recorded as such.

## 5. Conformance Profiles

Profiles are versioned. Each names its required tests and their acceptance criteria; a profile with an unstated
criterion is incomplete, not lenient.

| Profile | Requires |
|---|---|
| **Basic** | Stage 1; Stage 2 as far as the machine enforces it; Stage 3 for the declared behaviour |
| **Recovery** | Basic, and Stage 4: containment, bounded supervisor recovery, resource cleanup |
| **Live Replacement** | Recovery, and Stage 5: safe replacement, state preserved or reconstructed |
| **Continuity** | Live Replacement, with no externally observable interruption within the device's declared tolerances |

A driver does not fail Basic for lacking live replacement. Profiles are also qualified by class (`network/pci`,
`usb/vendor`, ...) and by environment: a profile met only in simulation is reported as `simulated`.

## 6. AI-Assisted Contributions

GodspeedOS welcomes AI-assisted driver development: porting drivers, implementing device protocols, reading
hardware documentation, generating tests, diagnosing failures, recovery logic, performance work, documentation.

- AI-generated contributions meet the same requirements as any other. Using AI is not grounds for rejection.
- Contributors remain responsible for legality, provenance, correctness and maintainability. A driver adapted
  from another project keeps that project's licence (section 10).
- GDCS must let an agent develop, test, repair and submit a driver using only published interfaces: every
  failure says what failed, why, and where to look, in words an agent can act on without the kernel's source.
- Human review is preferred. Automated merging may come later, for narrowly scoped and well-evidenced
  contributions, under an explicit repository policy.

Passing GDCS is evidence, not proof. It is not a guarantee of complete correctness or safety, and no report may
say otherwise.

## 7. Developer Experience

### 7.1 Commands

Illustrative:

```
godspeed driver test ./e1000      Stages 1-3
godspeed driver chaos ./e1000     Stage 4
godspeed driver certify ./e1000   the profile, end to end, with a report
```

And the same without the CLI: `gdcs run --profile recovery --package e1000-0.1.0.gsp --image os.img`.

### 7.2 Reports

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

Every report is also machine-readable (JSON, with a published schema) and records: the GDCS version, the
profile and its version, the driver package and its digest, the target OS version and platform file digest,
the environment (simulator and device, or board, firmware and the image's commit), every test run with its
result and duration, every test skipped and why, reruns, and the evidence for each result (a log digest, the
lines that decided it). A result whose evidence is missing is reported as not run.

A certification never implies physical hardware verification when only simulation was performed.

### 7.3 Physical hardware

A physical run names the board and the image, keeps its serial log with the report, and states anything done
by hand (a device unplugged, a cable moved). GDCS defines the steps an operator performs and the lines that
decide each result, so a hardware run takes minutes and is repeatable.

## 8. Failure and Recovery

Failure is contained. Recovery is automatic where it is safe. Replacement does not need an OS restart wherever
that is achievable.

- A driver crash does not require a reboot; neither does a driver update.
- A driver that cannot recover says so clearly, and does not destabilise unrelated services.
- Recovery accounts for hardware state, in-flight operations, persistent data and the device's reset
  requirements.
- Some failures need manual intervention, a device reset or a replacement part. GDCS reports them as that.

## 9. Kernel Live Upgrade

GodspeedOS aims, eventually, to update every component without a restart, the kernel included. That is a
separate research problem: the kernel owns the isolation every service relies on, so a kernel upgrade must
preserve or safely reconstruct its state and keep capability security across the change, without growing the
trusted computing base without compelling reason. It is not a prerequisite for GDCS, and GDCS does not depend
on it.

## 10. Licensing

`godspeed-gdcs` is Apache-2.0 (`LICENSE`), with SPDX identifiers on every file and required notices kept.

- Permissive for open-source and commercial use; modification and redistribution allowed; an explicit
  contributor patent grant; no copyleft on drivers tested with it.
- Contributions under the Developer Certificate of Origin (DCO), including AI-assisted ones.
- Third-party code, emulators, fixtures and adapted drivers keep their own licences.
- **GodspeedOS's kernel, services and test harness are GPL-2.0-only.** GDCS may run them, but must not copy
  their code into this repository without that code being relicensed by its owner. Its interfaces are the
  Apache-2.0 `gs` library and SDK and the published contract schema and platform file.

## 11. Design Constraints

GDCS must:

- stay independent of the kernel's implementation, testing only published, versioned interfaces;
- run without the full Godspeed CLI, and in CI;
- distinguish simulation from physical hardware in every result;
- produce reproducible, machine-readable reports;
- support adversarial and failure-injection testing;
- add no kernel responsibility to make testing possible; a capability a test needs is the same one a driver
  or the operator already has;
- show each of its own checks failing before relying on it;
- grow without new repositories or services unless one is clearly needed.

## 12. Long-Term Vision

A developer should be able to give an AI agent existing driver source or a hardware specification, the
`gs::driver` library and its documentation, the device's protocol, and the GDCS profile - and get back a
userspace driver that passes it, with the report to show, ready to contribute.

The kernel stays small, stable and rigorously audited. The userspace ecosystem evolves quickly, with
AI-assisted engineering, without asking the kernel to change with it. GodspeedOS should make experiments
cheap, failures containable, recovery predictable and replacing software routine.
