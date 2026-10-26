# mmu-fault-probe

A Godspeed service, created by `godspeed new service mmu-fault-probe` against GodspeedOS v0.31.0.

## Files

- `src/main.rs` - the service. It starts at `service_main` and never returns.
- `contracts/mmu-fault-probe.toml` - the authority the service **requests**. It grants nothing; the OS decides.
- `godspeed.toml` - what kind of project this is, for the `godspeed` CLI.
- `rust-toolchain.toml` - the compiler GodspeedOS v0.31.0 is built with.

## Build

    godspeed check                 # validate the project and its contract
    godspeed build                 # build for x86_64
    godspeed build --arch riscv64  # or aarch64, armv7
    godspeed build --all           # every architecture the OS supports

## Run and test

Package and publish it (`godspeed package`, `godspeed publish`), then on a Godspeed machine (GodspeedOS 0.29.0
or later):

    godspeed fetch <publisher>/mmu-fault-probe
    godspeed install <publisher>/mmu-fault-probe

The supervisor starts it, and starts it again at every boot. `godspeed run` and `godspeed test` refuse a service,
and say why: nothing yet boots a machine with your program already on it.
