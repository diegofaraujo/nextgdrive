# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

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

## Architecture

Clean Architecture + Hexagonal (Ports & Adapters) in a Cargo workspace with one crate per layer/adapter under `crates/`, so the compiler enforces the dependency rule: dependencies point inward only. `domain` and `application` must never depend on Google APIs, the filesystem, GTK, clap, or any I/O crate.

| Crate | Role | May depend on |
|---|---|---|
| `domain` | Entities and rules (e.g. `Account`, validated `Email`). Pure, no I/O. | nothing internal |
| `application` | Use cases (`use_cases/`) and the ports they need (`ports/`, traits like `AccountRepository`). | `domain` |
| `adapter-*` | Driven adapters implementing ports. `adapter-config`: TOML files under XDG config (`~/.config/nextgdrive/accounts.toml`). | `domain`, `application` |
| `i18n` | Fluent translations shared by GUI and CLI, plus mapping of `AppError`/`DomainError` to localized messages. | `domain`, `application` |
| `cli` | Binary `nextgdrive` (clap). Composition root for the CLI. | everything |
| `gui` | Binary `nextgdrive-gui` (GTK4 + libadwaita, app id `io.github.diegofaraujo.NextGDrive`). Composition root for the GUI. | everything |

Patterns to follow:

- Use cases are small structs generic over their ports (`ListAccounts::new(&repo).execute()`), not trait objects. Unit tests use in-memory fakes from `application/src/use_cases/fakes.rs`.
- Adapters wrap their failures in the opaque `RepositoryError`; the application layer never sees adapter-specific error types.
- GUI and CLI contain no business logic; they call use cases and render results.
- New Google Drive features (OAuth, Drive API, sync engine, file watcher, keyring credentials) go in new `adapter-*` crates behind new ports in `application`.

### i18n

- Translations live in `crates/i18n/locales/{en-US,pt-BR,es}/main.ftl` ([Fluent](https://projectfluent.org/) syntax). `en-US` is the reference and fallback. A test fails if any locale's message ids differ from `en-US`, so add every new message to all three files.
- Locale comes from the system (`LC_ALL`/`LC_MESSAGES`/`LANG`); other variants map by language (`pt-PT` → `pt-BR`, `es-MX` → `es`). Try another language with e.g. `LANG=es_ES.UTF-8 cargo run -p nextgdrive-cli -- --help`.
- The CLI uses clap's **builder** API (not derive) so help texts and section headings come from Fluent at runtime. Clap's own parse errors (e.g. "unrecognized subcommand") are still English-only.
- GUI is built in Rust code, not `.ui` XML, so all text goes through `Localizer`. Set `use_markup(false)` on rows/labels that show user data (emails, file names).

## Commands

Build requirements: Rust ≥ 1.92 (workspace MSRV, required by gtk4-rs 0.11) and GTK ≥ 4.14 / libadwaita ≥ 1.5 dev packages (`sudo apt install libgtk-4-dev libadwaita-1-dev` on Ubuntu/Pop!_OS 24.04).

```bash
cargo build                                   # build everything
cargo test                                    # run all tests
cargo test -p nextgdrive-application remove   # tests in one crate whose name matches "remove"
cargo clippy --all-targets -- -D warnings     # lint (must be clean)
cargo fmt --all
cargo run -p nextgdrive-cli -- account list   # run the CLI
cargo run -p nextgdrive-gui                   # run the GUI
```

Set `XDG_CONFIG_HOME=$(mktemp -d)` to run against a throwaway config instead of `~/.config/nextgdrive`.
