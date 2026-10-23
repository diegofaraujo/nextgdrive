# Next GDrive

An open-source Google Drive sync client for Linux, written in Rust.

> **Status:** early development, not yet ready for important data. Two-way sync of selected folders works; test it on a non-critical account first.

## Goals

- **Two-way sync** between a local folder and Google Drive.
- **My Drive and Shared Drives**: both personal and shared drives are supported.
- **Multiple Google accounts**: each account syncs into its own subfolder.
- **Graphical interface** to add and remove accounts, configure sync, and see each account's status (storage usage, sync progress, errors).
- **Command-line interface** with the same features, for headless setups and scripting.
- **Multilingual**: English, Portuguese (Brazil), and Spanish to start.

## Building from source

Requirements: Rust 1.92 or newer, and the GTK 4 (≥ 4.14) and libadwaita (≥ 1.5) development packages. On Ubuntu or Pop!_OS 24.04:

```bash
sudo apt install libgtk-4-dev libadwaita-1-dev
cargo build --release
```

This produces two programs in `target/release/`: `nextgdrive-gui` (graphical app) and `nextgdrive` (command line).

### Google OAuth client

Next GDrive does not ship Google API credentials yet, so you need your own OAuth client:

1. In [Google Cloud Console](https://console.cloud.google.com/), create a project and enable the **Google Drive API**.
2. Under **Google Auth Platform**, configure the consent screen with audience **External**, and add your Google accounts as **test users**.
3. Under **Clients**, create a client of type **Desktop app** and download its JSON file.
4. Save that file as `~/.config/nextgdrive/oauth-client.json` (or set `NEXTGDRIVE_CLIENT_ID` and `NEXTGDRIVE_CLIENT_SECRET`).

### Usage

```bash
nextgdrive account add       # sign in with Google in your browser
nextgdrive account status    # storage usage of each linked account
nextgdrive account list
nextgdrive account remove <email>
```

```bash
nextgdrive sync folders <email>          # browse drives and folders
nextgdrive sync select <email> <id>      # sync a folder (and everything in it)
nextgdrive sync run                      # sync now
nextgdrive status                        # what the background sync is doing
nextgdrive daemon                        # background sync without the graphical app
```

The graphical app (`nextgdrive-gui`) lives in the system tray and keeps the selected folders in sync in both directions. Files deleted on the computer go to the Google Drive trash (nothing is ever deleted permanently); when a file changes on both sides, both versions are kept.

Refresh tokens are stored in the system keyring (Secret Service, such as GNOME Keyring), never in plain files.

## Architecture

The project follows Clean Architecture with a Hexagonal (Ports & Adapters) design. The core sync logic does not depend on Google APIs, the filesystem, or the GUI and CLI; those are adapters around it.

## Contributing

Contributions are welcome. Code, comments, commit messages, and documentation are written in English. Translations live in dedicated locale files.

## License

Licensed under the [GNU General Public License v3.0](LICENSE) or (at your option) any later version.
