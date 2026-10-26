//! mmu-fault-probe: GDCS's Stage 2 fixture for the attempts that KILL the caller.
//!
//! Some of Stage 2's attempts cannot be reported by the service that makes them: dereferencing an address the
//! service was never granted - an MMIO window, another service's memory, or any unmapped address - does not
//! return an error, it takes a CPU fault, and the kernel kills the faulting task. A service cannot log "I was
//! refused" after it is dead. So here the DEATH is the evidence, and GDCS reads it from the supervisor.
//!
//! On startup this service announces the address it is about to read and then reads it. In a per-address-space
//! microkernel every address it was not granted is simply not mapped for it, so address 0 - the cleanest
//! unmapped address - faults exactly as an ungranted MMIO or cross-service address would. A conforming kernel
//! kills this task at the ring-3 page fault and keeps running (GodspeedOS CLAUDE.md 22, A14/A15); the supervisor
//! reports `died - memory fault at 0x0` (a fault, not a kill) and restarts it a bounded number of times, then
//! leaves it down. GDCS asserts all of that, and that the kernel never panicked.
//!
//! The deliberate fault goes through `godspeed_sdk::adversarial`, which wraps the one `unsafe` it needs, so this
//! crate holds none of its own. A model service would never do this; it is a test instrument.

#![deny(unsafe_code)]
#![no_std]
#![no_main]

use godspeed::ServiceContext;
use godspeed_sdk::adversarial;

#[allow(unsafe_code)]
#[no_mangle]
pub extern "C" fn service_main(ctx: ServiceContext) -> ! {
    ctx.log("mmu-fault-probe: reading 0x0, an address this service was never granted (like any ungranted MMIO or cross-service address, it is unmapped for this task)");
    // A read of an unmapped address. The kernel must kill this task at the fault, not wedge or reset the
    // machine. It never returns; the next line is never reached, and the supervisor sees a memory fault.
    adversarial::fault_null_read();
    ctx.log("mmu-fault-probe: FAIL - the read of 0x0 returned; the kernel did not refuse it");
    loop {
        core::hint::spin_loop();
    }
}
