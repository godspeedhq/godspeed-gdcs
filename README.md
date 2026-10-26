# godspeed-gdcs

**The Godspeed Driver Conformance Suite (GDCS)**: an independent way to test a GodspeedOS driver and to say,
in a report anyone can check, what was shown, where, and what was not.

> **Status: the Basic and Recovery profiles, in QEMU on x86-64 and riscv64.** `gdcs check` runs Stage 1 on a package file;
> `gdcs run` adds Stages 2 and 3 on a release image in QEMU, Stage 2 by two probes GDCS installs and watches -
> one refused each thing it tries, one read of an ungranted address that the kernel kills it for while staying
> up. `gdcs run --recovery` adds Stage 4 (MAX CARNAGE) on a second machine: the driver killed over and over,
> recovered each time and then left down at the bound, the kernel and other services untouched.
> [`SPECIFICATION.md`](SPECIFICATION.md) is the draft (0.1) and says exactly what is built and what is not;
> [Running it](#running-it) is how.

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

Illustrative: the shape of a Recovery report. Recovery (Stage 4) is not built yet; a Basic run is below.

## Running it

GDCS is a Rust program (`cargo build --release`; the binary is `gdcs`). It needs a driver package and, from the
[GodspeedOS release](https://github.com/godspeedhq/godspeed-os/releases) it targets, `godspeed-platform.toml` and,
for `run`, the image for the machine: on x86-64 the image unpacked from `godspeed-os-<tag>-x86_64.img.zip`, on
riscv64 (`--arch riscv64`) the QEMU `virt` kernel `godspeed-os-<tag>-riscv64-qemu-virt.elf`. A release ships that
kernel from the one after v0.31.0; for v0.31.0 itself there is none, and its riscv64 network driver does not drive
QEMU's e1000, so a riscv64 run needs the next release.

```
gdcs check edu-driver-0.1.0.gsp --platform godspeed-platform.toml --source ./edu-driver
gdcs run   edu-driver-0.1.0.gsp --platform godspeed-platform.toml --source ./edu-driver \
           --image godspeed-os-v0.31.0-x86_64.img --out gdcs-run
gdcs run   ... --recovery        # the Recovery profile: also Stage 4 (MAX CARNAGE)
gdcs run   ... --arch riscv64 --image godspeed-os-<tag>-riscv64-qemu-virt.elf   # on QEMU virt
```

- **`check`** is Stage 1 alone, on the file, with no machine. `--source` is the driver's project folder, for the
  two tests a binary cannot answer (no `unsafe` of its own, direct SDK use); without it they read NOT RUN.
- **`run`** is the Basic profile: Stage 1, then the package installed on the release in QEMU exactly as an operator
  would - the data disk formatted with `drives flash`, the package served from this machine and fetched with
  `download`, its key trusted with `godspeed trust add`, `godspeed install` - and watched. It needs
  `qemu-system-x86_64` and the UEFI firmware QEMU ships (`share/edk2-x86_64-code.fd`; `--qemu` and `--firmware`
  name them otherwise), or for `--arch riscv64` `qemu-system-riscv64`, whose `virt` board boots OpenSBI and the
  kernel and has the data disk as a USB stick on an xHCI (`gdcs-xhci`), since this port keeps its storage on USB. A run takes a few minutes. `--out` gets `report.txt`, `report.json` and `serial.log`, with
  the log's SHA-256 in the report.
- **`run --recovery`** is the Recovery profile: everything in Basic, then Stage 4 on a SECOND fresh machine - the
  driver killed one past the restart cap, recovered each time and then left down, with the kernel and unrelated
  services (shell, `fs`, `net-stack`) still working afterward. It keeps a `serial-carnage.log` beside the rest.
- Exit status: 0 conformant, 1 not conformant, 3 incomplete (a required test did not run), 2 the run could not start.

**The driver's declaration**, `gdcs.toml` in its project folder, says which release the driver is for, what it
claims to do, and what QEMU must give it. A functional pass covers only what is declared; a driver that declares
nothing is a probe, and says so.

```toml
format = 1

[target]
os = "0.31.0"                 # the GodspeedOS release this driver is built and tested against

[qemu]
devices = ["edu"]

[[behaviour]]
id = "identity-factorial-dma"
expect = "edu-driver: device id 0x010000ed, alive true, 5! = 120 by interrupt, 64 bytes through DMA and back intact"
within-secs = 30
```

`[target].os` is optional and declarative: GDCS checks it against the platform file and image you pass (and the
bundled probe) and refuses to start on a mismatch, so a run against the wrong release is caught before anything
boots rather than misread as a driver fault. It does not fetch anything; you still supply the platform file and
image. This is what makes a dogfood sweep - the same driver, `gdcs run ./that-driver`, against the release its
own config names - one command per driver.

That is the declaration of the driver `godspeed new driver` makes, and the Stages 2 and 3 of a `run` of it on
v0.31.0 (2026-10-26); Stage 1 is the 14 `S1.*` tests shown earlier:

```
S3.boot                              PASS       the release boots to a prompt
S3.setup                             PASS       a data disk, the package fetched, its key trusted
S3.install                           PASS       installed and started by the supervisor
S3.behaviour.identity-factorial-dma  PASS       declared behaviour
S3.running                           PASS       still running after its behaviour
S2.privilege                         PASS       given no privilege
S2.dma                               NOT ENFORCEABLE ON THIS MACHINE  DMA confined to its arena
S2.forged-cap                        PASS       a fabricated capability is refused
S2.kernel-pointer                    PASS       a kernel-space pointer to a syscall is refused
S2.introspect                        PASS       a gated kernel query is denied without INTROSPECT
S2.reboot                            PASS       the machine's power is denied without the capability
S2.acquire                           PASS       a send capability to a non-peer is denied
S2.fuzz                              PASS       out-of-range syscalls return a defined error, no crash
S2.unmapped-read                     PASS       a dereference of an address not granted faults, killing the caller, not the kernel
S3.kernel                            PASS       no kernel panic or wedge
```

`S2.dma` is `NOT ENFORCEABLE` because QEMU's `pc` board has no IOMMU; on a machine that confines the device it
reads `NOT SHOWN` (QEMU maps but does not fault) or, on silicon, `PASS`. The S2 attempts are made by GDCS's two
[probes](fixtures/), installed on the same machine: the survivable ones logged by the probe, and
`S2.unmapped-read` shown by the fault probe's death (the kernel kills it at the bad access, restarts it a bounded
number of times, and leaves it down, all without panicking).

With `--recovery`, Stage 4 follows on a second machine:

```
S4.recover                           PASS       each kill is recovered by the supervisor
S4.bounded                           PASS       recovery is bounded, then left down loudly
S4.cleanup                           PASS       every death reclaims the task's frames
S4.bystanders                        PASS       the kernel and unrelated services survive
S4.kernel                            PASS       no kernel panic or wedge under carnage

34 run: 33 passed, 0 failed, 0 not run, 1 not shown or not enforceable, 0 findings
RESULT: CONFORMANT (offline + simulated)
```

**Real drivers to try it on.** [`examples/`](examples) holds driver packages for devices QEMU emulates. Each is a
reference for one shape of driver, and each is CONFORMANT (simulated) on v0.31.0, run in QEMU on x86-64.

Every example is also **built and checked for all four architectures GodspeedOS supports** (x86_64, aarch64,
armv7, riscv64). CI packages each one for every architecture against the release its `gdcs.toml` names, and
`gdcs check` verifies each binary is an executable of its machine (the `examples` job). That shows the driver
builds against the published interfaces on every architecture.

**Run on riscv64 too** (QEMU `virt`, 2026-10-26, against the OS change that ships the virt kernel and drives the
e1000 there - not yet released):

| Example | riscv64 result | Why |
|---|---|---|
| `e1000` | 28/29, `S2.dma` NOT RUN; Recovery 33/34 | the riscv64 kernel does not say whether it can confine DMA, so the result cannot be read |
| `usb-serial` | 29/29, CONFORMANT | its device on the disk's xHCI (`[qemu.riscv64]` in its `gdcs.toml`) |
| `hda` | NOT CONFORMANT, `S2.dma` NOT RUN | riscv64 routes no PCI interrupt to a driver package, and the driver says so: its by-interrupt behaviour cannot happen |
| `xhci` | not run | its `gdcs.toml` says why: the riscv64 disk is on the xHCI it would take over |

The two gaps are the OS's, found by running: GDCS reports them rather than working around them. Arm (the Raspberry
Pis) has no network in QEMU on either port, so the examples are built and checked there but not run.

| Example | The shape it shows | Device |
|---|---|---|
| [`e1000`](examples/e1000) | PCI network card: reset, link, MAC, one frame sent by DMA | QEMU `e1000` |
| [`hda`](examples/hda) | PCI device driven through DMA command rings, answered by interrupt | QEMU `intel-hda` |
| [`usb-serial`](examples/usb-serial) | USB device driver: every transfer a request to its host | QEMU `usb-serial` (FTDI) |
| [`xhci`](examples/xhci) | host controller, brought up by registers alone (no interrupt, no DMA) | QEMU `qemu-xhci` |

The e1000 was the first real driver run through GDCS, and it found the gap PR #7 fixed: a driver that takes
the network away used to stop GDCS fetching its probes.

**Its own tests.** `cargo test` runs Stage 1's, the Stage 2 parser's, and Stage 4's: a package or a log built to
fail each check, which fails that check and no other. The QEMU cases are `#[ignore]`d and named in
`tests/basic.rs`, with the variables they need.

## AI-assisted drivers

Welcome, and held to the same bar as any other. GDCS is designed so that an agent can develop, test, repair and
submit a driver from published interfaces alone, with every failure saying what failed, why and where to look.
Passing GDCS is evidence, not proof.

## Is GodspeedOS itself tested by GDCS?

Not yet, and here is why.

GodspeedOS came first. The developer CLI, the package registry and GDCS were built after it, out of what
building it taught: the OS had to exist before anyone could see which tools a contributor would need. So most
of what ships today was written and tested without them. Its in-tree drivers are services built with the OS
and tested by its own suites (`osdev test ...`) and on hardware. None of them has been through GDCS.

New work goes through the tools. A driver contributed as a package is built and signed with the CLI, served by
the registry, and tested by GDCS. The reference drivers in [`examples/`](examples) are tested by GDCS against
every release they target.

GodspeedOS will move its own code onto the same tools where its design allows. Some parts cannot move without
a change to the OS itself:

- `console` needs a fixed framebuffer device, which a package cannot request;
- `hw-enumerator` needs PCI configuration-space access, a privilege no package is given;
- the USB hosts and `block-driver` are reached by name by the services that depend on them.

The OS will say which parts cannot move yet, and why, rather than leave it to be found.

## Licence

Apache-2.0 ([`LICENSE`](LICENSE)). Contributions under the Developer Certificate of Origin, AI-assisted ones
included. GodspeedOS's kernel, services and test harness are GPL-2.0-only: GDCS runs them, and does not copy
their code here.
