#!/usr/bin/env bash
# Build a Debian/Ubuntu package of OxiNote (app + CLI).
#
#   scripts/build-deb.sh BIN_DIR VERSION [OUT_DIR]
#
# BIN_DIR holds the `oxinote-app` and `oxinote` binaries (for example
# target/x86_64-unknown-linux-gnu/release). VERSION is the release version,
# with or without a leading `v` (`0.1.0`, `v0.1.0-rc.1`). The package lands in
# OUT_DIR (default: current directory) as oxinote_<version>_amd64.deb.
set -euo pipefail

bin_dir=${1:?usage: build-deb.sh BIN_DIR VERSION [OUT_DIR]}
version=${2:?usage: build-deb.sh BIN_DIR VERSION [OUT_DIR]}
out_dir=${3:-.}
root=$(cd "$(dirname "$0")/.." && pwd)

# Debian ordering: a pre-release sorts before the final version with `~`.
version=${version#v}
version=${version/-/\~}
arch=$(dpkg --print-architecture)

stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
pkg="$stage/oxinote"

install -Dm755 "$bin_dir/oxinote-app" "$pkg/usr/bin/oxinote-app"
install -Dm755 "$bin_dir/oxinote" "$pkg/usr/bin/oxinote"
install -Dm644 "$root/assets/linux/oxinote.desktop" \
  "$pkg/usr/share/applications/oxinote.desktop"
for size in 16 32 48 64 128 256 512; do
  install -Dm644 "$root/assets/logo/oxinote-icon-$size.png" \
    "$pkg/usr/share/icons/hicolor/${size}x${size}/apps/oxinote.png"
done
install -Dm644 "$root/README.md" "$pkg/usr/share/doc/oxinote/README.md"
cat > "$pkg/usr/share/doc/oxinote/copyright" <<'EOF'
Format: https://www.debian.org/doc/packaging-manuals/copyright-format/1.0/
Upstream-Name: OxiNote
Source: https://github.com/LeVibeux/OxiNote

Files: *
License: MIT or Apache-2.0

Files: usr/bin/oxinote-app
Comment: embeds the Inter font (SIL Open Font License 1.1) and the
 Phosphor icon font (MIT).
EOF

# Libraries the binaries link against, as computed by dpkg-shlibdeps.
mkdir -p "$stage/debian"
touch "$stage/debian/control"
shlibs=$(cd "$stage" && dpkg-shlibdeps -O \
  "$pkg/usr/bin/oxinote-app" "$pkg/usr/bin/oxinote" 2>/dev/null |
  sed -n 's/^shlibs:Depends=//p')
installed_size=$(du -sk "$pkg" | cut -f1)

mkdir -p "$pkg/DEBIAN"
# The package was called obsirust up to 0.2.x: Conflicts/Replaces make
# installing this one remove it.
cat > "$pkg/DEBIAN/control" <<EOF
Package: oxinote
Version: $version
Architecture: $arch
Maintainer: OxiNote <https://github.com/LeVibeux/OxiNote>
Installed-Size: $installed_size
Depends: ${shlibs:-libc6}, libgl1 | libegl1, libxkbcommon0
Recommends: libxkbcommon-x11-0, libwayland-client0, libx11-6, libxcursor1, libxrandr2, libxi6, xdg-desktop-portal
Suggests: git
Conflicts: obsirust
Replaces: obsirust
Provides: obsirust
Section: editors
Priority: optional
Homepage: https://github.com/LeVibeux/OxiNote
Description: Obsidian-compatible Markdown knowledge base, written in Rust
 OxiNote opens existing Obsidian vaults: Live Preview, Source and Reading
 modes, wikilinks and backlinks, graph view, typed properties and tags,
 search, PDF viewer and a local Git backup. The package also installs the
 oxinote command-line tool.
EOF

mkdir -p "$out_dir"
deb="$out_dir/oxinote_${version}_${arch}.deb"
dpkg-deb --root-owner-group --build "$pkg" "$deb" >/dev/null
echo "$deb"
