#!/usr/bin/env bash
# Build the portable x86_64 binary and the .deb, .rpm and .tar.gz packages into dist/.
#
# Meant to run inside a Rocky Linux 8 container (glibc 2.28) so the binary
# works on RHEL 8+, Debian 10+, Ubuntu 20.04+ and other glibc distributions:
#
#   docker run --rm -v "$PWD":/src -w /src rockylinux/rockylinux:8 bash packaging/build-glibc.sh
#
# The source is copied to a scratch directory for the build so the host's
# target/ directory is left alone; only dist/ is written back.
set -euo pipefail

SRC=$(cd "$(dirname "$0")/.." && pwd)
OWNER=$(stat -c %u:%g "$SRC")
BUILD=/tmp/mytop-build

echo "==> installing build tools"
dnf install -y -q gcc tar gzip findutils >/dev/null
if ! command -v cargo >/dev/null; then
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs |
        sh -s -- -y -q --profile minimal --default-toolchain stable
fi
# shellcheck disable=SC1091
source "$HOME/.cargo/env"
rustc --version

echo "==> copying source"
rm -rf "$BUILD"
mkdir -p "$BUILD"
tar -C "$SRC" --exclude=./target --exclude=./dist --exclude=./.git -cf - . | tar -C "$BUILD" -xf -
cd "$BUILD"

VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
ARCH=$(uname -m)

echo "==> building mytop $VERSION"
cargo build --release --locked
echo "glibc symbols required: $(objdump -T target/release/mytop 2>/dev/null | grep -o 'GLIBC_[0-9.]*' | sort -Vu | tail -1 || true)"

echo "==> installing packagers"
cargo install --locked --quiet cargo-deb cargo-generate-rpm

mkdir -p dist
echo "==> .deb"
cargo deb --no-build --no-strip --output dist/
echo "==> .rpm"
cargo generate-rpm --output dist/
echo "==> .tar.gz"
T="mytop-$VERSION"
mkdir -p "dist/$T"
cp target/release/mytop docs/mytop.1 README.md CHANGELOG.md LICENSE "dist/$T/"
tar -C dist -czf "dist/$T-$ARCH-linux-gnu.tar.gz" "$T"
rm -rf "dist/$T"

mkdir -p "$SRC/dist"
cp dist/* "$SRC/dist/"
chown -R "$OWNER" "$SRC/dist"
ls -l "$SRC/dist"
