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
| `adapter-localfs` | `LocalFileSystem` (downloads go to `.<name>.nextgdrive-part`, fsync, then rename into place), the inotify watcher, and `DiskStaging` (`StagingArea` for streamed writes under `~/.local/share/nextgdrive/staging/<account id>/`, `<key>.data` + `<key>.json`). | `domain`, `application` |
| `adapter-sqlite` | `SyncStateStore` in `~/.local/share/nextgdrive/state.sqlite3` (rusqlite, bundled SQLite). | `domain`, `application` |
| `adapter-google` | `Authenticator` (OAuth 2.0 for installed apps: PKCE + loopback server on `127.0.0.1:<random port>`) and `DriveService` (Drive API v3). | `domain`, `application` |
| `adapter-keyring` | `CredentialStore` in the Secret Service keyring via `oo7`. Only refresh tokens are stored, never on disk. | `domain`, `application` |
| `i18n` | Fluent translations shared by GUI and CLI; maps `AppError`/`DomainError` to messages; formats sizes and quotas. | `domain`, `application` |
| `adapter-cache` | `BlockCache` on disk under XDG cache (`~/.cache/nextgdrive/blocks/<file id>/<version>/<index>`); evicts least recently used blocks past its limit. | `domain`, `application` |
| `adapter-fuse` | FUSE filesystem (`fuser`, pure Rust: needs only `fusermount3`, not libfuse) for streaming; defines the `StreamBackend` it calls, answers each request in a Tokio task (writes are answered inline, keeping their order). | `domain`, `application` |
| `adapter-dbus` | Session-bus service `io.github.diegofaraujo.NextGDrive.Sync` (zbus) exposed by whichever process runs background sync, and the `Client` the CLI uses. Owning the bus name means "I run background sync". | — |
| `composition` | Shared composition root: builds the concrete adapters (`Services`), localizes setup errors, and `host::SyncHost` (background-sync loop with status, sync-now/pause/resume, served on D-Bus). | everything except front-ends |
| `cli` | Binary `nextgdrive` (clap). | `composition`, inner layers |
| `gui` | Binary `nextgdrive-gui` (GTK4 + libadwaita, app id `io.github.diegofaraujo.NextGDrive`): window plus system tray icon (`ksni`, StatusNotifierItem). | `composition`, inner layers |

Patterns to follow:

- Use cases are small structs generic over their ports (`ListAccounts::new(&repo).execute()`), not trait objects. Unit tests use in-memory fakes from `application/src/use_cases/fakes.rs`.
- Async ports declare `fn ... -> impl Future<Output = ...> + Send` so use-case futures can run on a multi-threaded runtime; implementations just write `async fn`. Local, cheap ports (`AccountRepository`) are synchronous.
- Adapters wrap their failures in the opaque `PortError`. Use cases map each port's errors to a distinct `AppError` variant (`Storage`, `Auth`, `Credentials`, `Drive`), which front-ends localize.
- Tokens (`AccessToken`, `RefreshToken`) redact themselves in `Debug`; read them only through `.secret()` at the point of use.
- Sign-in is two-step (`AddAccount::start` → `pending.url()` → `AddAccount::finish`) so each front-end opens the URL its own way (CLI: print + `xdg-open`; GUI: `gtk::UriLauncher`).
- GUI threading: GTK stays on the main thread; use cases run on the Tokio runtime from `ctx::runtime()`, and results come back through `ctx::spawn`. Sync progress crosses threads over a `tokio::sync::mpsc` channel. Never block the main thread on network or keyring calls.
- GUI ownership (see `gui/src/ctx.rs`): each page's state is an `Rc` kept alive by its root `NavigationPage` via `keep_alive`; the state may hold the root's children but never the root, and closures stored in widgets the state owns hold only `Weak`. Breaking this either frees the page while visible (dead buttons) or leaks it.
- GUI and CLI contain no business logic; they call use cases and render results.

### Sync engine (`application/src/use_cases/sync/`)

Two-way. `sync/plan.rs` is a pure function from (Drive tree, local scan, recorded base) to operations; `sync/apply.rs` runs them; `sync/remote.rs` lists Drive, `sync/local.rs` scans the disk. Invariants that `sync/tests.rs` enforces (each was mutation-checked); keep them when changing the engine:

- Nothing is deleted permanently in Drive. Local deletions **trash** items; items others own are only removed from My Drive (`Capabilities::remove_from_my_drive`); items the user may not delete are restored locally and reported (`ConflictReason::CannotDelete`). Deleting a Google Doc's `.html` shortcut trashes the document, as the official client does with `.gdoc`.
- A change is never lost. Changed on both sides → Drive's version keeps the name, the local one becomes `name (<conflict label>).ext` and is uploaded next to it. Deleted locally but changed in Drive → Drive wins (downloaded again). Edited locally but deleted in Drive → uploaded as new. A folder deleted locally is trashed as one item only if everything in it is gone and nothing in it changed or was added in Drive.
- The full remote tree is listed and the local tree scanned **before** any change, so an API failure aborts instead of looking like mass deletion.
- Guards: if a selected folder that was synced before is missing locally (unmounted disk), sync stops with `AppError::LocalFolderMissing`; if more than `MASS_DELETION_MIN` files and over half of the tracked ones were deleted locally, it stops with `AppError::MassDeletion` unless `SyncOptions::deletions` is `Trash` (confirmed) or `Restore`.
- Local moves/renames are recognized by inode (`LocalMetadata::inode`, unchanged size+mtime) and become moves in Drive, keeping history and sharing. Folder moves on either side are resolved first and rewrite every path below them.
- Ignored names (`local_name::is_ignored`: editor lock/swap files, partial downloads, …) are never uploaded; only content inside selected folders is.
- Downloads are verified against Drive's MD5 before replacing anything; local mtimes are set to Drive's `modifiedTime`, and uploads send the local mtime as `modifiedTime`.
- Each `SyncedItem` remembers the selected folder it was synced under. Deselecting (`ChangeSelection::deselect`) makes the user choose `LocalCopies::Keep` or `LocalCopies::Remove` (CLI `--keep-local`/`--remove-local`, GUI dialog); `use_cases/local_copies.rs` is the single place that decides whether a local copy may be removed.
- `SetSyncRoot` moves the folders of accounts synced from this state to the new root, and never touches folders it did not sync.
- Local layout: `<sync root>/<email>/<My Drive folder>/…` and `<sync root>/<email>/<Shared drives folder>/<drive>/…`; the two folder names are translated once and pinned in `settings.toml`. Google Docs become `.html` redirect pages (`.desktop` links do not open from COSMIC Files or recent Nautilus).

Process model: exactly one process per session runs background sync, by hosting `SyncHost` and owning the D-Bus name. Normally that is the GUI: it starts `SyncHost`, shows the tray icon, hides the window on close (`--background` starts hidden; the XDG autostart entry toggled in Preferences uses it), and only **Quit** in the tray stops syncing. On headless machines `nextgdrive daemon` hosts it instead. `nextgdrive status` and `nextgdrive sync run` (without an email) talk to the host over D-Bus when one runs, and otherwise work standalone.

Modes (`SyncMode`, per account in `settings.toml`): **Mirror** (selected folders copied, background sync below) or **Stream** (the whole Drive mounted at the account folder). The host reconciles mounts at the start of every loop round, *before* setting up file watchers: the watcher covers only mirrored account folders, never the sync root (a recursive inotify watch would walk a mounted Drive through FUSE, and since mounts answer on the host's runtime that can hang it — this happened). `SetSyncMode` switches: Mirror→Stream removes unchanged mirrored copies and moves anything that exists only here to `<email> (<kept files label>)`, so the folder is empty for the mount; Stream→Mirror refuses with `AppError::PendingUploads` while the account's staging area is not empty. After switching, front-ends call `sync_now` over D-Bus so the host mounts/unmounts right away. Unmounting runs `fusermount3 -u -z` on a blocking thread.

If a FUSE mount ever hangs (processes stuck, `ls` freezes), abort its connection: find the mount's minor in `/proc/self/mountinfo` (or the one with no mount left), then `echo 1 > /sys/fs/fuse/connections/<n>/abort`. A process stuck that way keeps its D-Bus name until then.

Background sync (`application/src/background.rs`): `BackgroundSync::tick` reads Drive's change feed per account and runs `SyncAccount` when something changed remotely, the file watcher (inotify, `adapter-localfs::watch`) saw local changes, the selection changed, or `FULL_SYNC_INTERVAL` (15 min) passed (a safety net for changes made while the app was not running). The feed cursor is saved only after a successful sync. `SyncAccount` and `ChangeSelection::deselect` hold a per-account lock (`SyncStateStore::try_lock`; a `flock`ed file next to the SQLite DB), so the tray app, `nextgdrive daemon` and one-off CLI runs never sync the same account at once.

To exercise the engine against a real account without touching the real setup, isolate everything except the keyring, and write the sync root into the isolated `settings.toml` **before the first command**. Do not use `config sync-root` for this: it moves the account folders of accounts synced from that state, and the default root (`~/GoogleDrive`) is shared with the real setup since `HOME` is. Keep test writes inside a dedicated Drive folder (e.g. `Work/NextGDrive-Testes` on the test account), and trash it at the end.

```bash
T=$(mktemp -d); mkdir -p $T/config/nextgdrive
cp ~/.config/nextgdrive/{accounts.toml,oauth-client.json} $T/config/nextgdrive/
printf 'sync_root = "%s/GoogleDrive"\n' $T > $T/config/nextgdrive/settings.toml
export XDG_CONFIG_HOME=$T/config XDG_DATA_HOME=$T/data
nextgdrive sync select <email> <folder-id> && nextgdrive sync run
```

Two such environments syncing the same folder act as two computers, which is how upload, move, conflict and trash behavior is checked end to end.

Trash (`use_cases/trash.rs`): `DriveTrash` lists only *explicitly* trashed items (a trashed folder is one entry; restoring it brings back its content) and restores them in place. Drive reports `trashedTime` only for shared-drive items, so `TrashEntry::trashed_at` is optional and My Drive items keep Drive's `recency` order.

Streaming (`application/src/streaming.rs`): an account's whole Drive as a virtual tree, content fetched only when read. `StreamState` (one per mount) keeps a node table with ids stable across relistings (keyed by parent + Drive id), folder listings cached for `listing_ttl`, and where each file's last read ended; `StreamingDrive` answers attr/list/lookup/read. Content is fetched in `block_size` blocks keyed by Drive's MD5 (so a new version never gets stale blocks); a sequential read fetches up to `readahead_blocks` in one ranged request, a random one (thumbnailers) just one block. Try it with `nextgdrive stream <email> <folder>` (foreground, Ctrl+C unmounts).

Streaming writes (`streaming/write.rs`) follow sync's rules: content goes to the `StagingArea` and stays there until it is in Drive (failed uploads back off `retry_backoff`, then retry; `recover()` picks staged files up after a restart). Edits of existing files upload on close (`flush`); **new files upload only once unchanged for `new_file_quiet`**, because programs close files more than once while creating them and save through temporary files renamed over the original (`sed -i`, gedit) — renaming a never-uploaded file over an existing one uploads it as a new version of that file, keeping its id, history and sharing. If the file changed in Drive since editing began, the edit becomes a conflict copy. Deleting trashes (or removes from My Drive). Ignored names are never uploaded. A file deleted while its upload is in flight has the just-uploaded copy trashed (`tests.rs` reproduces the race by making the fake Drive yield mid-upload).

Locking rule for `StreamState::table()` (a `std::sync::Mutex`): never call it in an `if let`/`match` condition whose body locks again. Temporaries in those conditions live until the end of the block, so the second lock deadlocks (this happened). Take the value into a `let` first.

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
cargo run -p nextgdrive-cli -- account list   # run the CLI: daemon, status, account add|status|remove, config show|sync-root, config mode, sync run|folders|select|deselect, trash list|restore, stream
cargo run -p nextgdrive-gui                   # run the GUI
```

Set `XDG_CONFIG_HOME=$(mktemp -d)` to run against a throwaway config instead of `~/.config/nextgdrive` (copy `oauth-client.json` into it if you need sign-in).

When checking the GUI headlessly with GTK's Broadway backend, libadwaita layouts render squeezed to minimum width; that is a Broadway artifact. Verify layout on a real desktop session.
