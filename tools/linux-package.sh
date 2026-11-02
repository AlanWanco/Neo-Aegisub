#!/usr/bin/env bash
# Build a self-contained Linux binary tarball from a Meson install tree.
#
# The archive unpacks to a single directory containing the host binary, the GPUI
# front-end and the shared libraries they need, so it runs without installing.
set -euo pipefail
SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"

BUILD_DIR="${1:?usage: linux-package.sh <build-dir> <arch> [output-dir]}"
ARCH="${2:?usage: linux-package.sh <build-dir> <arch> [output-dir]}"
OUTPUT_DIR="${3:-${BUILD_DIR}}"

VERSION="$(sed -n 's/^#define BUILD_GIT_VERSION_STRING "\(.*\)"$/\1/p' "${BUILD_DIR}/git_version.h")"
if [ -z "${VERSION}" ]; then
  echo "Could not read the build version from ${BUILD_DIR}/git_version.h" >&2
  exit 1
fi

# Discover the configured prefix before installing so the install tree path is known.
PREFIX="$(python3 - "${BUILD_DIR}/meson-info/intro-buildoptions.json" <<'PY'
import json, sys
options = json.load(open(sys.argv[1]))
print(next(o["value"] for o in options if o["name"] == "prefix"))
PY
)"

STAGE="$(mktemp -d)"
trap 'rm -rf "${STAGE}"' EXIT

ROOT_NAME="Neo-Aegisub-${VERSION}-${ARCH}"
APP_DIR="${STAGE}/${ROOT_NAME}"
mkdir -p "${APP_DIR}"

DESTDIR="${STAGE}/install" meson install -C "${BUILD_DIR}" --no-rebuild --skip-subprojects >/dev/null
INSTALL_ROOT="${STAGE}/install${PREFIX}"

if [ ! -x "${INSTALL_ROOT}/bin/aegisub" ]; then
  echo "Host binary missing from install tree: ${INSTALL_ROOT}/bin/aegisub" >&2
  exit 1
fi
cp -a "${INSTALL_ROOT}/bin/." "${APP_DIR}/"
python3 "${SCRIPT_DIR}/verify-gpui-package.py" "${APP_DIR}" --arch "$([ "${ARCH}" = linux-aarch64 ] && echo arm64 || echo x86_64)"

if [ ! -x "${APP_DIR}/aegisub-gpui-prototype" ]; then
  echo "GPUI front-end missing from the install tree" >&2
  exit 1
fi

# Ship the resources the host reads at runtime.
for dir in share/locale share/aegisub; do
  if [ -d "${INSTALL_ROOT}/${dir}" ]; then
    mkdir -p "${APP_DIR}/share"
    cp -a "${INSTALL_ROOT}/${dir}" "${APP_DIR}/${dir}"
  fi
done
if [ -d "${APP_DIR}/share/locale" ]; then
  mkdir -p "${APP_DIR}/share/aegisub"
  ln -s ../locale "${APP_DIR}/share/aegisub/locale"
fi

# Bundle non-system shared libraries so the tarball runs on a clean machine.
if command -v ldd >/dev/null 2>&1; then
  mkdir -p "${APP_DIR}/lib"
  copy_libs() {
    ldd "$1" 2>/dev/null | awk '/=> \//{print $3}' | while read -r lib; do
      case "$(basename "${lib}")" in
        libc.so.*|libm.so.*|libdl.so.*|libpthread.so.*|librt.so.*|ld-linux*|libgcc_s.so.*|libstdc++.so.*) continue ;;
      esac
      if [ ! -e "${APP_DIR}/lib/$(basename "${lib}")" ]; then
        cp -L "${lib}" "${APP_DIR}/lib/"
      fi
    done
  }
  copy_libs "${APP_DIR}/aegisub"
  copy_libs "${APP_DIR}/aegisub-gpui-prototype"
fi

# Launch from the unpacked directory, preserving the installed resource layout.
mv "${APP_DIR}/aegisub" "${APP_DIR}/aegisub-core"
cat > "${APP_DIR}/aegisub" <<'LAUNCHER'
#!/bin/sh
APP_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
export LD_LIBRARY_PATH="${APP_DIR}/lib${LD_LIBRARY_PATH:+:${LD_LIBRARY_PATH}}"
export AEGISUB_DATA_PATH="${APP_DIR}/share/aegisub"
exec "${APP_DIR}/aegisub-core" "$@"
LAUNCHER
chmod +x "${APP_DIR}/aegisub"
cat > "${APP_DIR}/README.txt" <<EOF
Neo Aegisub ${VERSION} (${ARCH}) - GPUI portable build

Run ./aegisub to open the GPUI workspace directly.
The launcher configures the bundled libraries and automation resources.
Requires a graphical Linux session compatible with Ubuntu 24.04 or newer.
EOF

mkdir -p "${OUTPUT_DIR}"
ARCHIVE="${OUTPUT_DIR}/Neo-Aegisub-${VERSION}-${ARCH}.tar.gz"
tar -C "${STAGE}" -czf "${ARCHIVE}" "${ROOT_NAME}"
echo "Created ${ARCHIVE}"
