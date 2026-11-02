# Changelog

All notable changes to this crate will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

This crate is an out-of-band benchmark harness (`publish = false`, version pinned
at 0.0.0); it does not follow the workspace's lockstep version.

## [Unreleased]

## [0.11.0] - 2026-10-04

### Added
- **`bench_capnp` is public under the `capnproto` feature.** The head-to-head suites race the same generated Cap'n Proto readers and builders the harness does.

### Changed
- **The harness's suites moved into the test package.** `superiority`, `bulk_cursor`, `codec_report`, `fixed_view` and `random_access_guarantees` run from `crates/logicaffeine_tests/tests/`; `logicaffeine-tests` forwards `capnproto` to reach the Cap'n Proto rows.

## [0.10.0] - 2026-07-08

### Added
- Wire codec benchmark harness comparing the LOGOS wire format against Cap'n Proto, Protobuf, MessagePack, bincode, postcard, CBOR, Arrow and JSON; emits `latest-codec.json` for the web Benchmarks page.
