# Changelog

All notable changes to this crate will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Fixed
- Query parameters survive router boot: `/studio?file=`, `/success?session_id=`, `/registry?token=&login=`, and `/news?tag=` are now typed query segments on their routes (received as props), so the router's startup URL normalization can no longer destroy them. Studio share links and refreshes open the linked file; Stripe license activation and the registry OAuth callback work again. Locked by route round-trip tests, a source ratchet forbidding query-string scraping, and sitemap perfection locks (enum-completeness, URL hygiene, robots.txt agreement, `lastmod` freshness).

## [0.10.0] - 2026-07-08

Synced to workspace version 0.10.0. See root CHANGELOG for full history.

## [0.8.12] - 2026-02-14

Synced to workspace version 0.8.12. See root CHANGELOG for full history.

## [0.6.0] - 2026-01-17

Initial release (web application, not published to crates.io).

### Added

- Web-based IDE using Dioxus 0.6
- Interactive curriculum with exercises
- Vocabulary reference component
- User profile page
- Universal navigation
- WASM compilation for browser deployment
