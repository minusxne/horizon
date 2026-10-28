#!/usr/bin/env bash
# Packages built Skyline programs for a release.
#
#   tools/package_skyline.sh windows VERSION OUT   # OUT/skyline-windows-x64.zip
#   tools/package_skyline.sh deb     VERSION OUT   # OUT/skyline_VERSION_amd64.deb
#
# Expects `cargo build --release -p horizon-client -p horizon-prepare -p horizon-server` (target/release)
# and `cargo build --release` in launcher/ (launcher/target/release) to have run. The release folder holds
# the programs, the sources skyline-prepare needs to build its exporters (as `cargo xtask package` takes them),
# PLAY.txt, README.md and LICENSE. No Mojang files.
set -euo pipefail

kind=${1:?windows or deb}
version=${2:?version}
out=$(mkdir -p "${3:?out folder}" && cd "$3" && pwd)
repo=$(cd "$(dirname "$0")/.." && pwd)
cd "$repo"

exe=""
[[ "$kind" == windows ]] && exe=".exe"
programs=(skyline skyline-ctl skyline-prepare skyline-server)

stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT

# The release folder: programs plus the sources skyline-prepare compiles its exporters from.
folder() {
    local dir=$1
    mkdir -p "$dir"
    git archive --format=tar HEAD tools mods/horizon/src/test/java crates/horizon-prepare research-lock.json LICENSE \
        README.md | tar -x -C "$dir"
    cp packaging/PLAY.txt "$dir/PLAY.txt"
    for p in "${programs[@]}"; do
        cp "target/release/$p$exe" "$dir/"
    done
    cp "launcher/target/release/skyline-launcher$exe" "$dir/"
}

case "$kind" in
windows)
    dir="$stage/skyline-windows-x64"
    folder "$dir"
    rm -f "$out/skyline-windows-x64.zip"
    if command -v 7z > /dev/null; then
        (cd "$stage" && 7z a -tzip -bd "$out/skyline-windows-x64.zip" skyline-windows-x64 > /dev/null)
    else
        (cd "$stage" && zip -qr "$out/skyline-windows-x64.zip" skyline-windows-x64)
    fi
    echo "$out/skyline-windows-x64.zip"
    ;;
deb)
    # Debian versions must start with a digit.
    debver=${version#v}
    [[ "$debver" =~ ^[0-9] ]] || debver="0.0.0+$debver"
    root="$stage/root"
    folder "$root/usr/lib/skyline"
    chmod 755 "$root/usr/lib/skyline/"skyline*
    mkdir -p "$root/usr/bin" "$root/usr/share/applications" "$root/usr/share/doc/skyline"
    for p in "${programs[@]}" skyline-launcher; do
        ln -s "../lib/skyline/$p" "$root/usr/bin/$p"
    done
    cp packaging/linux/*.desktop "$root/usr/share/applications/"
    mkdir -p "$root/usr/share/icons"
    cp -R packaging/icons/hicolor "$root/usr/share/icons/"
    cp LICENSE "$root/usr/share/doc/skyline/copyright"
    size=$(du -sk "$root" | cut -f1)
    mkdir -p "$root/DEBIAN"
    cat > "$root/DEBIAN/control" << EOF
Package: skyline
Version: $debver
Section: games
Priority: optional
Architecture: amd64
Maintainer: minusxne <74992183+minusxne@users.noreply.github.com>
Installed-Size: $size
Depends: libc6, libasound2t64 | libasound2, libudev1, libxkbcommon0, libwayland-client0, libvulkan1, libfontconfig1
Recommends: default-jre-headless | java-runtime-headless
Homepage: https://github.com/minusxne/horizon
Description: Minecraft: Java Edition 26.3, rewritten in Rust, as an offline game
 Skyline reads the same worlds and generates the same terrain as the Java game,
 but never goes online. It ships no Mojang files: skyline-prepare builds the
 resources from your own copy of Minecraft 26.3. Start skyline-launcher, then
 Versions > Add from folder > /usr/lib/skyline.
EOF
    chmod -R u+rwX,go+rX,go-w "$root"
    deb="$out/skyline_${debver}_amd64.deb"
    dpkg-deb --root-owner-group --build "$root" "$deb" > /dev/null
    echo "$deb"
    ;;
*)
    echo "unknown kind: $kind" >&2
    exit 2
    ;;
esac
