# Runnable examples

Each directory is a separate project with its own `slug.toml`. Run these
commands from this `examples/` directory with `SLUG_HOME` pointing to the
existing installation and its `target/release/slug` executable. See
[setup](../getting-started.md) first; do not download or build a replacement
without explicit user confirmation.
The module example uses the core clutch; the file example also uses the
filesystem clutch. All successful commands below exit with status 0.

## A first program

```sh
"$SLUG_HOME/target/release/slug" hello/src/main.slug
```

Expected output: `Hello, Slug!`.

## Logic in a module, tested in one process

```sh
"$SLUG_HOME/target/release/slug" modules/src/main.slug
"$SLUG_HOME/target/release/slug" modules/tests/all.slug
```

Expected output: `Hello, Slug!`, then
`Tests run: 3, Passed: 3, Failures: 0, Errors: 0` from the test program.
The greeting function carries `@testWith` input/result pairs, and
`modules/tests/checks.slug` contains an ordinary `@test` function with
assertions. The test entry imports the installed `slug.test.runner` and supplies
a discovery callback using `describe`. It runs all tests even after failures,
prints totals, and returns a failing exit status for failures, errors, malformed
tags, or an empty suite. Both tags need runner wiring; imports alone do not
execute them. The installation must contain `lib/slug/test/runner.slug`.

## Configuration and file cleanup

```sh
cd files
"$SLUG_HOME/target/release/slug" src/main.slug
"$SLUG_HOME/target/release/slug" src/main.slug --example.lines.path=assets/lines.txt
```

Both print `2`. The path is relative to the shell's working directory, so
these commands run from the file project itself. This example opens the file
at runtime; it does not embed it with `asset`. From `files/`, check failure
propagation too:

```sh
"$SLUG_HOME/target/release/slug" src/main.slug --example.lines.path=assets/missing.txt
```

Expect a runtime error and exit status 1. No handler suppresses the open failure.
