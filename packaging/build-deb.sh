#!/usr/bin/env bash
# Build a Debian package from an already compiled binary.
#
# Usage: packaging/build-deb.sh VERSION DEB_ARCH BINARY OUT_DIR
#   VERSION   package version, e.g. 0.0.3
#   DEB_ARCH  Debian architecture: amd64, arm64, armhf...
#   BINARY    compiled sdr-universal executable (already stripped)
#   OUT_DIR   directory that receives sdr-universal_VERSION_ARCH.deb
#
# Environment: DEB_MAINTAINER overrides the Maintainer field.
set -euo pipefail
umask 022

if [ "$#" -ne 4 ]; then
    echo "usage: $0 VERSION DEB_ARCH BINARY OUT_DIR" >&2
    exit 2
fi

VERSION="$1"
ARCH="$2"
BIN="$3"
OUT="$4"

MAINTAINER="${DEB_MAINTAINER:-pbranly <pbranly@users.noreply.github.com>}"
HOMEPAGE="https://github.com/pbranly/sdr-universal"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

[[ "$VERSION" =~ ^[0-9][A-Za-z0-9.+~-]*$ ]] || { echo "invalid version: $VERSION" >&2; exit 2; }
[ -x "$BIN" ] || { echo "binary not found or not executable: $BIN" >&2; exit 2; }
for f in README.md LICENSE packaging/sdr-universal.1; do
    [ -f "$ROOT/$f" ] || { echo "missing file: $f" >&2; exit 2; }
done

STAGE="$OUT/stage-$ARCH"
DEB="$OUT/sdr-universal_${VERSION}_${ARCH}.deb"
DOC="$STAGE/usr/share/doc/sdr-universal"

rm -rf "$STAGE"
mkdir -p "$STAGE/DEBIAN" "$STAGE/usr/bin" "$DOC" "$STAGE/usr/share/man/man1"

# --- files ---------------------------------------------------------------
install -m 755 "$BIN" "$STAGE/usr/bin/sdr-universal"
install -m 644 "$ROOT/README.md" "$DOC/README.md"
if [ -d "$ROOT/docs" ]; then
    install -m 644 "$ROOT"/docs/*.md "$DOC/"
fi
gzip -9n -c "$ROOT/packaging/sdr-universal.1" > "$STAGE/usr/share/man/man1/sdr-universal.1.gz"
chmod 644 "$STAGE/usr/share/man/man1/sdr-universal.1.gz"

cat > "$DOC/changelog" << CHANGELOG
sdr-universal (${VERSION}) unstable; urgency=medium

  * Release ${VERSION}.
  * Release notes: ${HOMEPAGE}/releases

 -- ${MAINTAINER}  $(date -R -u)
CHANGELOG
gzip -9n "$DOC/changelog"

cat > "$DOC/copyright" << 'COPYRIGHT'
Format: https://www.debian.org/doc/packaging-manuals/copyright-format/1.0/
Upstream-Name: sdr-universal
Source: https://github.com/pbranly/sdr-universal
Comment: The per-band gain tables come from SDRplay's rsp_tcp
 (https://github.com/SDRplay/RSPTCPServer), distributed under the GNU GPL
 version 2 or later.

Files: *
Copyright: COPYRIGHT_YEAR pbranly
License: GPL-3+

License: GPL-3+
 This program is free software: you can redistribute it and/or modify
 it under the terms of the GNU General Public License as published by
 the Free Software Foundation, either version 3 of the License, or
 (at your option) any later version.
 .
 This program is distributed in the hope that it will be useful,
 but WITHOUT ANY WARRANTY; without even the implied warranty of
 MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
 GNU General Public License for more details.
 .
 On Debian systems, the complete text of the GNU General Public
 License version 3 can be found in "/usr/share/common-licenses/GPL-3".
COPYRIGHT
sed -i "s/COPYRIGHT_YEAR/$(date -u +%Y)/" "$DOC/copyright"
chmod 644 "$DOC/copyright"

# --- dependencies: glibc version actually required by the binary ---------
GLIBC_MIN="$(readelf --dyn-syms -W "$BIN" 2>/dev/null | grep -o 'GLIBC_[0-9][0-9.]*' | sed 's/^GLIBC_//' | sort -V | tail -1 || true)"
DEPENDS="libc6${GLIBC_MIN:+ (>= ${GLIBC_MIN})}, libgcc-s1"
echo "Depends: ${DEPENDS}"

INSTALLED_SIZE="$(du -sk --exclude=DEBIAN "$STAGE" | cut -f1)"

cat > "$STAGE/DEBIAN/control" << CONTROL
Package: sdr-universal
Version: ${VERSION}
Section: hamradio
Priority: optional
Architecture: ${ARCH}
Installed-Size: ${INSTALLED_SIZE}
Maintainer: ${MAINTAINER}
Depends: ${DEPENDS}
Homepage: ${HOMEPAGE}
Description: rtl_tcp gateway for SDRplay RSP1B
 Exposes an SDRplay RSP1B as an rtl_tcp server so that software
 written for RTL-SDR (e.g. AbracaDABra) can use the RSP with
 correct per-band gain, RF level and bandwidth selection.
 .
 Requires the SDRplay API 3.x (libsdrplay_api), which is not part of
 this package: install it from https://www.sdrplay.com/software/install.sh
 and make sure the sdrplay_apiService service is running. Without it,
 only the simulated receiver (--mock) can be used.
CONTROL

cat > "$STAGE/DEBIAN/postinst" << 'POSTINST'
#!/bin/sh
set -e

if [ "$1" = "configure" ]; then
    found=no
    for f in /usr/local/lib/libsdrplay_api.so* /usr/lib/libsdrplay_api.so* /usr/lib/*/libsdrplay_api.so*; do
        if [ -e "$f" ]; then
            found=yes
            break
        fi
    done

    if [ "$found" = "no" ]; then
        echo "sdr-universal: warning: the SDRplay API (libsdrplay_api) was not found." >&2
        echo "  Install it with: curl -fsSL https://www.sdrplay.com/software/install.sh | sudo bash" >&2
        echo "  (sdr-universal --mock works without it.)" >&2
    fi
fi

exit 0
POSTINST
chmod 755 "$STAGE/DEBIAN/postinst"

# --- permissions and build ----------------------------------------------
find "$STAGE" -type d -exec chmod 755 {} +

mkdir -p "$OUT"
dpkg-deb --root-owner-group -Zxz --build "$STAGE" "$DEB"
rm -rf "$STAGE"

echo "Built: $DEB"
