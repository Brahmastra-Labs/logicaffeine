# Changelog

All notable changes to this crate will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

This crate is an out-of-band benchmark harness (`publish = false`, version pinned
at 0.0.0); it does not follow the workspace's lockstep version.

## [Unreleased]

### Added
- Wire codec benchmark harness comparing the LOGOS wire format against Cap'n Proto, Protobuf, MessagePack, bincode, postcard, CBOR, Arrow and JSON; emits `latest-codec.json` for the web Benchmarks page.
