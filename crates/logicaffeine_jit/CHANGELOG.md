# Changelog

All notable changes to this crate will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

## [0.10.0] - 2026-07-08

### Added
- Initial release. The LOGOS native tier — the bridge from the bytecode VM's tier-up seam to the copy-and-patch forge JIT.
- `ForgeTier` translates VM bytecode (`Op`) into the forge's `MicroOp` subset for both whole functions (`ChainFn`) and hot loop regions (`RegionChain`), compiling each to a native stencil chain or the register-allocating backend via `logicaffeine-forge`.
- `install()` makes the tier process-wide; every live VM constructor picks it up, and `largo` installs it at startup.
- Deopt contract: anything outside the supported integer/float subset declines (`compile_* -> None`) and stays on bytecode, so correctness never depends on the JIT accepting a program.
- Native-only (`#![cfg(not(target_arch = "wasm32"))]`); WASM builds compile it to nothing and the browser runs pure bytecode. See the root CHANGELOG for cross-crate context.
