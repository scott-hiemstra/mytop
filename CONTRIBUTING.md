# Contributing

Bug reports, hardware reports (especially AMD and Intel xe GPUs) and pull
requests are welcome.

## Reporting a problem

Please include:

- distribution and kernel version (`uname -r`)
- GPU model(s) and driver (`ls -l /sys/class/drm/card*/device/driver`)
- the output of `mytop --bench`
- for missing or wrong GPU values, the relevant sysfs files, e.g.
  `ls /sys/class/drm/card1/device/` and `ls /sys/class/drm/card1/device/hwmon/*/`

## Development

```sh
cargo build --release
cargo clippy --release
cargo fmt
./target/release/mytop --bench
```

Keep the program dependency-light and free of external commands: read
`/proc`, `/sys` or a vendor library directly. See
[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for the code layout.

## Releasing

Releases are built and published by GitHub Actions
(`.github/workflows/release.yml`) when a `v*` tag is pushed.

1. Update `version` in `Cargo.toml` and `docs/mytop.1`, add a section to
   `CHANGELOG.md`, and run `cargo build` so `Cargo.lock` is updated.
2. Commit, then tag and push:

   ```sh
   git tag -a v0.2.0 -m "mytop 0.2.0"
   git push origin main v0.2.0
   ```

3. The workflow:
   - builds the binary in a **Rocky Linux 8** container (glibc 2.28), so one
     binary runs on RHEL 8+, Debian 10+, Ubuntu 20.04+ and other glibc
     distributions;
   - packages it as a `.deb` (cargo-deb), an `.rpm` (cargo-generate-rpm) and
     a `.tar.gz`;
   - builds a native **Arch Linux** package from source with `makepkg` in an
     `archlinux` container, using `packaging/arch/PKGBUILD`;
   - creates the GitHub release with all packages, the filled-in `PKGBUILD`
     and `SHA256SUMS`.

The same scripts run locally with Docker to test a release before tagging:

```sh
git archive --prefix=mytop-0.2.0/ -o mytop-0.2.0.tar.gz HEAD
docker run --rm -v "$PWD":/src -w /src rockylinux/rockylinux:8 bash packaging/build-glibc.sh
docker run --rm -v "$PWD":/src -w /src archlinux:latest bash packaging/arch/build.sh
ls dist/
```
