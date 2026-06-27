# Changelog

All notable changes to this crate will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

## [0.9.17] - 2026-06-11

This crate's per-crate changelog begins at 0.9.17; the crate has shipped since 0.8.0 (see the root CHANGELOG and news for earlier history).

### Added
- Language Server Protocol implementation for `.logos` files (`tower-lsp` + `tokio`), with full document re-analysis on change and a per-document symbol index.
- 14 capabilities: diagnostics, hover, semantic tokens, go-to-definition, completion, signature help, code actions, rename, folding ranges, inlay hints, code lens, and formatting. See the root CHANGELOG for cross-crate context.
