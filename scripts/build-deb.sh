#!/usr/bin/env bash
# Build a Debian/Ubuntu package of ObsiRust (app + CLI).
#
#   scripts/build-deb.sh BIN_DIR VERSION [OUT_DIR]
#
# BIN_DIR holds the `obsirust-app` and `obsirust` binaries (for example
# target/x86_64-unknown-linux-gnu/release). VERSION is the release version,
# with or without a leading `v` (`0.1.0`, `v0.1.0-rc.1`). The package lands in
# OUT_DIR (default: current directory) as obsirust_<version>_amd64.deb.
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
pkg="$stage/obsirust"

install -Dm755 "$bin_dir/obsirust-app" "$pkg/usr/bin/obsirust-app"
install -Dm755 "$bin_dir/obsirust" "$pkg/usr/bin/obsirust"
install -Dm644 "$root/assets/linux/obsirust.desktop" \
  "$pkg/usr/share/applications/obsirust.desktop"
for size in 16 32 48 64 128 256 512; do
  install -Dm644 "$root/assets/logo/obsirust-icon-$size.png" \
    "$pkg/usr/share/icons/hicolor/${size}x${size}/apps/obsirust.png"
done
install -Dm644 "$root/README.md" "$pkg/usr/share/doc/obsirust/README.md"
cat > "$pkg/usr/share/doc/obsirust/copyright" <<'EOF'
Format: https://www.debian.org/doc/packaging-manuals/copyright-format/1.0/
Upstream-Name: ObsiRust
Source: https://github.com/LeVibeux/ObsiRust

Files: *
License: MIT or Apache-2.0

Files: usr/bin/obsirust-app
Comment: embeds the Inter font (SIL Open Font License 1.1) and the
 Phosphor icon font (MIT).
EOF

# Libraries the binaries link against, as computed by dpkg-shlibdeps.
mkdir -p "$stage/debian"
touch "$stage/debian/control"
shlibs=$(cd "$stage" && dpkg-shlibdeps -O \
  "$pkg/usr/bin/obsirust-app" "$pkg/usr/bin/obsirust" 2>/dev/null |
  sed -n 's/^shlibs:Depends=//p')
installed_size=$(du -sk "$pkg" | cut -f1)

mkdir -p "$pkg/DEBIAN"
cat > "$pkg/DEBIAN/control" <<EOF
Package: obsirust
Version: $version
Architecture: $arch
Maintainer: ObsiRust <https://github.com/LeVibeux/ObsiRust>
Installed-Size: $installed_size
Depends: ${shlibs:-libc6}, libgl1 | libegl1, libxkbcommon0
Recommends: libxkbcommon-x11-0, libwayland-client0, libx11-6, libxcursor1, libxrandr2, libxi6, xdg-desktop-portal
Suggests: git
Section: editors
Priority: optional
Homepage: https://github.com/LeVibeux/ObsiRust
Description: Obsidian-compatible Markdown knowledge base, written in Rust
 ObsiRust opens existing Obsidian vaults: Live Preview, Source and Reading
 modes, wikilinks and backlinks, graph view, typed properties and tags,
 search, PDF viewer and a local Git backup. The package also installs the
 obsirust command-line tool.
EOF

mkdir -p "$out_dir"
deb="$out_dir/obsirust_${version}_${arch}.deb"
dpkg-deb --root-owner-group --build "$pkg" "$deb" >/dev/null
echo "$deb"
