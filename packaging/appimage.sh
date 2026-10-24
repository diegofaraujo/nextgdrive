#!/bin/sh
# Builds dist/NextGDrive-<version>-<arch>.AppImage from the binaries in
# target/release, bundling GTK 4 and libadwaita with linuxdeploy.
#   packaging/appimage.sh <version> <x86_64|aarch64> <tools dir>
# The tools dir holds linuxdeploy-<arch>.AppImage,
# linuxdeploy-plugin-appimage-<arch>.AppImage and linuxdeploy-plugin-gtk.sh
# (versions and checksums pinned in .github/workflows/release.yml).
set -eu

VERSION=$1
ARCH=$2
TOOLS=$(realpath "$3")
APP_ID=io.github.diegofaraujo.NextGDrive
APPDIR=target/AppDir

rm -rf "${APPDIR}"
install -Dm755 target/release/nextgdrive-gui "${APPDIR}/usr/bin/nextgdrive-gui"
install -Dm755 target/release/nextgdrive "${APPDIR}/usr/bin/nextgdrive"
install -Dm644 "data/${APP_ID}.metainfo.xml" "${APPDIR}/usr/share/metainfo/${APP_ID}.appdata.xml"
for icon in data/icons/hicolor/*/apps/"${APP_ID}".png; do
    install -Dm644 "${icon}" "${APPDIR}/usr/share/${icon#data/}"
done

mkdir -p dist
# Run the tools without FUSE (CI containers have none).
export APPIMAGE_EXTRACT_AND_RUN=1
export DEPLOY_GTK_VERSION=4
export LINUXDEPLOY_OUTPUT_VERSION="${VERSION}"
export LDAI_OUTPUT="dist/NextGDrive-${VERSION}-${ARCH}.AppImage"
export PATH="${TOOLS}:${PATH}"
"${TOOLS}/linuxdeploy-${ARCH}.AppImage" \
    --appdir "${APPDIR}" \
    --executable "${APPDIR}/usr/bin/nextgdrive-gui" \
    --executable "${APPDIR}/usr/bin/nextgdrive" \
    --desktop-file "data/${APP_ID}.desktop" \
    --icon-file "data/icons/hicolor/256x256/apps/${APP_ID}.png" \
    --plugin gtk \
    --output appimage
