# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project status

The repository is at its initial stage (no code yet). This file captures the project requirements and architectural decisions agreed upon so far. Update the "Commands" and "Architecture" sections as the codebase takes shape.

## Project overview

**Next GDrive** is an open-source Google Drive sync client written in Rust. It keeps a local folder synchronized with Google Drive.

- Repository: https://github.com/diegofaraujo/nextgdrive (GitHub, default branch `main`; use the `gh` CLI for issues/PRs)
- Target platform: **Linux only** (for now). Prefer Linux-native integrations (e.g. inotify for file watching, Secret Service/libsecret for credentials, XDG base directories for config/data/cache paths) over cross-platform abstractions when they conflict.
- License: GPL-3.0. Only add dependencies with GPL-3.0-compatible licenses (e.g. MIT, Apache-2.0, BSD, MPL-2.0).

Core requirements:

- **Full Google Drive coverage**: sync both My Drive (personal) and Shared Drives.
- **Multiple Google accounts**: each account is synced into its own subfolder under the root sync folder (e.g. `<root>/<account>/My Drive`, `<root>/<account>/Shared drives/<name>`).
- **GUI** for configuration and monitoring: add/remove accounts, view per-account status (storage quota/available space, sync state, errors, activity), and other monitoring/sync controls.
- **CLI** exposing the same capabilities as the GUI (account management, status, sync control), suitable for headless use and scripting.
- **Internationalization (i18n)**: initial languages are English, Portuguese (pt-BR), and Spanish. No user-facing string may be hardcoded; all go through the i18n layer. English is the fallback locale.

## Conventions

- All code, identifiers, comments, commit messages, and project documentation (README, CONTRIBUTING, docs/, this file) must be written in **English** to ease open-source community contribution. Only translation resource files contain other languages.
- Architecture follows **Clean Architecture + Hexagonal (Ports & Adapters)**. See below; respect the dependency rule.

## Architecture (intended)

Dependency rule: dependencies point inward only. The domain and application layers must not depend on Google APIs, the filesystem, GUI/CLI frameworks, or any I/O crate.

- **Domain**: core entities and rules (accounts, drives, files/folders, sync state, change sets, conflict resolution policy). Pure Rust, no I/O.
- **Application**: use cases (add/remove account, start/pause sync, get account status, resolve conflicts) orchestrating the domain through **ports** (traits), e.g. remote drive storage, local filesystem, credential/token storage, sync state persistence, filesystem change notifications, clock.
- **Adapters (driven/outbound)**: implementations of ports: Google Drive API client (OAuth2, changes feed, Shared Drives), local filesystem + watcher, secure credential storage (OS keyring), local database for sync metadata.
- **Adapters (driving/inbound)**: GUI and CLI. Both are thin layers that call application use cases; neither contains business logic. Both share the same i18n resources.
- **Composition root**: binaries wire concrete adapters into the use cases.

A Cargo workspace with one crate per layer/adapter is the expected layout, so the compiler enforces the dependency rule (inner crates must not list outer crates as dependencies).

## Commands

Standard Cargo workflow (verify once the workspace exists):

```bash
cargo build                         # build all workspace crates
cargo test                          # run all tests
cargo test -p <crate> <test_name>   # run a single test in a specific crate
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --all
```
