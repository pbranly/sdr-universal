# Releasing

Packages are built by GitHub Actions (`.github/workflows/release-deb.yml`). This page is
for maintainers.

## What is built

| Package | Runner | Target |
|---|---|---|
| `sdr-universal_X.Y.Z_amd64.deb` | `ubuntu-24.04` | `x86_64-unknown-linux-gnu` |
| `sdr-universal_X.Y.Z_arm64.deb` | `ubuntu-24.04-arm` | `aarch64-unknown-linux-gnu` |

Each job runs the test suite (against the simulated receiver), builds the release
binary, builds the package with `packaging/build-deb.sh`, lints it with `lintian`
(errors fail the job), then installs it on the runner and runs `--version` and a short
`--mock` session. A `SHA256SUMS` file is published with the packages.

The SDRplay API is **not** linked or shipped: the binary loads `libsdrplay_api.so` at
run time, and the workflow fails if the binary depends on it at link time.

## Publishing a release

1. Make sure `main` is what you want to ship and CI is green.
2. Create and push a tag `vMAJOR.MINOR.PATCH`:

   ```bash
   git tag -a v0.1.0 -m "v0.1.0"
   git push origin v0.1.0
   ```

3. The workflow builds both packages and creates a GitHub Release with generated
   release notes.

The tag is the source of truth for the version: the workflow writes it into
`Cargo.toml` for the build, so `sdr-universal --version`, the package version and the
release all agree. Only plain `vX.Y.Z` tags are accepted; anything else fails the
workflow at its first step. Tags without the `v` prefix do not trigger a release.

## Trial run without a release

**Actions → Release .deb → Run workflow**, leaving the tag empty: the packages are
built and uploaded as workflow artifacts (version `X.Y.Z+ciN`) but no release is
created. Enter an existing tag to (re)publish it instead.

## Building a package locally

```bash
cargo build --release
bash packaging/build-deb.sh 0.1.0 amd64 target/release/sdr-universal target/deb
sudo apt install ./target/deb/sdr-universal_0.1.0_amd64.deb
```

The script computes the required glibc version from the binary (currently 2.34) and
fills the package's `Depends` field with it.

## Not built yet

- **32-bit ARM (`armhf`)**, for 32-bit Raspberry Pi OS. The SDRplay API supports it;
  it needs cross-compilation (`armv7-unknown-linux-gnueabihf`) in the workflow.
- Distributions older than glibc 2.34 (Debian 11, Ubuntu 20.04).
