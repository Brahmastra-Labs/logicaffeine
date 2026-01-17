# Changelog

All notable changes to this crate will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

## [0.6.0] - 2026-01-17

Initial crates.io release.

### Added

- Platform IO abstraction layer
- Networking support via libp2p (optional `networking` feature)
- Persistence layer with memmap2 (optional `persistence` feature)
- Concurrency via tokio and rayon (optional `concurrency` feature)
- Distributed coordination (optional `distributed` feature)
- Synced<T> wrapper for CRDT network synchronization
- Feature flags: `networking`, `persistence`, `concurrency`, `full`
