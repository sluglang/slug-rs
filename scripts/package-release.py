#!/usr/bin/env python3
"""Assemble a release archive of the Slug CLI for the current platform.

    python3 -I scripts/package-release.py --version V --slug PATH --repl PATH [--out DIR]

The archive is an installation: `bin/` holds `slug` and `slug-repl`, and the
directory above it is the Slug home they find without `$SLUG_HOME`:

    slug-<version>-<platform>/
      bin/slug, bin/slug-repl
      lib/
      clutch/manifest.toml, clutch/<name>.clutch/...
      LICENSE.md, README.md

Each clutch ships its manifest, modules, and assets with the native library
staged for this platform (`scripts/stage-native-clutches.sh` builds them
first); its tests, vendored sources, and other platforms' builds stay out. A
clutch whose native library was not staged here still ships, and reports
that at import. The archive is a `.zip` on Windows and a `.tar.gz` elsewhere;
its SHA-256 is written beside it, and its path is printed.
"""

import argparse
import hashlib
import os
import platform
import shutil
import sys
import tarfile
import zipfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
# Built for staging but not loaded at run time.
STAGING_ONLY = (".a", ".lib", ".c", ".h", ".o", ".obj", ".exp", ".pdb")
# Clutch entries that are for developing the clutch, not using it.
CLUTCH_SKIP = {"tests", "tools"}
SKIP_NAMES = {".DS_Store"}


def current_platform():
    machine = platform.machine().lower()
    arch = {"arm64": "aarch64", "aarch64": "aarch64", "x86_64": "x86_64", "amd64": "x86_64"}.get(machine)
    system = {"Darwin": "macos", "Linux": "linux", "Windows": "windows"}.get(platform.system())
    name = f"{system}-{arch}"
    if name not in {"macos-aarch64", "macos-x86_64", "linux-x86_64", "windows-x86_64"}:
        sys.exit(f"releases are unsupported on {platform.system()}-{platform.machine()}")
    return name


def ignore_junk(_directory, names):
    return [name for name in names if name in SKIP_NAMES]


def copy_clutch(source, destination, target):
    os.makedirs(destination)
    for entry in sorted(os.listdir(source)):
        path = os.path.join(source, entry)
        if entry in CLUTCH_SKIP or entry in SKIP_NAMES:
            continue
        if entry == "native":
            built = os.path.join(path, target)
            if os.path.isdir(built):
                out = os.path.join(destination, "native", target)
                os.makedirs(out)
                for name in sorted(os.listdir(built)):
                    if not name.endswith(STAGING_ONLY) and os.path.isfile(os.path.join(built, name)):
                        shutil.copy2(os.path.join(built, name), out)
            continue
        if os.path.isdir(path):
            shutil.copytree(path, os.path.join(destination, entry), ignore=ignore_junk)
        else:
            shutil.copy2(path, destination)


def sha256(path):
    digest = hashlib.sha256()
    with open(path, "rb") as file:
        for block in iter(lambda: file.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--version", required=True, help="release version, without the leading v")
    parser.add_argument("--slug", required=True, help="the release `slug` executable")
    parser.add_argument("--repl", required=True, help="the release `slug-repl` executable")
    parser.add_argument("--out", default=os.path.join(ROOT, "target/release-archive"))
    options = parser.parse_args()

    target = current_platform()
    windows = target.startswith("windows")
    name = f"slug-{options.version}-{target}"
    out = os.path.abspath(options.out)
    home = os.path.join(out, name)
    shutil.rmtree(home, ignore_errors=True)

    bin_dir = os.path.join(home, "bin")
    os.makedirs(bin_dir)
    suffix = ".exe" if windows else ""
    for executable, installed in ((options.slug, "slug"), (options.repl, "slug-repl")):
        shutil.copy2(executable, os.path.join(bin_dir, installed + suffix))

    shutil.copytree(os.path.join(ROOT, "lib"), os.path.join(home, "lib"), ignore=ignore_junk)
    clutches = os.path.join(ROOT, "clutch")
    os.makedirs(os.path.join(home, "clutch"))
    shutil.copy2(os.path.join(clutches, "manifest.toml"), os.path.join(home, "clutch"))
    for entry in sorted(os.listdir(clutches)):
        source = os.path.join(clutches, entry)
        if entry.endswith(".clutch") and os.path.isfile(os.path.join(source, "clutch.toml")):
            copy_clutch(source, os.path.join(home, "clutch", entry), target)
    for document in ("LICENSE.md", "README.md"):
        shutil.copy2(os.path.join(ROOT, document), home)

    if windows:
        archive = f"{home}.zip"
        with zipfile.ZipFile(archive, "w", zipfile.ZIP_DEFLATED) as zipped:
            for directory, _, files in sorted(os.walk(home)):
                for file in sorted(files):
                    path = os.path.join(directory, file)
                    zipped.write(path, os.path.relpath(path, out))
    else:
        archive = f"{home}.tar.gz"
        with tarfile.open(archive, "w:gz") as tarred:
            tarred.add(home, arcname=name)
    with open(f"{archive}.sha256", "w", newline="\n") as file:
        file.write(f"{sha256(archive)}  {os.path.basename(archive)}\n")
    print(archive)


if __name__ == "__main__":
    main()
