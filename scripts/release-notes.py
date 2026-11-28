#!/usr/bin/env python3
"""Check a release tag against the checkout and print its release notes.

    python3 -I scripts/release-notes.py TAG [--repository OWNER/NAME]

TAG is `v<version>`, and `<version>` must be the workspace version in
Cargo.toml. The notes are the changelog section headed `## <version>`
(anything after the version on that line, such as a date, is allowed), which
the release commit makes by renaming `## Unreleased`. A section longer than a
GitHub release body allows is replaced by a link to the changelog at the tag.
"""

import argparse
import os
import re
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
# GitHub refuses release bodies over 125,000 characters; leave room for the link.
LIMIT = 120_000


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("tag")
    parser.add_argument("--repository", default="babyman/slug-rs")
    options = parser.parse_args()

    match = re.fullmatch(r"v(\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?)", options.tag)
    if not match:
        sys.exit(f"tag {options.tag!r} is not v<major>.<minor>.<patch>[-prerelease]")
    version = match.group(1)
    with open(os.path.join(ROOT, "Cargo.toml"), encoding="utf-8") as file:
        # The version line of `[workspace.package]`; Python 3.9 has no tomllib.
        found = re.search(r'^\[workspace\.package\][^\[]*?^version = "([^"]+)"', file.read(), re.MULTILINE | re.DOTALL)
    if not found:
        sys.exit("Cargo.toml has no [workspace.package] version")
    workspace = found.group(1)
    if workspace != version:
        sys.exit(f"tag {options.tag} does not match the workspace version {workspace}")

    with open(os.path.join(ROOT, "changelog.md"), encoding="utf-8") as file:
        text = file.read()
    heading = re.search(rf"^## {re.escape(version)}(?=[ \t]|$).*$", text, re.MULTILINE)
    if not heading:
        sys.exit(f"changelog.md has no `## {version}` section; rename `## Unreleased` for the release")
    following = re.search(r"^## ", text[heading.end():], re.MULTILINE)
    end = heading.end() + following.start() if following else len(text)
    notes = text[heading.end():end].strip()
    link = f"https://github.com/{options.repository}/blob/{options.tag}/changelog.md"
    if len(notes) > LIMIT:
        notes = f"This release's changes are too long for a release page; see [changelog.md]({link})."
    print(notes)


if __name__ == "__main__":
    main()
