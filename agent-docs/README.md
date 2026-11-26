# Slug application agent package

Draft documentation to copy into projects written in Slug. The distributable
directory is [slug-lang/](slug-lang/README.md); the application's instructions
start from [AGENTS.md.template](AGENTS.md.template).

## Copy into a project

From this repository, replacing the destination with your application's path:

```sh
cp -R agent-docs/slug-lang /path/to/application/slug-lang
```

For a new application with no `AGENTS.md`, copy `AGENTS.md.template` to its
root as `AGENTS.md`, then fill in the project details. For an existing
application, merge the relevant instructions into its existing file.

The application owns its root `AGENTS.md`. Treat `slug-lang/` as an upstream
snapshot and keep local conventions outside it. When updating, review the
incoming package, compare its `VERSION` with the installed runtime revision,
and replace the old directory in full. Copying over an existing directory
alone can leave files that upstream removed. Run the examples and application
tests before accepting an update.

## Maintaining the draft

Keep the core guide small and link to detailed specifications and actual
module sources. Update `slug-lang/VERSION` when changing the package, and
record the implementation revision used for verification. Links to upstream
`main` are discovery links; the recorded revision identifies the baseline.

Validate all Slug fences in `slug-lang/language.md` as standalone programs.
Check API-reference signatures manually against the corresponding exported
source declarations, and run their explicitly runnable Slug examples. Signature
blocks marked `text` omit bodies and are not programs.
Run the commands in `slug-lang/examples/README.md` with the existing
`$SLUG_HOME/target/release/slug` and matching installed clutches. Downloading,
building, or substituting a runtime, or staging native clutches, requires
explicit user confirmation. Check expected output and exit status,
including the missing-file case. Review links and run `git diff --check`.
These checks are currently manual; the draft adds no CI integration.

This package teaches applications. Compiler contribution instructions,
speculative language features, and native shim implementation belong in the
language repository's existing documentation.
