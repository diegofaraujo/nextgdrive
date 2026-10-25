#!/bin/sh
# Builds the Flatpak repository published on GitHub Pages: an OSTree repo
# with the bundles of every architecture, signed, plus the .flatpakrepo and
# .flatpakref files that point to it and an install page.
#   packaging/flatpak-repo.sh <bundles dir> <output dir> <base URL> <GPG key id> [GPG homedir]
# The public key is flatpak/nextgdrive-repo.asc; the private one must be in
# the GPG homedir (CI imports it from the FLATPAK_GPG_PRIVATE_KEY secret).
set -eu

BUNDLES=$1
OUT=$2
URL=${3%/}
KEY=$4
HOMEDIR=${5:-${GNUPGHOME:-${HOME}/.gnupg}}
APP_ID=io.github.diegofaraujo.NextGDrive
BRANCH=master

mkdir -p "${OUT}"
# An empty archive repository, as `ostree init --mode=archive-z2` makes it
# (the ostree tool is not always installed).
mkdir -p "${OUT}/repo/objects" "${OUT}/repo/refs/heads" "${OUT}/repo/refs/mirrors" \
    "${OUT}/repo/refs/remotes" "${OUT}/repo/state" "${OUT}/repo/tmp/cache" "${OUT}/repo/extensions"
printf '[core]\nrepo_version=1\nmode=archive-z2\n' > "${OUT}/repo/config"
for bundle in "${BUNDLES}"/*.flatpak; do
    flatpak build-import-bundle --gpg-sign="${KEY}" --gpg-homedir="${HOMEDIR}" \
        "${OUT}/repo" "${bundle}"
done
flatpak build-update-repo --gpg-sign="${KEY}" --gpg-homedir="${HOMEDIR}" \
    --title="Next GDrive" --default-branch="${BRANCH}" \
    --generate-static-deltas --prune "${OUT}/repo"

# Flatpak wants the key as base64 of its binary form, on one line.
GPG_KEY=$(gpg --dearmor < flatpak/nextgdrive-repo.asc | base64 -w0)
cp data/icons/hicolor/128x128/apps/${APP_ID}.png "${OUT}/icon.png"

cat > "${OUT}/nextgdrive.flatpakrepo" <<REPO
[Flatpak Repo]
Title=Next GDrive
Url=${URL}/repo/
Homepage=https://github.com/diegofaraujo/nextgdrive
Comment=Releases of Next GDrive
Description=Google Drive client for Linux: sync or stream your Google Drive
Icon=${URL}/icon.png
GPGKey=${GPG_KEY}
REPO

cat > "${OUT}/${APP_ID}.flatpakref" <<REF
[Flatpak Ref]
Name=${APP_ID}
Branch=${BRANCH}
Title=Next GDrive
Url=${URL}/repo/
SuggestRemoteName=nextgdrive
Homepage=https://github.com/diegofaraujo/nextgdrive
Icon=${URL}/icon.png
RuntimeRepo=https://dl.flathub.org/repo/flathub.flatpakrepo
IsRuntime=false
GPGKey=${GPG_KEY}
REF

cat > "${OUT}/index.html" <<HTML
<!doctype html>
<html lang="en">
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Next GDrive · Flatpak</title>
<style>
  body { font: 16px/1.5 system-ui, sans-serif; max-width: 42rem; margin: 3rem auto; padding: 0 1rem; }
  pre { background: #f2f2f2; padding: 0.75rem 1rem; overflow-x: auto; border-radius: 6px; }
  @media (prefers-color-scheme: dark) { body { background: #1e1e1e; color: #eee; } pre { background: #2c2c2c; } a { color: #8fd3fe; } }
</style>
<img src="icon.png" alt="" width="96" height="96">
<h1>Next GDrive</h1>
<p>Google Drive client for Linux. This is its Flatpak repository: installing from it keeps the app up to date with <code>flatpak update</code>.</p>
<h2>Install</h2>
<pre>flatpak install --user ${URL}/${APP_ID}.flatpakref</pre>
<p>Or add the repository, then install from it:</p>
<pre>flatpak remote-add --user --if-not-exists nextgdrive ${URL}/nextgdrive.flatpakrepo
flatpak install --user nextgdrive ${APP_ID}</pre>
<p>The GNOME runtime it needs comes from <a href="https://flathub.org">Flathub</a>. Other packages (deb, rpm, AppImage) are on the <a href="https://github.com/diegofaraujo/nextgdrive/releases">releases page</a>.</p>
</html>
HTML
