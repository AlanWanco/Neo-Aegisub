#!/usr/bin/env python3
"""Copy non-system Mach-O dependencies into an app's Frameworks directory."""

import argparse
from pathlib import Path
import shutil
import subprocess


def output(*args):
    return subprocess.check_output(args, text=True)


def system_library(name):
    return name.startswith(("/usr/lib/", "/System/Library/"))


def dependencies(binary):
    # otool's version suffix is the delimiter; install names may contain spaces.
    return [line.strip().split(" (compatibility version", 1)[0]
            for line in output("otool", "-L", str(binary)).splitlines()[1:]]


def rpaths(binary, executable_dir):
    lines = output("otool", "-l", str(binary)).splitlines()
    paths = []
    for index, line in enumerate(lines):
        if line.strip() == "cmd LC_RPATH":
            value = lines[index + 2].strip().removeprefix("path ").split(" (offset", 1)[0]
            paths.append((value, Path(value.replace("@loader_path", str(binary.parent))
                                      .replace("@executable_path", str(executable_dir)))))
    return paths


def resolve(name, binary, executable_dir, search_paths):
    expanded = name.replace("@loader_path", str(binary.parent)).replace(
        "@executable_path", str(executable_dir))
    candidates = ([directory / name.removeprefix("@rpath/") for directory in search_paths]
                  if name.startswith("@rpath/") else [Path(expanded)])
    for candidate in candidates:
        if candidate.is_file():
            return candidate.resolve()
    raise RuntimeError(f"Cannot resolve {name} required by {binary}")


def copy_notices(source, destination):
    # Homebrew packages retain their licenses, source formula and SPDX SBOM.
    for root in source.parents:
        source_notices = root / "share" / "licenses" / "libmpv"
        if source_notices.is_dir():
            shutil.copytree(source_notices, destination, dirs_exist_ok=True)
            return
        if not (root / ".brew").is_dir():
            continue
        target = destination / f"{root.parent.name}-{root.name}"
        target.mkdir(parents=True, exist_ok=True)
        for item in root.iterdir():
            if item.is_file() and (item.name.upper().startswith(
                    ("LICENSE", "COPYING", "COPYRIGHT", "NOTICE"))
                    or item.name == "sbom.spdx.json"):
                shutil.copy2(item, target / item.name)
        for formula in (root / ".brew").glob("*.rb"):
            shutil.copy2(formula, target / formula.name)
        return


def bundle(executables, libraries):
    executable_dir = executables[0].resolve().parent
    contents = executable_dir.parent
    frameworks = contents / "Frameworks"
    frameworks.mkdir(parents=True, exist_ok=True)
    notices = contents / "Resources" / "licenses" / "libmpv"
    expected_arches = set(output("lipo", "-archs", str(executables[0])).split())
    copied = {}
    names = {}
    visited = set()

    def collect(source, target, inherited_paths):
        source = source.resolve()
        if source in visited:
            return
        visited.add(source)
        arches = set(output("lipo", "-archs", str(source)).split())
        if not expected_arches.issubset(arches):
            raise RuntimeError(f"Wrong architecture {arches}: {source}; expected {expected_arches}")
        source_rpaths = rpaths(source, executable_dir)
        paths = [expanded for _, expanded in source_rpaths] + inherited_paths
        ids = output("otool", "-D", str(source)).splitlines()[1:]
        changes = []
        for name in dependencies(source):
            if name in ids or system_library(name):
                continue
            dependency = resolve(name, source, executable_dir, paths)
            destination = copy_library(dependency, Path(name).name, paths)
            replacement = (f"@loader_path/{destination.name}" if target.parent == frameworks
                           else f"@loader_path/../Frameworks/{destination.name}")
            changes.extend(["-change", name, replacement])
        for original in dict.fromkeys(original for original, _ in source_rpaths):
            changes.extend(["-delete_rpath", original])
        if target.parent == frameworks:
            changes.extend(["-id", f"@rpath/{target.name}"])
        if changes:
            target.chmod(target.stat().st_mode | 0o200)
            subprocess.run(["install_name_tool", *changes, str(target)], check=True)

    def copy_library(source, name, inherited_paths):
        source = source.resolve()
        if source in copied:
            return copied[source]
        if name in names and names[name] != source:
            raise RuntimeError(f"Different libraries share bundled name {name}: {source}, {names[name]}")
        names[name] = source
        target = frameworks / name
        copied[source] = target
        shutil.copy2(source, target)
        copy_notices(source, notices)
        collect(source, target, inherited_paths)
        print(f"Bundled {name}")
        return target

    for executable in executables:
        collect(executable, executable, [])
    for library in libraries:
        copy_library(library, library.name, [])


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("executables", type=Path, nargs="+")
    parser.add_argument("--library", type=Path, action="append", default=[])
    arguments = parser.parse_args()
    bundle(arguments.executables, arguments.library)
