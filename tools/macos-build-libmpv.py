#!/usr/bin/env python3
"""Build the macOS libmpv used by Aegisub's native video backend.

Runs the dependency layer (macos-build-mpv-deps.py), then builds static
glslang and libplacebo and a shared LGPL libmpv with Cocoa/Vulkan video output.
MoltenVK is pinned separately and linked directly, without a Vulkan loader.
Logs go to stderr; the resulting libmpv path is the only stdout.

Usage:
  macos-build-libmpv.py --work-dir DIR --arch arm64|x86_64 --deployment-target 13.0
"""

import argparse
import hashlib
import json
import os
import shlex
import shutil
import subprocess
import sys
import tarfile
from pathlib import Path


SCRIPT_DIR = Path(__file__).resolve().parent
DEPS_SCRIPT = SCRIPT_DIR / "macos-build-mpv-deps.py"

DEPLOYMENT_TARGET = "13.0"
ARCH_TRIPLES = {
    "arm64": "arm64-apple-macos13.0",
    "x86_64": "x86_64-apple-macos13.0",
}
MESON_CPU_FAMILY = {"arm64": "aarch64", "x86_64": "x86_64"}

LIBPLACEBO = {
    "version": "7.387.0",
    "url": "https://code.videolan.org/videolan/libplacebo/-/archive/v7.387.0/libplacebo-v7.387.0.tar.gz",
    "sha256": "3bf5d72786682c194387c33d563872f6bfe202a9816f9dde6535a31a9b2e0d74",
}
# Submodules of the libplacebo v7.387.0 tag (commit pins from its git tree);
# the release archive does not contain them.
LIBPLACEBO_SUBMODULES = [
    {
        "name": "glad",
        "commit": "73db193f853e2ee079bf3ca8a64aa2eaf6459043",
        "url": "https://github.com/Dav1dde/glad/archive/73db193f853e2ee079bf3ca8a64aa2eaf6459043.tar.gz",
        "sha256": "33dbeae44d8315ece57e14eba4b1b4a02ac5406d1c4f49cd20048c91522a6b9a",
    },
    {
        "name": "jinja",
        "commit": "15206881c006c79667fe5154fe80c01c65410679",
        "url": "https://github.com/pallets/jinja/archive/15206881c006c79667fe5154fe80c01c65410679.tar.gz",
        "sha256": "b88a20dcc2e34072fcf4159325bc6c34cd4b29a81a8b83d15d2f28ba561da296",
    },
    {
        "name": "markupsafe",
        "commit": "297fc8e356e6836a62087949245d09a28e9f1b13",
        "url": "https://github.com/pallets/markupsafe/archive/297fc8e356e6836a62087949245d09a28e9f1b13.tar.gz",
        "sha256": "da7c010c9c81a66ac73036558c1fcb6212b50482f43211cd1254035b94f82414",
    },
    {
        "name": "fast_float",
        "commit": "97b54ca9e75f5303507699d27c6b4f4efe4641a1",
        "url": "https://github.com/fastfloat/fast_float/archive/97b54ca9e75f5303507699d27c6b4f4efe4641a1.tar.gz",
        "sha256": "2b132274539286e41f37857cac22aa8441d21bd86d55de825a3342b149f66801",
    },
    {
        "name": "Vulkan-Headers",
        "commit": "74d8a6cb930c68ef617b202c3ff3c59d919e086b",
        "url": "https://github.com/KhronosGroup/Vulkan-Headers/archive/74d8a6cb930c68ef617b202c3ff3c59d919e086b.tar.gz",
        "sha256": "75967a8288ff16044a86d1af462b96d2e75c435937efb75f2bb4102ab0382f29",
    },
]

MPV = {
    "version": "0.41.0",
    "url": "https://codeload.github.com/mpv-player/mpv/tar.gz/refs/tags/v0.41.0",
    "sha256": "ee21092a5ee427353392360929dc64645c54479aefdb5babc5cfbb5fad626209",
}

GLSLANG = {
    "version": "14.3.0",
    "url": "https://codeload.github.com/KhronosGroup/glslang/tar.gz/refs/tags/14.3.0",
    "sha256": "be6339048e20280938d9cb399fcdd06e04f8654d43e170e8cce5a56c9a754284",
}
MOLTENVK = {
    "version": "1.4.2",
    "url": "https://github.com/KhronosGroup/MoltenVK/releases/download/v1.4.2/MoltenVK-macos.tar",
    "sha256": "f95765a6229cb7b915990a2890ce12ebe36a730b021545d3d52ae69ce4c4024e",
}

LIBPLACEBO_OPTIONS = [
    "--default-library=static",
    "-Ddemos=false",
    "-Dtests=false",
    "-Dbench=false",
    "-Dfuzz=false",
    "-Dopengl=enabled",
    "-Dvulkan=enabled",
    "-Dvk-proc-addr=enabled",
    "-Dd3d11=disabled",
    "-Dglslang=enabled",
    "-Dshaderc=disabled",
    "-Dlcms=disabled",
    "-Ddovi=disabled",
    "-Dlibdovi=disabled",
    "-Dunwind=disabled",
    "-Dxxhash=disabled",
]

MPV_DISABLED_FEATURES = [
    "cdda", "cplugins", "dvbin", "dvdnav", "libarchive", "libavdevice",
    "libbluray", "rubberband", "uchardet", "vapoursynth",
    "zimg", "sixel", "jpeg", "caca", "lcms2", "alsa", "jack", "openal",
    "pipewire", "pulse", "sndio", "oss-audio", "x11", "wayland", "drm", "gbm",
    "egl", "vaapi", "vdpau", "cuda-hwaccel", "cuda-interop", "dmabuf-wayland",
    "shaderc", "spirv-cross",
]


def log(message):
    print(f"[macos-build-libmpv] {message}", file=sys.stderr, flush=True)


def run(command, env):
    log("+ " + shlex.join(str(part) for part in command))
    subprocess.run(command, check=True, env=env, stdout=sys.stderr, stderr=sys.stderr)


def stamp_key(payload):
    return hashlib.sha256(json.dumps(payload, sort_keys=True).encode()).hexdigest()


def sha256_file(path):
    digest = hashlib.sha256()
    with path.open("rb") as file:
        for chunk in iter(lambda: file.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def fetch(work, url, sha256):
    downloads = work / "downloads"
    downloads.mkdir(parents=True, exist_ok=True)
    archive = downloads / f"{sha256[:16]}-{url.rsplit('/', 1)[-1]}"
    if archive.exists() and sha256_file(archive) == sha256:
        return archive
    partial = archive.with_name(archive.name + ".part")
    partial.unlink(missing_ok=True)
    run(
        ["curl", "--fail", "--location", "--silent", "--show-error", "--retry", "3",
         "--output", partial, url],
        os.environ,
    )
    actual = sha256_file(partial)
    if actual != sha256:
        partial.unlink()
        raise SystemExit(f"sha256 mismatch for {url}: expected {sha256}, got {actual}")
    partial.replace(archive)
    return archive


def unpack(archive, target):
    staging = target.with_name(target.name + ".extract")
    shutil.rmtree(target, ignore_errors=True)
    shutil.rmtree(staging, ignore_errors=True)
    staging.mkdir(parents=True)
    with tarfile.open(archive) as tar:
        tar.extractall(staging, filter="data")
    entries = list(staging.iterdir())
    if len(entries) != 1 or not entries[0].is_dir():
        raise SystemExit(f"{archive}: expected exactly one top-level directory")
    target.parent.mkdir(parents=True, exist_ok=True)
    entries[0].rename(target)
    shutil.rmtree(staging)


def host_arch():
    # hw.optional.arm64 reports 1 on Apple silicon even when Python runs under Rosetta.
    out = subprocess.run(
        ["sysctl", "-n", "hw.optional.arm64"], check=False, capture_output=True, text=True
    ).stdout.strip()
    return "arm64" if out == "1" else "x86_64"


def machine_file(arch, prefix):
    host = host_arch()
    flags = ["-arch", arch, f"-mmacosx-version-min={DEPLOYMENT_TARGET}"]
    compile_flags = flags + [f"-I{prefix}/include"]
    link_flags = flags + [f"-L{prefix}/lib"]

    def array(values):
        return "[" + ", ".join(f"'{value}'" for value in values) + "]"

    lines = [
        "[binaries]",
        "c = 'clang'",
        "cpp = 'clang++'",
        "objc = 'clang'",
        "pkgconfig = 'pkg-config'",
    ]
    kind = "native"
    if arch != host:
        kind = "cross"
        if arch == "x86_64":
            lines.append(f"exe_wrapper = {array(['arch', '-x86_64'])}")
    lines += [
        "",
        "[built-in options]",
        f"c_args = {array(compile_flags)}",
        f"cpp_args = {array(compile_flags)}",
        f"objc_args = {array(compile_flags)}",
        f"c_link_args = {array(link_flags)}",
        f"cpp_link_args = {array(link_flags)}",
        f"objc_link_args = {array(link_flags)}",
    ]
    if kind == "cross":
        lines += [
            "",
            "[host_machine]",
            "system = 'darwin'",
            f"cpu_family = '{MESON_CPU_FAMILY[arch]}'",
            f"cpu = '{arch}'",
            "endian = 'little'",
        ]
    return kind, "\n".join(lines) + "\n"


def build_env(prefix):
    env = os.environ.copy()
    env["PKG_CONFIG_LIBDIR"] = f"{prefix}/lib/pkgconfig:{prefix}/share/pkgconfig"
    env.pop("PKG_CONFIG_PATH", None)
    env["PATH"] = f"{prefix}/bin:{env.get('PATH', '')}"
    return env


def meson_build(src, build, prefix, machine, kind, options, env):
    shutil.rmtree(build, ignore_errors=True)
    run(
        ["meson", "setup", build, src, f"--prefix={prefix}", "--libdir=lib",
         "--buildtype=release", "--wrap-mode=nodownload", f"--{kind}-file", machine,
         *options],
        env,
    )
    run(["meson", "compile", "-C", build], env)
    run(["meson", "install", "-C", build], env)


def is_current(stamp, key, artifact):
    return artifact.exists() and stamp.exists() and stamp.read_text().strip() == key


def build_glslang(work, arch, prefix, env):
    options = [
        "-G", "Ninja", "-DCMAKE_BUILD_TYPE=Release",
        f"-DCMAKE_INSTALL_PREFIX={prefix}", "-DCMAKE_INSTALL_LIBDIR=lib",
        f"-DCMAKE_OSX_ARCHITECTURES={arch}",
        f"-DCMAKE_OSX_DEPLOYMENT_TARGET={DEPLOYMENT_TARGET}",
        f"-DCMAKE_OSX_SYSROOT={env.get('SDKROOT', 'macosx')}",
        "-DCMAKE_POSITION_INDEPENDENT_CODE=ON", "-DBUILD_SHARED_LIBS=OFF",
        "-DBUILD_EXTERNAL=OFF", "-DENABLE_OPT=OFF", "-DENABLE_HLSL=OFF",
        "-DENABLE_GLSLANG_BINARIES=OFF", "-DENABLE_SPVREMAPPER=OFF",
        "-DGLSLANG_TESTS=OFF", "-DGLSLANG_ENABLE_INSTALL=ON",
    ]
    key = stamp_key({"pin": GLSLANG, "options": options})
    stamp = work / "stamps" / f"glslang-{arch}.stamp"
    artifact = prefix / "lib" / "libSPIRV.a"
    if is_current(stamp, key, artifact):
        log("glslang up to date")
        return
    src = work / "src" / "glslang"
    unpack(fetch(work, GLSLANG["url"], GLSLANG["sha256"]), src)
    build = work / "build" / f"glslang-{arch}"
    shutil.rmtree(build, ignore_errors=True)
    run(["cmake", "-S", src, "-B", build, *options], env)
    run(["cmake", "--build", build, "--parallel"], env)
    run(["cmake", "--install", build], env)
    stamp.parent.mkdir(parents=True, exist_ok=True)
    stamp.write_text(key + "\n")


def install_moltenvk(work, arch, prefix, env):
    src = work / "src" / "MoltenVK"
    unpack(fetch(work, MOLTENVK["url"], MOLTENVK["sha256"]), src)
    artifact = prefix / "lib" / "libMoltenVK.dylib"
    run(["lipo", src / "MoltenVK/dynamic/dylib/macOS/libMoltenVK.dylib",
         "-thin", arch, "-output", artifact], env)
    run(["install_name_tool", "-id", artifact, artifact], env)
    run(["codesign", "--force", "--sign", "-", artifact], env)
    shutil.copytree(src / "MoltenVK/include", prefix / "include", dirs_exist_ok=True)
    (prefix / "lib/pkgconfig/vulkan.pc").write_text(
        f"prefix={prefix}\nlibdir=${{prefix}}/lib\nincludedir=${{prefix}}/include\n"
        f"Name: Vulkan\nDescription: Vulkan through pinned MoltenVK\nVersion: {MOLTENVK['version']}\n"
        "Libs: -L${libdir} -lMoltenVK\nCflags: -I${includedir}\n"
    )
    verify_libmpv(artifact, arch)


def build_libplacebo(work, arch, prefix, env, kind, machine_text):
    options = [*LIBPLACEBO_OPTIONS, f"-Dvulkan-sdk={prefix}"]
    patch = SCRIPT_DIR / "patches" / "libplacebo-glslang-prefix.patch"
    key = stamp_key({
        "name": "libplacebo",
        "pin": LIBPLACEBO,
        "submodules": LIBPLACEBO_SUBMODULES,
        "patch": sha256_file(patch),
        "glslang": GLSLANG,
        "moltenvk": MOLTENVK,
        "options": options,
        "arch": arch,
        "deployment_target": DEPLOYMENT_TARGET,
        "machine": machine_text,
    })
    stamp = work / "stamps" / f"libplacebo-{arch}.stamp"
    artifact = prefix / "lib" / "libplacebo.a"
    if is_current(stamp, key, artifact):
        log("libplacebo up to date")
        return key

    stamp.unlink(missing_ok=True)
    src = work / "src" / "libplacebo"
    unpack(fetch(work, LIBPLACEBO["url"], LIBPLACEBO["sha256"]), src)
    # The upstream static glslang lookup omits its configured SDK library directory.
    run(["patch", "--batch", "--fuzz=0", "-p1", "-d", src, "-i", patch], env)
    for sub in LIBPLACEBO_SUBMODULES:
        unpack(fetch(work, sub["url"], sub["sha256"]), src / "3rdparty" / sub["name"])

    meson_build(src, work / "build" / f"libplacebo-{arch}", prefix,
                work / "meson" / f"{arch}.ini", kind, options, env)
    if not artifact.exists():
        raise SystemExit(f"libplacebo build did not produce {artifact}")
    stamp.parent.mkdir(parents=True, exist_ok=True)
    stamp.write_text(key + "\n")
    return key


def build_mpv(work, arch, prefix, env, kind, machine_text, libplacebo_key):
    options = [
        "--default-library=shared",
        "-Dlibmpv=true",
        "-Dgpl=false",
        "-Dcplayer=false",
        "-Dtests=false",
        "-Dfuzzers=false",
        "-Dbuild-date=false",
        "-Dmanpage-build=disabled",
        "-Dhtml-build=disabled",
        "-Dpdf-build=disabled",
        "-Dlua=disabled",
        "-Djavascript=disabled",
        "-Dcocoa=enabled",
        "-Dgl=enabled",
        "-Dgl-cocoa=enabled",
        "-Dmacos-cocoa-cb=enabled",
        "-Dswift-build=enabled",
        "-Dvulkan=enabled",
        "-Dvideotoolbox-pl=enabled",
        f"-Dswift-flags=-target {ARCH_TRIPLES[arch]}",
    ] + [f"-D{feature}=disabled" for feature in MPV_DISABLED_FEATURES]
    key = stamp_key({
        "name": "mpv",
        "pin": MPV,
        "options": options,
        "libplacebo": libplacebo_key,
        "arch": arch,
        "deployment_target": DEPLOYMENT_TARGET,
        "machine": machine_text,
    })
    stamp = work / "stamps" / f"mpv-{arch}.stamp"
    artifact = prefix / "lib" / "libmpv.2.dylib"
    if is_current(stamp, key, artifact):
        log("libmpv up to date")
        return

    stamp.unlink(missing_ok=True)
    src = work / "src" / "mpv"
    unpack(fetch(work, MPV["url"], MPV["sha256"]), src)
    meson_build(src, work / "build" / f"mpv-{arch}", prefix,
                work / "meson" / f"{arch}.ini", kind, options, env)
    if not artifact.exists():
        raise SystemExit(f"libmpv build did not produce {artifact}")
    stamp.parent.mkdir(parents=True, exist_ok=True)
    stamp.write_text(key + "\n")


def version_key(version):
    parts = [int(part) for part in version.split(".")]
    return tuple(parts + [0] * (3 - len(parts)))


def minimum_versions(binary, arch):
    output = subprocess.run(
        ["otool", "-arch", arch, "-l", str(binary)],
        check=True, capture_output=True, text=True,
    ).stdout
    versions = []
    command = None
    for line in output.splitlines():
        fields = line.split()
        if fields[:1] == ["cmd"]:
            command = fields[1] if len(fields) > 1 else None
        elif command == "LC_BUILD_VERSION" and fields[:1] == ["minos"]:
            versions.append(fields[1])
        elif command == "LC_VERSION_MIN_MACOSX" and fields[:1] == ["version"]:
            versions.append(fields[1])
    return versions


def linked_libraries(binary, arch):
    output = subprocess.run(
        ["otool", "-arch", arch, "-L", str(binary)],
        check=True, capture_output=True, text=True,
    ).stdout
    # First line is the "<file>:" header.
    return [line.strip().split(" (", 1)[0] for line in output.splitlines()[1:] if line.strip()]


def verify_libmpv(lib, arch):
    versions = minimum_versions(lib, arch)
    if not versions:
        raise SystemExit(f"{lib}: no macOS deployment target load command")
    too_new = [v for v in versions if version_key(v) > version_key(DEPLOYMENT_TARGET)]
    if too_new:
        raise SystemExit(f"{lib}: minos {', '.join(too_new)} exceeds {DEPLOYMENT_TARGET}")
    for name in linked_libraries(lib, arch):
        if name.startswith(("/usr/lib/", "/System/Library/")):
            continue
        if Path(name).name == lib.name:
            continue
        if lib.name == "libmpv.2.dylib" and Path(name) == lib.parent / "libMoltenVK.dylib":
            continue
        raise SystemExit(f"{lib}: links non-system library {name}")
    log(f"verified {lib}: minos <= {DEPLOYMENT_TARGET}, only bundled/system dependencies")


def install_licenses(work, prefix):
    dest = prefix / "share" / "licenses" / "libmpv"
    for component in ("libplacebo", "mpv"):
        shutil.rmtree(dest / component, ignore_errors=True)
    src_placebo = work / "src" / "libplacebo"
    src_mpv = work / "src" / "mpv"

    (dest / "libplacebo").mkdir(parents=True)
    shutil.copy2(src_placebo / "LICENSE", dest / "libplacebo" / "LICENSE")
    (dest / "mpv").mkdir(parents=True)
    shutil.copy2(src_mpv / "LICENSE.LGPL", dest / "mpv" / "LICENSE.LGPL")
    shutil.copy2(src_mpv / "Copyright", dest / "mpv" / "Copyright")
    for component, license_name in (("glslang", "LICENSE.txt"), ("MoltenVK", "LICENSE")):
        target = dest / component
        target.mkdir(parents=True, exist_ok=True)
        shutil.copy2(work / "src" / component / license_name, target / license_name)

    for sub in LIBPLACEBO_SUBMODULES:
        found = sorted((src_placebo / "3rdparty" / sub["name"]).glob("LICENSE*"))
        if not found:
            raise SystemExit(f"no license file found for libplacebo submodule {sub['name']}")
        target = dest / "libplacebo" / "3rdparty" / sub["name"]
        target.mkdir(parents=True)
        for path in found:
            if path.is_dir():
                shutil.copytree(path, target / path.name)
            else:
                shutil.copy2(path, target / path.name)

    lines = [
        "libmpv build provenance",
        f"deployment_target: {DEPLOYMENT_TARGET}",
        f"libplacebo {LIBPLACEBO['version']} (static, OpenGL and Vulkan)",
        f"  url: {LIBPLACEBO['url']}",
        f"  sha256: {LIBPLACEBO['sha256']}",
    ]
    for sub in LIBPLACEBO_SUBMODULES:
        lines.append(f"  3rdparty/{sub['name']} commit {sub['commit']}")
        lines.append(f"    url: {sub['url']}")
        lines.append(f"    sha256: {sub['sha256']}")
    lines += [
        f"mpv {MPV['version']} (shared libmpv, LGPL; gpl=false, cplayer=false)",
        f"  url: {MPV['url']}",
        f"  sha256: {MPV['sha256']}",
        "  license: LGPL-2.1-or-later (mpv/LICENSE.LGPL)",
    ]
    for name, pin in (("glslang", GLSLANG), ("MoltenVK", MOLTENVK)):
        lines += [f"{name} {pin['version']}", f"  url: {pin['url']}",
                  f"  sha256: {pin['sha256']}"]
    (dest / "PROVENANCE.txt").write_text("\n".join(lines) + "\n")


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--work-dir", required=True)
    parser.add_argument("--arch", required=True, choices=sorted(ARCH_TRIPLES))
    parser.add_argument("--deployment-target", required=True, choices=[DEPLOYMENT_TARGET])
    args = parser.parse_args()

    work = Path(args.work_dir).resolve()
    arch = args.arch
    work.mkdir(parents=True, exist_ok=True)

    run(
        [sys.executable, str(DEPS_SCRIPT), "--work-dir", str(work), "--arch", arch,
         "--deployment-target", DEPLOYMENT_TARGET],
        os.environ,
    )
    prefix = work / "prefix"
    if not prefix.is_dir():
        raise SystemExit(f"dependency layer did not create {prefix}")

    env = build_env(prefix)
    kind, machine_text = machine_file(arch, prefix)
    machine = work / "meson" / f"{arch}.ini"
    machine.parent.mkdir(parents=True, exist_ok=True)
    machine.write_text(machine_text)
    build_glslang(work, arch, prefix, env)
    install_moltenvk(work, arch, prefix, env)

    libplacebo_key = build_libplacebo(work, arch, prefix, env, kind, machine_text)
    build_mpv(work, arch, prefix, env, kind, machine_text, libplacebo_key)

    lib = prefix / "lib" / "libmpv.2.dylib"
    verify_libmpv(lib, arch)
    install_licenses(work, prefix)
    print(lib.resolve())
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
