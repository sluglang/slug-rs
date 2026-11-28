#!/usr/bin/env python3
"""Check an unpacked release archive works as an installation.

    python3 -I scripts/release-smoke.py HOME [--version V] [--skip MODULE...]

Runs `HOME/bin/slug` from a scratch directory outside any checkout, with
`$SLUG_HOME` unset, so it must find its library and clutches beside itself.
Each check is a small program importing one installed module; those with
native libraries call into them, so each staged library is loaded. `--skip`
leaves out modules the platform does not ship a native build of.
"""

import argparse
import os
import subprocess
import sys
import tempfile

# module -> (program, expected stdout)
CHECKS = {
    "slug.std": (
        'val {map} = import("slug.std")\nprintln(map([1, 2, 3], fn(x) { x * 2 }))\n',
        "[2, 4, 6]",
    ),
    "slug.time": (
        'val time = import("slug.time")\nprintln(time.nowMs() > 0)\n',
        "true",
    ),
    "slug.fmt.json": (
        'val json = import("slug.fmt.json")\nprintln(json.parse("[1, 2]"))\n',
        "[1, 2]",
    ),
    "slug.text.mustache": (
        'val mustache = import("slug.text.mustache")\nprintln(mustache.render("{{a}}!", {a: "hi"}))\n',
        "hi!",
    ),
    "slug.text.markdown": (
        'val markdown = import("slug.text.markdown")\nprint(markdown.toHtml("# Hi"))\n',
        "<h1>Hi</h1>",
    ),
    "slug.net.http": ('val http = import("slug.net.http")\nprintln("loaded")\n', "loaded"),
    "slug.crypto.hash": (
        'val hash = import("slug.crypto.hash")\nprintln(hash.hex(hash.sha256Of("abc")))\n',
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
    ),
    "slug.random.secure": (
        'val secure = import("slug.random.secure")\nprintln(len(secure.bytes(16)))\n',
        "16",
    ),
    "slug.io.fs": (
        'val fs = import("slug.io.fs")\nval root = fs.temporaryDirectory("slug-smoke-")\nprintln("made")\n',
        "made",
    ),
    "slug.net.tcp": (
        'val tcp = import("slug.net.tcp")\nprintln(tcp.localPort(tcp.listen("127.0.0.1", 0)) > 0)\n',
        "true",
    ),
    "slug.db.sqlite": (
        'val sqlite = import("slug.db.sqlite")\nval db = sqlite.open(":memory:")\nprintln("opened")\n',
        "opened",
    ),
    "slug.arcade": ('val arcade = import("slug.arcade")\nprintln("loaded")\n', "loaded"),
    "slug.games.crawl": ('val game = import("slug.games.crawl")\nprintln("loaded")\n', "loaded"),
}


def run(command, cwd, environment):
    return subprocess.run(command, cwd=cwd, env=environment, capture_output=True, text=True, timeout=120)


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("home", help="the unpacked archive's top directory")
    parser.add_argument("--version", help="the version `slug --version` must report")
    parser.add_argument("--skip", nargs="*", default=[], help="modules not shipped natively here")
    options = parser.parse_args()

    unknown = set(options.skip) - CHECKS.keys()
    if unknown:
        sys.exit(f"no checks named {', '.join(sorted(unknown))}")
    home = os.path.abspath(options.home)
    suffix = ".exe" if os.name == "nt" else ""
    slug = os.path.join(home, "bin", "slug" + suffix)
    environment = {name: value for name, value in os.environ.items() if not name.startswith("SLUG")}

    failures = []
    with tempfile.TemporaryDirectory(prefix="slug-release-smoke-") as scratch:
        version = run([slug, "--version"], scratch, environment)
        print(f"--version: {version.stdout.strip()}")
        if version.returncode != 0 or (options.version and f" {options.version}" not in version.stdout):
            failures.append(f"--version gave {version.stdout!r} (exit {version.returncode})")
        for module, (program, expected) in CHECKS.items():
            if module in options.skip:
                print(f"skip {module}")
                continue
            path = os.path.join(scratch, f"{module.replace('.', '-')}.slug")
            with open(path, "w", newline="\n") as file:
                file.write(program)
            result = run([slug, path], scratch, environment)
            if result.returncode == 0 and result.stdout.strip() == expected:
                print(f"ok   {module}")
            else:
                print(f"FAIL {module} (exit {result.returncode})")
                failures.append(
                    f"{module}: exit {result.returncode}\n  stdout: {result.stdout.strip()}\n"
                    f"  stderr: {result.stderr.strip()}"
                )
    if failures:
        sys.exit("release smoke test failed:\n" + "\n".join(failures))


if __name__ == "__main__":
    main()
