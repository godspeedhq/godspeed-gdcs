//! isolation-probe: GDCS's Stage 2 fixture.
//!
//! An ordinary isolated service that holds nothing but `log_write`, and on startup tries, one after another,
//! the things an isolated service must not be able to do. Each attempt uses only what any service can reach -
//! the published SDK, no kernel interface a driver could not use (SPECIFICATION.md 11) - and each must be
//! REFUSED with a defined error rather than succeeding or crashing the machine. The probe survives every
//! attempt (none is a deliberate fault, so the supervisor never has to restart it) and logs one line each:
//!
//!   isolation-probe: <id> PASS - <what was refused>
//!   isolation-probe: <id> FAIL - <what leaked>
//!
//! and finally `isolation-probe: done, N of M refused`. GDCS reads those lines (`src/isolation.rs`). A FAIL,
//! or a line that never arrives because the machine stopped, is a Stage 2 failure.
//!
//! The raw and malformed syscalls go through `godspeed_sdk::adversarial`, which wraps the one `unsafe` the
//! ABI needs, so this crate holds none of its own (`#![deny(unsafe_code)]`). That direct SDK use is why
//! `gdcs check` reports an SDK FINDING for this package: it is a test instrument, not a model driver.

#![deny(unsafe_code)]
#![no_std]
#![no_main]

use godspeed::{self as gs, ServiceContext};
use godspeed_sdk::adversarial;
use godspeed_sdk::{CapHandle, Message};

/// `cap_err_to_i64(CapNotHeld)` in the kernel's dispatch: the code a gated syscall returns to a holder-less caller.
const CAP_NOT_HELD: i64 = -2;
/// `UnknownSyscall`: what every syscall number outside the valid range returns.
const UNKNOWN_SYSCALL: i64 = -1;

#[allow(unsafe_code)]
#[no_mangle]
pub extern "C" fn service_main(ctx: ServiceContext) -> ! {
    ctx.log("isolation-probe: started");
    let mut passed = 0u32;
    let mut total = 0u32;
    let mut check = |ok: bool, id: &str, refused: &str, leaked: &str| {
        total += 1;
        if ok {
            passed += 1;
            ctx.log_fmt(format_args!("isolation-probe: {id} PASS - {refused}"));
        } else {
            ctx.log_fmt(format_args!("isolation-probe: {id} FAIL - {leaked}"));
        }
    };

    // forged-cap: a capability handle this service was never given (a high, out-of-range slot) cannot be used.
    // The kernel's cap table has no such entry, so a send on it is CapNotHeld - a fabricated token is not authority.
    let forged = gs::cap::Cap::from(CapHandle(0xffff_fffe));
    let msg = Message::from_bytes(b"forged");
    check(
        gs::ipc::try_send_to(&ctx, forged, &msg).is_err(),
        "forged-cap",
        "a fabricated capability handle is CapNotHeld, not authority",
        "a fabricated capability handle was accepted as authority",
    );

    // kernel-pointer: kernel-space and null addresses handed to a syscall as a buffer pointer are rejected by
    // the kernel's user-pointer check, and the caller is told so rather than faulting or reading kernel memory.
    // This is the syscall-boundary face of "cannot reach memory it was not granted".
    const KERNEL_ADDRS: &[u64] = &[0xffff_8000_0000_0000, 0xffff_ffff_ffff_fff0, 0x0, 0x0000_8000_0000_0000];
    let mut pointer_ok = true;
    for &addr in KERNEL_ADDRS {
        // Send (syscall 2) and Recv (syscall 3) with the address as the message/buffer pointer. A conforming
        // kernel returns an error for each; none may succeed, and the probe must still be alive afterwards.
        if adversarial::fuzz_syscall(2, 0, addr, 4096) >= 0 || adversarial::fuzz_syscall(3, 0, addr, 4096) >= 0 {
            pointer_ok = false;
        }
    }
    check(
        pointer_ok,
        "kernel-pointer",
        "kernel and null addresses as syscall buffer pointers are refused; the caller survives",
        "a kernel or null address as a syscall buffer pointer was accepted",
    );

    // introspect: a gated kernel query (TaskStat) is denied without the INTROSPECT capability, which this
    // service does not hold. TaskStat(0) targets a live TCB task, so a denial reads back invalid; a valid
    // answer would mean the gate is open. An ungated query (the tick clock) must still work.
    let gated_denied = !ctx.task_stat(0).valid;
    let ambient_open = gs::driver::wait::ticks(&ctx) != 0;
    check(
        gated_denied && ambient_open,
        "introspect",
        "a gated InspectKernel query is denied without INTROSPECT; ungated ones still answer",
        if gated_denied { "an ungated query stopped answering" } else { "a gated InspectKernel query answered without INTROSPECT" },
    );

    // reboot: Reboot and PowerOff, now and deferred, are denied without their capabilities, which this service
    // does not hold - and nothing is left pending. An open gate would not return (the machine would reset, turn
    // off, or hold a deadline), so a returned CAP_NOT_HELD with no deadline pending is the proof.
    let reboot_denied = ctx.try_reboot() == CAP_NOT_HELD && ctx.power_off() == CAP_NOT_HELD;
    let deferred_denied = ctx.reboot_in(5) == CAP_NOT_HELD && ctx.power_off_in(5) == CAP_NOT_HELD;
    let nothing_pending = ctx.power_pending().is_none();
    check(
        reboot_denied && deferred_denied && nothing_pending,
        "reboot",
        "Reboot and PowerOff, now and deferred, are denied without the capability; the machine is intact",
        "the machine's power or a power deadline was reachable without the capability",
    );

    // acquire: minting a SEND capability to another service by name is denied without ACQUIRE_ANY and with no
    // declared peer, which is this service's position. `events` is a real registered service, so acquiring it
    // would be ambient send authority - exactly what the capability model forbids.
    check(
        gs::cap::acquire(&ctx, "events").is_err(),
        "acquire",
        "AcquireSendCap to a non-peer is denied without ACQUIRE_ANY",
        "a SEND capability to a non-peer was minted without ACQUIRE_ANY",
    );

    // fuzz: thousands of syscall numbers outside the valid range, with arbitrary arguments, each return
    // UnknownSyscall and never crash the kernel. A service cannot reach past the syscall surface by guessing.
    let mut rng = 0x1234_5678_9abc_def1u64;
    let mut bad = 0u32;
    for _ in 0..20_000u32 {
        let raw = xorshift64(&mut rng);
        let nr = if raw <= 60 { raw + 100 } else { raw };
        let (a0, a1, a2) = (xorshift64(&mut rng), xorshift64(&mut rng), xorshift64(&mut rng));
        if adversarial::fuzz_syscall(nr, a0, a1, a2) != UNKNOWN_SYSCALL {
            bad += 1;
        }
    }
    check(
        bad == 0,
        "fuzz",
        "20000 out-of-range syscall numbers each returned UnknownSyscall; the kernel did not crash",
        "an out-of-range syscall number returned something other than UnknownSyscall",
    );

    ctx.log_fmt(format_args!("isolation-probe: done, {passed} of {total} refused"));
    gs::ipc::park(&ctx)
}

/// A tiny deterministic PRNG, so a run is reproducible and nothing here needs the clock.
fn xorshift64(s: &mut u64) -> u64 {
    let mut x = *s;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    *s = x;
    x
}
