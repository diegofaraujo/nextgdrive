# Next GDrive

An open-source Google Drive sync client for Linux, written in Rust.

> **Status:** early development. Nothing is usable yet.

## Goals

- **Two-way sync** between a local folder and Google Drive.
- **My Drive and Shared Drives**: both personal and shared drives are supported.
- **Multiple Google accounts**: each account syncs into its own subfolder.
- **Graphical interface** to add and remove accounts, configure sync, and see each account's status (storage usage, sync progress, errors).
- **Command-line interface** with the same features, for headless setups and scripting.
- **Multilingual**: English, Portuguese (Brazil), and Spanish to start.

## Architecture

The project follows Clean Architecture with a Hexagonal (Ports & Adapters) design. The core sync logic does not depend on Google APIs, the filesystem, or the GUI and CLI; those are adapters around it.

## Contributing

Contributions are welcome. Code, comments, commit messages, and documentation are written in English. Translations live in dedicated locale files.

## License

Licensed under the [GNU General Public License v3.0](LICENSE).
