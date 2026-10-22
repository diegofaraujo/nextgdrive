# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project overview

**Next GDrive** is an open-source Google Drive sync client written in Rust. It keeps a local folder synchronized with Google Drive.

- Repository: https://github.com/diegofaraujo/nextgdrive (GitHub, default branch `main`; use the `gh` CLI for issues/PRs)
- Target platform: **Linux only** (for now). Prefer Linux-native integrations (e.g. inotify for file watching, Secret Service/libsecret for credentials, XDG base directories for config/data/cache paths) over cross-platform abstractions when they conflict.
- License: GPL-3.0-or-later (set once in the root `Cargo.toml` `[workspace.package]`). Only add dependencies with GPL-3.0-compatible licenses (e.g. MIT, Apache-2.0, BSD, MPL-2.0).

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
| `domain` | Entities and rules (`Account`, `Email`, `StorageQuota`, `DriveItem`, `Settings`) and `local_name` (Drive name → valid, unique Linux file name; Google Doc shortcut format). Pure, no I/O. | nothing internal |
| `application` | Use cases (`use_cases/`), the ports they need (`ports/`), and `TokenProvider` (cached access tokens). | `domain` |
| `adapter-config` | `AccountRepository` and `SettingsRepository` as TOML under XDG config (`~/.config/nextgdrive/{accounts,settings}.toml`). | `domain`, `application` |
| `adapter-localfs` | `LocalFileSystem`: downloads go to `.<name>.nextgdrive-part`, fsync, then rename into place. | `domain`, `application` |
| `adapter-sqlite` | `SyncStateStore` in `~/.local/share/nextgdrive/state.sqlite3` (rusqlite, bundled SQLite). | `domain`, `application` |
| `adapter-google` | `Authenticator` (OAuth 2.0 for installed apps: PKCE + loopback server on `127.0.0.1:<random port>`) and `DriveService` (Drive API v3). | `domain`, `application` |
| `adapter-keyring` | `CredentialStore` in the Secret Service keyring via `oo7`. Only refresh tokens are stored, never on disk. | `domain`, `application` |
| `i18n` | Fluent translations shared by GUI and CLI; maps `AppError`/`DomainError` to messages; formats sizes and quotas. | `domain`, `application` |
| `composition` | Shared composition root: builds the concrete adapters (`Services`) and localizes setup errors. | everything except front-ends |
| `cli` | Binary `nextgdrive` (clap). | `composition`, inner layers |
| `gui` | Binary `nextgdrive-gui` (GTK4 + libadwaita, app id `io.github.diegofaraujo.NextGDrive`). | `composition`, inner layers |

Patterns to follow:

- Use cases are small structs generic over their ports (`ListAccounts::new(&repo).execute()`), not trait objects. Unit tests use in-memory fakes from `application/src/use_cases/fakes.rs`.
- Async ports declare `fn ... -> impl Future<Output = ...> + Send` so use-case futures can run on a multi-threaded runtime; implementations just write `async fn`. Local, cheap ports (`AccountRepository`) are synchronous.
- Adapters wrap their failures in the opaque `PortError`. Use cases map each port's errors to a distinct `AppError` variant (`Storage`, `Auth`, `Credentials`, `Drive`), which front-ends localize.
- Tokens (`AccessToken`, `RefreshToken`) redact themselves in `Debug`; read them only through `.secret()` at the point of use.
- Sign-in is two-step (`AddAccount::start` → `pending.url()` → `AddAccount::finish`) so each front-end opens the URL its own way (CLI: print + `xdg-open`; GUI: `gtk::UriLauncher`).
- GUI threading: GTK stays on the main thread; use cases run on the Tokio runtime from `ctx::runtime()`, and results come back through `ctx::spawn`. Sync progress crosses threads over a `tokio::sync::mpsc` channel. Never block the main thread on network or keyring calls.
- GUI ownership (see `gui/src/ctx.rs`): each page's state is an `Rc` kept alive by its root `NavigationPage` via `keep_alive`; the state may hold the root's children but never the root, and closures stored in widgets the state owns hold only `Weak`. Breaking this either frees the page while visible (dead buttons) or leaks it.
- GUI and CLI contain no business logic; they call use cases and render results.

### Sync engine (`application/src/use_cases/sync_account.rs`)

Download-only for now (upload is the next phase). Invariants that tests in that file enforce; keep them when changing it:

- A local file whose size or mtime differs from what was recorded at its last sync is **never overwritten or deleted**; it is reported in `SyncReport::conflicts`. Untracked local files are only adopted if their MD5 matches Drive.
- The full remote tree of every selected folder is listed **before** any local change, so an API failure aborts instead of looking like mass deletion.
- Each `SyncedItem` remembers the selected folder it was synced under. Deselecting (`ChangeSelection::deselect`) always makes the user choose `LocalCopies::Keep` (stop tracking, files stay) or `LocalCopies::Remove` (delete unchanged copies now); the CLI requires `--keep-local`/`--remove-local` and the GUI asks in a dialog. An item that disappears while its folder is still selected was removed on Drive, and its unchanged local copy is deleted. Both paths use `use_cases/local_copies.rs`, the single place that decides whether a local copy may be removed.
- Downloads are verified against Drive's MD5 before replacing anything. Local mtimes are set to Drive's `modifiedTime`.
- Local layout: `<sync root>/<email>/<My Drive folder>/…` and `<sync root>/<email>/<Shared drives folder>/<drive>/…`. The two folder names are translated once and pinned in `settings.toml`.
- Google Docs/Sheets/etc. become small `.html` pages that redirect to the document (`.desktop` links do not open from COSMIC Files or recent Nautilus). Shortcuts are rewritten, never moved, when their name or format changes. Drive shortcuts and non-downloadable files are skipped and reported.

Background sync (`application/src/background.rs`): `BackgroundSync::tick` reads Drive's change feed per account and runs `SyncAccount` when something changed remotely, the selection changed, or `FULL_SYNC_INTERVAL` (15 min) passed (the feed cannot report local deletions). The feed cursor is saved only after a successful sync. `SyncAccount` and `ChangeSelection::deselect` hold a per-account lock (`SyncStateStore::try_lock`; a `flock`ed file next to the SQLite DB), so the tray app, `nextgdrive daemon` and one-off CLI runs never sync the same account at once.

To exercise the engine against a real account without touching the real setup, isolate everything except the keyring:

```bash
T=$(mktemp -d); mkdir -p $T/config/nextgdrive
cp ~/.config/nextgdrive/{accounts.toml,oauth-client.json} $T/config/nextgdrive/
export XDG_CONFIG_HOME=$T/config XDG_DATA_HOME=$T/data
nextgdrive config sync-root $T/GoogleDrive && nextgdrive sync select <email> <folder-id> && nextgdrive sync run
```

### Google OAuth client

The OAuth client (Google Cloud "Desktop app") is not in the repository; each developer uses their own. It is read from `NEXTGDRIVE_CLIENT_ID` + `NEXTGDRIVE_CLIENT_SECRET`, otherwise from the JSON downloaded from Google Cloud Console saved as `~/.config/nextgdrive/oauth-client.json`. The app requests the full `drive` scope (needed for two-way sync). While the Google Cloud app is in testing mode, only accounts listed as test users can sign in.

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
cargo test -p nextgdrive-adapter-keyring -- --ignored   # keyring test against the real session keyring
cargo test -p nextgdrive-application remove   # tests in one crate whose name matches "remove"
cargo clippy --all-targets -- -D warnings     # lint (must be clean)
cargo fmt --all
cargo run -p nextgdrive-cli -- account list   # run the CLI: daemon, account add|status|remove, config show|sync-root, sync run|folders|select|deselect
cargo run -p nextgdrive-gui                   # run the GUI
```

Set `XDG_CONFIG_HOME=$(mktemp -d)` to run against a throwaway config instead of `~/.config/nextgdrive` (copy `oauth-client.json` into it if you need sign-in).

When checking the GUI headlessly with GTK's Broadway backend, libadwaita layouts render squeezed to minimum width; that is a Broadway artifact. Verify layout on a real desktop session.
