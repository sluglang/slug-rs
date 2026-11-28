# Releasing

Releases publish the Slug CLI as one archive per platform, built by
`.github/workflows/release.yml`.

## What a release contains

| Platform | Runner | Archive |
| --- | --- | --- |
| `macos-aarch64` | `macos-latest` (macOS 12 and later) | `slug-<version>-macos-aarch64.tar.gz` |
| `linux-x86_64` | `ubuntu-22.04` (glibc 2.35 and later) | `slug-<version>-linux-x86_64.tar.gz` |
| `windows-x86_64` | `windows-latest` | `slug-<version>-windows-x86_64.zip` |

Each archive is an installation laid out as a Slug home:

```
slug-<version>-<platform>/
  bin/slug, bin/slug-repl
  lib/
  clutch/manifest.toml, clutch/<name>.clutch/
  LICENSE.md, README.md
```

`slug` uses the directory above its `bin/` as its home when `$SLUG_HOME` is
unset, so an unpacked archive runs as it is, also through a symlink to
`bin/slug`. Each clutch ships its modules and assets with the native library
staged for that platform, not its tests or vendored sources.
`scripts/package-release.py` assembles the archive.

`slug` is the profile-guided build (`scripts/pgo-build.py`), and `slug
--version` names its commit and CI build: `slug-vm 0.2.0 (a1b2c3d, build 57)`.
`SHA256SUMS` lists every archive's hash.

Known gaps: the Windows archive has no native `slug.db.sqlite` or
`slug.arcade`, and the binaries are not yet signed, so macOS quarantines a
downloaded archive (`xattr -dr com.apple.quarantine slug-<version>-<platform>`
clears it).

## Cutting a release

1. On a branch, set `[workspace.package] version` in `Cargo.toml`, rename
   `## Unreleased` in `changelog.md` to `## <version> (<date>)`, and add a
   fresh `## Unreleased` above it. Merge it to `master` with CI green.
2. Tag the merge commit and push the tag:

   ```sh
   git tag v<version> && git push origin v<version>
   ```

3. The workflow checks the tag against the workspace version and the
   changelog (`scripts/release-notes.py`), builds and checks each archive,
   and drafts a release with the changelog section as its notes. A tag with
   a pre-release suffix, such as `v0.2.0-rc.1`, drafts a pre-release.
4. Review the draft on GitHub and publish it.

Each archive is checked unpacked, run from outside the checkout with
`$SLUG_HOME` unset: `scripts/release-smoke.py` imports each installed module
and calls into each native library.

Running the workflow by hand (Actions → Release → Run workflow), or on a pull
request that changes the release tooling, is a dry run: it builds and checks
the archives, keeps them as workflow artifacts named `<version>-dev.<run>`,
and drafts nothing.
