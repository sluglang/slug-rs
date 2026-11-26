# Find and use library APIs

Only host builtins are implicit. Import library modules explicitly. Libraries
are supplied as clutches, often including platform-specific native code.
Setting `SLUG_HOME` alone does not build missing native libraries.
If a required library is missing, report it and obtain explicit user
confirmation before staging native clutches or changing the installation.

## Where to look

Use the installed sources or the matching revision of the
[public repository](https://github.com/sluglang/slug-rs). Its
[`clutch/manifest.toml`](https://github.com/sluglang/slug-rs/blob/main/clutch/manifest.toml)
maps module names to clutches; each clutch's `clutch.toml` maps modules to
source files. Read the exported declarations, their documentation, and nearby
tests. Keep the signature's argument order, labels, defaults, nilability, and
return type. Native `...Raw` declarations inside a module are implementation
details unless explicitly exported.

| Operation | Import | Source location within upstream |
|---|---|---|
| Collections | `slug.std` | `clutch/slug.core.clutch/modules/std.slug` |
| Strings and bytes | `slug.string`, `slug.bytes` | `clutch/slug.core.clutch/modules/string.slug`, `bytes.slug` |
| JSON and TOML | `slug.fmt.json`, `slug.fmt.toml` | `clutch/slug.core.clutch/modules/json.slug`, `toml.slug` |
| Assertions | `slug.test` | `clutch/slug.core.clutch/modules/test.slug` |
| Application test runner | `slug.test.runner` | `lib/slug/test/runner.slug` |
| Time and random values | `slug.time`, `slug.random` | `clutch/slug.core.clutch/modules/time.slug`, `random.slug` |
| Input and signals | `slug.io.stdin`, `slug.os.signal` | `clutch/slug.core.clutch/modules/stdin.slug`, `signal.slug` |
| Channels and tasks | `slug.channel` | `lib/slug/channel.slug` |
| Files | `slug.io.fs` | `clutch/slug.io.fs.clutch/modules/fs.slug` |
| TCP and HTTP | `slug.net.tcp`, `slug.net.http` | Corresponding clutch's `modules/tcp.slug` or `modules/http.slug` |
| SQLite | `slug.db.sqlite` | `clutch/slug.db.sqlite.clutch/clutch.toml` and its module sources |

The core clutch also supplies `slug.math`. Cryptography and secure randomness
have separate providers. Availability depends on the installed host and clutches;
do not assume every desktop module exists on an embedded host.

## API reference

The hand-maintained reference keeps declared parameter labels, defaults,
generic types, and return annotations alongside behavioral notes:

| Reference | Coverage |
|---|---|
| [Core APIs](api-reference.md) | Builtins, collections, strings, bytes, math, JSON/TOML, tests, channels, clocks, seeded randomness, stdin, and signals. |
| [Desktop APIs](api-desktop.md) | Files, TCP, HTTP serving, SQLite, secure randomness, hashing/signature verification, templates, and Markdown. |

This is a source snapshot, not a runtime compatibility promise. Consult the
installed declarations for differences or APIs beyond its scope. There is no
automatic reference synchronization yet.

`json.parse` may produce a scalar, list, map, or `nil`. Narrow its result with
`match` before treating it as a map or list. Do not guess convenience operations
such as filesystem `readText`, HTTP `get`, or a universal collection `.push`;
look up what the installed module actually exports.

For filesystem work, prefer the explicit open/read/write/close lifecycle shown
in the [file example](examples/README.md). `fs.close` closes files; the builtin
`close` closes channels.

Use `@test` for ordinary zero-argument test functions, and
`@testWith([arguments...], expected, ...)` for examples beside a function.
See [testing guidance](getting-started.md#test-functions-with-test) for tag
execution and the distinction between application tests and clutch suites.

`describe("module.name")` can inspect exported runtime declarations, values,
documentation, and tags, but it initializes the module. Use it only when those
effects are appropriate; read source for complete type signatures. Type-only
exports do not become runtime values. Inspecting unexported declarations needs
an explicit runner permission and is unnecessary for normal API discovery.
