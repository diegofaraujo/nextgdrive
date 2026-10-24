# Changelog

All notable changes are listed here, newest first. The project follows [Semantic Versioning](https://semver.org/); while it is below 1.0, minor versions may change behavior.

Each release is also described in `data/io.github.diegofaraujo.NextGDrive.metainfo.xml` (shown by software centers), with the same version.

## [Unreleased]

## [0.1.0] - 2026-10-24

First release.

- My Drive and shared drives, for multiple Google accounts, each in its own folder.
- Mirror mode: two-way sync of chosen folders. Deletions go to the Google Drive trash, changes on both sides keep both versions, and deleting many files asks first.
- Stream mode: the whole Drive mounted in the account folder, files downloaded on demand; files and folders available offline; a size-limited cache.
- Google Drive trash: browse and restore, also from the file manager's trash in streamed folders.
- Google Docs, Sheets and Slides as shortcuts that open in the browser.
- Tray app with notifications and start at login; command-line interface with the same features.
- English, Portuguese (Brazil) and Spanish.

[Unreleased]: https://github.com/diegofaraujo/nextgdrive/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/diegofaraujo/nextgdrive/releases/tag/v0.1.0
