# Write a project in Slug

This is a copied application-writing guide. Read
[getting-started.md](getting-started.md) for setup and project layout, then
[language.md](language.md) before writing code. Look up capabilities in
[library.md](library.md), read signatures in the [core API reference](api-reference.md)
or [desktop API reference](api-desktop.md), and use the runnable [examples](examples/README.md)
as starting points. When something fails, consult the
[troubleshooting tables](troubleshooting.md).

The application's root `AGENTS.md` owns its commands, architecture, and local
conventions. Keep additions there or in application docs so this directory can
be replaced when updating. [VERSION](VERSION) identifies this package and its
implementation baseline. Confirm which runtime the application actually uses.

Use the existing `$SLUG_HOME/target/release/slug` executable for every Slug
command. Downloading or building a runtime, substituting another executable,
or staging native clutches requires explicit user confirmation. If the
installation is unavailable, report the issue and ask how to proceed.

## Upstream and authority

Slug's public reference implementation is
[sluglang/slug-rs](https://github.com/sluglang/slug-rs). The earlier Go
implementation is legacy. Use these upstream sources when the compact guide
does not answer a question:

| Need | Source |
|---|---|
| Implemented features | [Implementation support matrix](https://github.com/sluglang/slug-rs/blob/main/docs/generated/language-support.md) |
| Language rules | [Language specification](https://github.com/sluglang/slug-rs/blob/main/docs/language/language-specification.md) and [grammar](https://github.com/sluglang/slug-rs/blob/main/docs/language/slug.ebnf) |
| Observable runtime rules | [Runtime requirements](https://github.com/sluglang/slug-rs/blob/main/docs/language/runtime-requirements.md) |
| Focused explanations | [Language documentation index](https://github.com/sluglang/slug-rs/blob/main/docs/language/README.md) |
| Library contracts | [Clutch sources](https://github.com/sluglang/slug-rs/tree/main/clutch) and [host builtins](https://github.com/sluglang/slug-rs/blob/main/lib/slug/builtin.slug) |
| Packaging and resolution | [Clutches](https://github.com/sluglang/slug-rs/blob/main/docs/reference/clutches.md) |

The specification describes the target language; the support matrix records
what this implementation supports. Neither a roadmap nor a specification
example alone proves availability. The guide summarizes those contracts and
does not redefine them. Report disagreements instead of silently choosing a
new behavior.

These links follow upstream `main`, which can be newer than the installed
runtime. For baseline-specific reading, replace `main` in a GitHub link with
the commit in `VERSION`; for another installed revision, use that revision.
Prefer the installed library's source signatures when checking available APIs.

## A productive workflow

1. Confirm `$SLUG_HOME/target/release/slug` and its installed clutches work
   together. Follow the setup guidance if the installation is unavailable.
2. Inspect the application's modules and tests; select the smallest change.
3. Verify any unfamiliar syntax or callable signature before using it.
4. Keep logic separate from entrypoint input/output and test it in Slug.
5. Run the application checks. Include the commands and results in handoff.

Use small runnable probes when behavior is unclear. Missing modules and native
libraries are setup failures to investigate; changing application semantics
does not resolve them.
