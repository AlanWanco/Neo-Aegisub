#!/usr/bin/env python3
"""Build the static, PIC dependency layer for libmpv on macOS.

Builds freetype, fribidi, harfbuzz, libass and FFmpeg as static PIC libraries
for one architecture and the macOS deployment target. zlib and iconv come from
the macOS SDK; nothing is taken from Homebrew or other prefixes.

Usage:
    macos-build-mpv-deps.py --work-dir DIR --arch arm64|x86_64 --deployment-target 13.0

Outputs:
    DIR/prefix/lib, DIR/prefix/include, DIR/prefix/lib/pkgconfig
    DIR/prefix/share/licenses/libmpv/<dep>/ (license files + PROVENANCE.txt)
Progress goes to stderr; per-step command output goes to DIR/logs.
Use one work directory per architecture.
"""

import argparse
import dataclasses
import hashlib
import json
import os
import platform
import shutil
import subprocess
import sys
import tarfile
import urllib.request
from pathlib import Path

DEPLOYMENT_TARGET = "13.0"
ARCHES = ("arm64", "x86_64")
USER_AGENT = "mpv-deps-build/1.0"
DEP_ORDER = ("freetype", "fribidi", "harfbuzz", "libass", "ffmpeg")

# Official release archives. harfbuzz, fribidi and libass digests are the
# GitHub release asset digests; freetype and ffmpeg digests were computed from
# the downloaded archive bytes.
PINS = {
    "freetype": {
        "version": "2.15.0",
        "url": "https://download.savannah.gnu.org/releases/freetype/freetype-2.15.0.tar.xz",
        "sha256": "93cd47eb719c325107f2d3cf087d06299e944c3b48a40b55537302304dea1dd5",
        "license": "FreeType License (FTL) or GPL-2.0-or-later",
        "license_files": ["docs/FTL.TXT", "docs/GPLv2.TXT"],
    },
    "fribidi": {
        "version": "1.0.18",
        "url": "https://github.com/fribidi/fribidi/releases/download/v1.0.18/fribidi-1.0.18.tar.xz",
        "sha256": "1de5e8073d035388b71d0b2d452b1da1f8caeea8a7702997b401fe1b6250727f",
        "license": "LGPL-2.1-or-later",
        "license_files": ["COPYING"],
    },
    "harfbuzz": {
        "version": "15.1.0",
        "url": "https://github.com/harfbuzz/harfbuzz/releases/download/15.1.0/harfbuzz-15.1.0.tar.xz",
        "sha256": "0fc7c565443a5f4be7da65ac5ee5bd737d9b158dc908708b7689a0a5796b0c8d",
        "license": "MIT",
        "license_files": ["COPYING"],
    },
    "libass": {
        "version": "0.17.5",
        "url": "https://github.com/libass/libass/releases/download/0.17.5/libass-0.17.5.tar.xz",
        "sha256": "2dca25c0e0c837ddf00b52011b3f82cac1e4ddd3ad018227806b0c2288864acc",
        "license": "ISC",
        "license_files": ["COPYING"],
    },
    "ffmpeg": {
        "version": "8.0.2",
        "url": "https://ffmpeg.org/releases/ffmpeg-8.0.2.tar.xz",
        "sha256": "5d16962332603c427b3d0887fc12b9166d6ee2cb1108b1865dd2d5eb06a09505",
        "license": "LGPL-2.1-or-later",
        "license_files": ["LICENSE.md", "COPYING.LGPLv2.1"],
    },
}

# Option lists are part of each dependency's stamp key.
FREETYPE_OPTS = [
    "-DFT_DISABLE_HARFBUZZ=ON",
    "-DFT_DISABLE_BROTLI=ON",
    "-DFT_DISABLE_BZIP2=ON",
    "-DFT_DISABLE_PNG=ON",
    "-DFT_REQUIRE_ZLIB=ON",
]
FRIBIDI_OPTS = [
    "-Ddocs=false",
    "-Dtests=false",
    "-Dbin=false",
]
HARFBUZZ_OPTS = [
    "-Dfreetype=enabled",
    "-Dglib=disabled",
    "-Dgobject=disabled",
    "-Dicu=disabled",
    "-Dcairo=disabled",
    "-Dcoretext=disabled",
    "-Dgpu=disabled",
    "-Draster=disabled",
    "-Dvector=disabled",
    "-Dsubset=disabled",
    "-Dtests=disabled",
    "-Ddocs=disabled",
    "-Dutilities=disabled",
    "-Dbenchmark=disabled",
]
LIBASS_OPTS = [
    "-Dfontconfig=disabled",
    "-Dcoretext=enabled",
    "-Ddirectwrite=disabled",
    "-Dlibunibreak=disabled",
    "-Dasm=enabled",
    "-Dtest=disabled",
    "-Dfuzz=disabled",
    "-Dcompare=disabled",
]
FFMPEG_OPTS = [
    "--enable-static",
    "--disable-shared",
    "--enable-pic",
    "--disable-autodetect",
    "--disable-doc",
    "--disable-programs",
    "--disable-network",
    "--disable-avdevice",
    "--disable-encoders",
    "--disable-protocols",
    "--enable-protocol=file",
    "--enable-decoders",
    "--enable-demuxers",
    "--enable-parsers",
    "--enable-videotoolbox",
    "--enable-zlib",
    "--enable-iconv",
]
OPTS = {
    "freetype": FREETYPE_OPTS,
    "fribidi": FRIBIDI_OPTS,
    "harfbuzz": HARFBUZZ_OPTS,
    "libass": LIBASS_OPTS,
    "ffmpeg": FFMPEG_OPTS,
}

# Files that prove a dependency was installed into the prefix.
MARKERS = {
    "freetype": ["lib/libfreetype.a", "lib/pkgconfig/freetype2.pc"],
    "fribidi": ["lib/libfribidi.a", "lib/pkgconfig/fribidi.pc"],
    "harfbuzz": ["lib/libharfbuzz.a", "lib/pkgconfig/harfbuzz.pc"],
    "libass": ["lib/libass.a", "lib/pkgconfig/libass.pc"],
    "ffmpeg": ["lib/libavcodec.a", "lib/pkgconfig/libavcodec.pc"],
}

REQUIRED_TOOLS = ("cmake", "meson", "ninja", "pkg-config", "make", "clang")


def log(msg):
    print(f"[mpv-deps] {msg}", file=sys.stderr, flush=True)


def jobs():
    return str(os.cpu_count() or 4)


def sha256_file(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def fetch(pin, downloads):
    dest = downloads / Path(pin["url"]).name
    if not dest.exists():
        log(f"downloading {pin['url']}")
        tmp = dest.with_name(dest.name + ".part")
        req = urllib.request.Request(pin["url"], headers={"User-Agent": USER_AGENT})
        with urllib.request.urlopen(req, timeout=300) as resp, open(tmp, "wb") as out:
            shutil.copyfileobj(resp, out)
        tmp.replace(dest)
    actual = sha256_file(dest)
    if actual != pin["sha256"]:
        dest.unlink()
        raise SystemExit(f"sha256 mismatch for {dest.name}: expected {pin['sha256']}, got {actual}")
    return dest


def extract(archive, name, srcroot):
    final_dir = None
    staging = srcroot / f".{name}-extract"
    shutil.rmtree(staging, ignore_errors=True)
    staging.mkdir(parents=True)
    kwargs = {"filter": "data"} if hasattr(tarfile, "data_filter") else {}
    log(f"extracting {archive.name}")
    with tarfile.open(archive, "r:xz") as tar:
        tar.extractall(staging, **kwargs)
    entries = list(staging.iterdir())
    if len(entries) != 1 or not entries[0].is_dir():
        raise SystemExit(f"{name}: archive must contain exactly one top-level directory")
    final_dir = srcroot / entries[0].name
    shutil.rmtree(final_dir, ignore_errors=True)
    entries[0].rename(final_dir)
    shutil.rmtree(staging)
    return final_dir


def build_env(arch, prefix):
    flags = f"-arch {arch} -mmacosx-version-min={DEPLOYMENT_TARGET}"
    env = dict(os.environ)
    for var in ("CFLAGS", "CXXFLAGS", "LDFLAGS", "OBJCFLAGS"):
        env[var] = f"{os.environ.get(var, '')} {flags}".strip()
    env["PKG_CONFIG_LIBDIR"] = f"{prefix}/lib/pkgconfig:{prefix}/share/pkgconfig"
    env.pop("PKG_CONFIG_PATH", None)
    # The SDK supplies libz and its headers, but no zlib.pc for static consumers.
    macros = subprocess.run(
        ["clang", "-dM", "-E", "-include", "zlib.h", "-"],
        input="", text=True, capture_output=True, check=True, env=env,
    ).stdout
    version = next(line.split('"')[1] for line in macros.splitlines()
                   if line.startswith("#define ZLIB_VERSION "))
    (prefix / "lib/pkgconfig").mkdir(parents=True, exist_ok=True)
    (prefix / "lib/pkgconfig/zlib.pc").write_text(
        f"Name: zlib\nDescription: macOS SDK zlib\nVersion: {version}\nLibs: -lz\n"
    )
    return env


@dataclasses.dataclass
class Build:
    name: str
    src: Path
    bdir: Path
    prefix: Path
    arch: str
    env: dict
    logs: Path

    def run(self, step, cmd, cwd=None):
        self.logs.mkdir(parents=True, exist_ok=True)
        path = self.logs / f"{self.name}-{self.arch}-{step}.log"
        log(f"{self.name}: {step} (log {path})")
        with open(path, "w") as out:
            out.write("$ " + " ".join(str(c) for c in cmd) + "\n")
            out.flush()
            rc = subprocess.call(
                [str(c) for c in cmd],
                cwd=str(cwd or self.src),
                env=self.env,
                stdout=out,
                stderr=subprocess.STDOUT,
            )
        if rc != 0:
            raise SystemExit(f"{self.name}: {step} failed (exit {rc}); see {path}")

    def fresh_bdir(self):
        shutil.rmtree(self.bdir, ignore_errors=True)
        self.bdir.mkdir(parents=True)


def build_cmake(b, opts):
    b.fresh_bdir()
    b.run("configure", [
        "cmake", "-S", b.src, "-B", b.bdir, "-G", "Ninja",
        f"-DCMAKE_INSTALL_PREFIX={b.prefix}",
        "-DCMAKE_INSTALL_LIBDIR=lib",
        "-DCMAKE_BUILD_TYPE=Release",
        "-DBUILD_SHARED_LIBS=OFF",
        "-DCMAKE_POSITION_INDEPENDENT_CODE=ON",
        f"-DCMAKE_OSX_ARCHITECTURES={b.arch}",
        f"-DCMAKE_OSX_DEPLOYMENT_TARGET={DEPLOYMENT_TARGET}",
        "-DCMAKE_IGNORE_PREFIX_PATH=/opt/homebrew;/usr/local",
        *opts,
    ])
    b.run("build", ["cmake", "--build", b.bdir, "-j", jobs()])
    b.run("install", ["cmake", "--install", b.bdir])


def build_meson(b, opts):
    b.fresh_bdir()
    cross_args = []
    if platform.machine() != b.arch:
        machine = b.bdir / "cross.ini"
        family = "aarch64" if b.arch == "arm64" else "x86_64"
        machine.write_text(
            "[binaries]\nc = 'clang'\ncpp = 'clang++'\n"
            "pkg-config = 'pkg-config'\n"
            f"exe_wrapper = ['arch', '-{b.arch}']\n"
            "[host_machine]\nsystem = 'darwin'\n"
            f"cpu_family = '{family}'\ncpu = '{b.arch}'\nendian = 'little'\n"
        )
        cross_args = ["--cross-file", machine]
    b.run("setup", [
        "meson", "setup", b.bdir, b.src,
        f"--prefix={b.prefix}",
        "--libdir=lib",
        "--buildtype=release",
        "--default-library=static",
        "-Db_staticpic=true",
        "--wrap-mode=nodownload",
        *cross_args,
        *opts,
    ])
    b.run("compile", ["meson", "compile", "-C", b.bdir, "-j", jobs()])
    b.run("install", ["meson", "install", "-C", b.bdir])


def build_freetype(b):
    build_cmake(b, OPTS["freetype"])


def build_fribidi(b):
    build_meson(b, OPTS["fribidi"])


def build_harfbuzz(b):
    build_meson(b, OPTS["harfbuzz"])


def build_libass(b):
    build_meson(b, OPTS["libass"])


def build_ffmpeg(b):
    b.fresh_bdir()
    flags = f"-arch {b.arch} -mmacosx-version-min={DEPLOYMENT_TARGET}"
    cross = [] if platform.machine() == b.arch else ["--enable-cross-compile"]
    b.run("configure", [
        b.src / "configure",
        f"--prefix={b.prefix}",
        f"--libdir={b.prefix}/lib",
        f"--arch={b.arch}",
        "--target-os=darwin",
        f"--extra-cflags={flags}",
        f"--extra-ldflags={flags}",
        *OPTS["ffmpeg"],
        *cross,
    ], cwd=b.bdir)
    b.run("build", ["make", "-j", jobs()], cwd=b.bdir)
    b.run("install", ["make", "install"], cwd=b.bdir)


BUILDERS = {
    "freetype": build_freetype,
    "fribidi": build_fribidi,
    "harfbuzz": build_harfbuzz,
    "libass": build_libass,
    "ffmpeg": build_ffmpeg,
}


def stamp_key(name, pin, arch):
    blob = json.dumps({
        "name": name,
        "version": pin["version"],
        "sha256": pin["sha256"],
        "arch": arch,
        "target": DEPLOYMENT_TARGET,
        "options": OPTS.get(name, []),
        "sdkroot": os.environ.get("SDKROOT", ""),
    }, sort_keys=True)
    return hashlib.sha256(blob.encode()).hexdigest()


def up_to_date(stamp, key, prefix, name):
    if not stamp.exists():
        return False
    if json.loads(stamp.read_text()).get("key") != key:
        return False
    return all((prefix / m).exists() for m in MARKERS[name])


def install_licenses(name, pin, src, licdir):
    dest = licdir / name
    shutil.rmtree(dest, ignore_errors=True)
    dest.mkdir(parents=True)
    for rel in pin["license_files"]:
        source = src / rel
        if not source.is_file():
            raise SystemExit(f"{name}: license file missing from source: {rel}")
        shutil.copy2(source, dest / source.name)
    (dest / "PROVENANCE.txt").write_text(
        f"name: {name}\n"
        f"version: {pin['version']}\n"
        f"license: {pin['license']}\n"
        f"url: {pin['url']}\n"
        f"sha256: {pin['sha256']}\n"
    )


def write_system_libraries(licdir):
    (licdir / "SYSTEM-LIBRARIES.txt").write_text(
        "Not bundled; provided by the macOS SDK and frameworks:\n"
        "zlib (libz, zlib license)\n"
        "iconv (libiconv, APSL/system)\n"
        "Frameworks: VideoToolbox, CoreText, CoreFoundation, CoreServices, Cocoa, OpenGL\n"
    )


def check_tools(arch):
    needed = list(REQUIRED_TOOLS)
    if arch == "x86_64":
        needed.append("nasm")
    missing = [t for t in needed if shutil.which(t) is None]
    if missing:
        raise SystemExit(f"missing build tools on PATH: {', '.join(missing)}")


def main():
    ap = argparse.ArgumentParser(description="Build static libmpv dependencies for macOS.")
    ap.add_argument("--work-dir", required=True, type=Path)
    ap.add_argument("--arch", required=True, choices=ARCHES)
    ap.add_argument("--deployment-target", default=DEPLOYMENT_TARGET)
    args = ap.parse_args()
    if args.deployment_target != DEPLOYMENT_TARGET:
        ap.error(f"--deployment-target must be {DEPLOYMENT_TARGET} (production baseline)")

    check_tools(args.arch)
    wd = args.work_dir.resolve()
    prefix = wd / "prefix"
    downloads = wd / "downloads"
    srcroot = wd / "src"
    builds = wd / "build"
    logs = wd / "logs"
    stamps = wd / "stamps"
    for d in (prefix, downloads, srcroot, builds, logs, stamps):
        d.mkdir(parents=True, exist_ok=True)

    env = build_env(args.arch, prefix)
    licdir = prefix / "share" / "licenses" / "libmpv"
    licdir.mkdir(parents=True, exist_ok=True)
    write_system_libraries(licdir)

    for name in DEP_ORDER:
        pin = PINS[name]
        key = stamp_key(name, pin, args.arch)
        stamp = stamps / f"{name}-{args.arch}.json"
        if up_to_date(stamp, key, prefix, name):
            log(f"{name} {pin['version']} up to date")
            continue

        archive = fetch(pin, downloads)
        src = extract(archive, name, srcroot)
        b = Build(
            name=name,
            src=src,
            bdir=builds / f"{name}-{args.arch}",
            prefix=prefix,
            arch=args.arch,
            env=env,
            logs=logs,
        )
        log(f"building {name} {pin['version']} for {args.arch}")
        BUILDERS[name](b)
        missing = [m for m in MARKERS[name] if not (prefix / m).exists()]
        if missing:
            raise SystemExit(f"{name}: install incomplete, missing {', '.join(missing)}")
        install_licenses(name, pin, src, licdir)
        stamp.write_text(json.dumps({"key": key, "name": name, "version": pin["version"]}, sort_keys=True))
        log(f"{name} {pin['version']} done")

    log(f"dependency prefix ready: {prefix}")


if __name__ == "__main__":
    main()
