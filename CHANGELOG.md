# Changelog

All notable changes are listed here, newest first. The project follows [Semantic Versioning](https://semver.org/); while it is below 1.0, minor versions may change behavior.

Each release is also described in `data/io.github.diegofaraujo.NextGDrive.metainfo.xml` (shown by software centers), with the same version.

## [Unreleased]

### Changed

- Browsing a Drive shown as a folder (streaming mode) no longer waits for Google on every folder. The whole Drive's file and folder list (names, sizes, dates; no content) is kept on this computer and kept up to date from Drive's change feed, so folders open at once, also offline and right after a restart. Changes made in Drive elsewhere show within the check interval.

## [0.1.2] - 2026-10-26

### Added

- A signed Flatpak repository at https://diegofaraujo.github.io/nextgdrive/, so Flatpak installs get updates, and the project's website with its privacy policy and terms of service.

### Fixed

- Closing the window on desktops without a tray (GNOME without the AppIndicator extension, as on Fedora) quit the app and stopped syncing. It now keeps syncing in the background and says so in a notification; launching the app again shows the window, and **Quit** (Ctrl+Q) in its menu stops it. The tray icon appears as soon as the desktop can show it.

## [0.1.1] - 2026-10-24

### Added

- Flatpak: a `.flatpak` bundle for x86_64 and aarch64 with each release, and the groundwork for Flathub. Stream mode, the tray icon, start at login (through the system's Background portal) and the keyring work inside the sandbox.
- The app's own icon in the window, the dock, the tray and a new About dialog.
- Screenshots in the app's description for software centers.

### Fixed

- Start at login from an AppImage pointed to a temporary path; it now starts the AppImage file.
- `nextgdrive daemon` did not show when a streamed folder could not be mounted.

## [0.1.0] - 2026-10-24

First release.

- My Drive and shared drives, for multiple Google accounts, each in its own folder.
- Mirror mode: two-way sync of chosen folders. Deletions go to the Google Drive trash, changes on both sides keep both versions, and deleting many files asks first.
- Stream mode: the whole Drive mounted in the account folder, files downloaded on demand; files and folders available offline; a size-limited cache.
- Google Drive trash: browse and restore, also from the file manager's trash in streamed folders.
- Google Docs, Sheets and Slides as shortcuts that open in the browser.
- Tray app with notifications and start at login; command-line interface with the same features.
- English, Portuguese (Brazil) and Spanish.

[Unreleased]: https://github.com/diegofaraujo/nextgdrive/compare/v0.1.2...HEAD
[0.1.2]: https://github.com/diegofaraujo/nextgdrive/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/diegofaraujo/nextgdrive/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/diegofaraujo/nextgdrive/releases/tag/v0.1.0
