#!/usr/bin/env bash
# Build the Arch Linux package with makepkg into dist/.
#
#   docker run --rm -v "$PWD":/src -w /src archlinux:latest bash packaging/arch/build.sh
#
# If mytop-<version>.tar.gz exists in the repository root it is used as the
# source (CI downloads GitHub's tag archive; for a local test create it with
# `git archive --prefix=mytop-<version>/ -o mytop-<version>.tar.gz HEAD`).
# Otherwise makepkg downloads the tag archive from GitHub.
# The PKGBUILD with real checksums is written to dist/ alongside the package.
set -euo pipefail

SRC=$(cd "$(dirname "$0")/../.." && pwd)
OWNER=$(stat -c %u:%g "$SRC")
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' "$SRC/Cargo.toml" | head -1)

echo "==> installing build tools"
pacman -Syu --noconfirm --needed --quiet base-devel pacman-contrib rust >/dev/null

id builder >/dev/null 2>&1 || useradd -m builder
WORK=/home/builder/pkg
rm -rf "$WORK"
mkdir -p "$WORK"
sed "s/^pkgver=.*/pkgver=$VERSION/" "$SRC/packaging/arch/PKGBUILD" >"$WORK/PKGBUILD"
if [[ -f "$SRC/mytop-$VERSION.tar.gz" ]]; then
    cp "$SRC/mytop-$VERSION.tar.gz" "$WORK/"
fi
chown -R builder: "$WORK"

echo "==> makepkg mytop $VERSION"
su builder -c "cd '$WORK' && updpkgsums && makepkg --noconfirm --force"

mkdir -p "$SRC/dist"
cp "$WORK"/mytop-"$VERSION"-*.pkg.tar.zst "$SRC/dist/"
cp "$WORK/PKGBUILD" "$SRC/dist/PKGBUILD"
chown -R "$OWNER" "$SRC/dist"
ls -l "$SRC/dist"
