#!/bin/sh
# Compile every catalog in LINGUAS into <build>/po/<lang>/LC_MESSAGES/aegisub.mo.
#
# `i18n.gettext` only does this during `meson install`, which leaves a plain
# build tree without catalogs, so every wx string falls back to English.
# Launchers export AEGISUB_LOCALE_DIR=<build>/po and call `meson compile catalogs`
# so a run from the build tree matches an installed build.
set -eu

BUILD_DIR="${1:?build directory required}"
SOURCE_DIR="${2:?source directory required}"

command -v msgfmt >/dev/null 2>&1 || {
    echo "msgfmt not found; install gettext to compile the catalogs" >&2
    exit 1
}

while read -r language; do
    [ -n "$language" ] || continue
    target="${BUILD_DIR}/${language}/LC_MESSAGES"
    mkdir -p "$target"
    # No -c: a catalog with a format-spec mismatch still has to load, and
    # msgfmt would otherwise refuse to write it at all.
    msgfmt -o "${target}/aegisub.mo" "${SOURCE_DIR}/${language}.po"
done < "${SOURCE_DIR}/LINGUAS"
