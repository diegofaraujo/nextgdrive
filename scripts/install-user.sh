#!/bin/sh
# Installs Next GDrive for the current user only (no root): the programs in
# ~/.local/bin, and the launcher and icons in ~/.local/share, so the app shows
# up in the app menu and dock with its icon. Run from the repository root.
# Uninstall with: scripts/install-user.sh --uninstall
set -eu

APP_ID=io.github.diegofaraujo.NextGDrive
BIN="${HOME}/.local/bin"
DATA="${XDG_DATA_HOME:-${HOME}/.local/share}"

if [ "${1:-}" = "--uninstall" ]; then
    rm -f "${BIN}/nextgdrive" "${BIN}/nextgdrive-gui" "${DATA}/applications/${APP_ID}.desktop"
    rm -f "${DATA}"/icons/hicolor/*/apps/"${APP_ID}".png
else
    cargo build --release --locked
    install -Dm755 target/release/nextgdrive "${BIN}/nextgdrive"
    install -Dm755 target/release/nextgdrive-gui "${BIN}/nextgdrive-gui"
    install -Dm644 "data/${APP_ID}.desktop" "${DATA}/applications/${APP_ID}.desktop"
    for icon in data/icons/hicolor/*/apps/"${APP_ID}".png; do
        install -Dm644 "${icon}" "${DATA}/${icon#data/}"
    done
fi
gtk-update-icon-cache -f -t "${DATA}/icons/hicolor" 2>/dev/null || true
update-desktop-database "${DATA}/applications" 2>/dev/null || true
