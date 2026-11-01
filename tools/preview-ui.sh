#!/bin/sh
# Run a local macOS build with a separate, persistent preview profile.
set -eu

if [ "$(uname -s)" != Darwin ]; then
    echo "This preview launcher currently supports macOS only." >&2
    exit 1
fi

SOURCE_DIR=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
BUILD_DIR=${1:-"${SOURCE_DIR}/build-ui"}
if [ "$#" -gt 0 ]; then shift; fi
BUILD_DIR=$(CDPATH= cd -- "$BUILD_DIR" && pwd)
APP_EXECUTABLE="${BUILD_DIR}/Aegisub.app/Contents/MacOS/aegisub"

if [ ! -x "$APP_EXECUTABLE" ]; then
    echo "Build and bundle Aegisub first; missing: ${APP_EXECUTABLE}" >&2
    exit 1
fi

# `i18n.gettext` only compiles catalogs on install, so a build tree has none
# and every wxWidgets string would fall back to English next to a Chinese
# GPUI workspace. Compile them here and point the app at them.
if command -v meson >/dev/null 2>&1; then
    meson compile -C "$BUILD_DIR" catalogs >/dev/null 2>&1 ||
        echo "warning: could not build the message catalogs; wxWidgets strings stay English" >&2
fi
if [ -d "${BUILD_DIR}/po/zh_CN/LC_MESSAGES" ]; then
    export AEGISUB_LOCALE_DIR="${BUILD_DIR}/po"
fi

PREVIEW_HOME=${AEGISUB_UI_PREVIEW_HOME:-"${BUILD_DIR}/preview-home"}
mkdir -p "$PREVIEW_HOME"
PREVIEW_HOME=$(CDPATH= cd -- "$PREVIEW_HOME" && pwd)

python3 - "$PREVIEW_HOME" <<'PY'
import json
from pathlib import Path
import sys

profile = Path(sys.argv[1]) / "Library/Application Support/Aegisub"
profile.mkdir(parents=True, exist_ok=True)
config = profile / "config.json"
try:
    with config.open("x", encoding="utf-8") as stream:
        json.dump({
            "App": {
                "First Start": False,
                "Language": "en_US",
                "Auto": {"Check For Updates": False},
            }
        }, stream, indent=2)
except FileExistsError:
    pass
PY

# CFFIXED_USER_HOME also redirects the Foundation Application Support lookup.
export CFFIXED_USER_HOME="$PREVIEW_HOME"
exec "$APP_EXECUTABLE" "$@"
