# Setup and project structure

## Runner and library

Use the existing executable at `$SLUG_HOME/target/release/slug` and its matching
library installation. `SLUG_HOME` points to the directory containing `lib/`,
`clutch/`, and `target/release/slug`; it is not the application directory.
Check the configured installation before running application commands:

```sh
: "${SLUG_HOME:?SLUG_HOME must point to the existing Slug installation}"
test -x "$SLUG_HOME/target/release/slug"
"$SLUG_HOME/target/release/slug" --help
```

Do not download, clone, install, build, rebuild, or substitute another Slug
runtime without explicit user confirmation. Do not fall back to a `slug` found
on `PATH` or a debug build. If `SLUG_HOME` is unset, the executable is missing
or unusable, or a required clutch is unavailable, report the setup issue and
ask the user to confirm how to proceed. Do not rebuild or stage native clutches
without confirmation either.

The public [language repository](https://github.com/sluglang/slug-rs) is a
reference for documentation and API discovery; linking it does not authorize
a local checkout or build. Run application commands from the application
directory. A restricted or embedded host may expose fewer modules and
concurrency features than the full desktop runner.

## Application layout

```text
application/
    AGENTS.md
    slug-lang/
    slug.toml
    src/
        main.slug
        app/
            greeting.slug
    tests/
        all.slug
    assets/
    clutch/
```

An empty `slug.toml` is enough to mark a project. `src/` is its module root,
so `src/app/greeting.slug` is imported as `import("app.greeting")`.
Imports search the importing file's directory, then `src/`, then the library
root, then clutch providers. The project root itself is not a module search
directory. Avoid names that accidentally shadow library modules.

The runner discovers the nearest `slug.toml` at or above the entry file.
`assets/` holds files embedded with `asset("name")`; `clutch/` is optional for
project providers. These directory names are fixed. Create only what you need.

## Run and test

```sh
"$SLUG_HOME/target/release/slug" src/main.slug
"$SLUG_HOME/target/release/slug" tests/all.slug
```

The runner evaluates the entry module, then invokes a local top-level `main`.
Use one eligible form: `fn()`, `fn(args:list)`, or `fn(args:map)`. The latter
forms receive raw program arguments or parsed `{options, positional}` values.
An imported module's `main` is not invoked automatically.

A numeric `main` result becomes the process exit code and must be an integer
from 0 through 255. End a successful entrypoint with `nil`, an output call,
or an intentional exit status; do not accidentally return a numeric total.

Application tests can be ordinary programs importing `slug.test` and the
application modules. A failed assertion throws and makes the run fail. Keep
many assertions in one process. The installed `slug test` tool is for clutch
suites; it is not a general application test-directory discovery command.

### Test functions with `@test`

Use `@test` for an ordinary zero-argument test function. Put assertions in its
body; it passes when it does not throw or return `false`:

```slug
val test = import("slug.test")
val greeting = import("app.greeting")

@test
export val greetingTest = fn() {
    test.assertEqual(greeting.greet("Slug"), "Hello, Slug!")
}
```

The [module example](examples/README.md) puts these functions in
`tests/checks.slug`. Use the installed `slug.test.runner` from a small test
entry, such as `tests/all.slug`:

```slug
val runner = import("slug.test.runner")

val main = fn():num {
    runner.run(["app.greeting", "checks"], fn(name:str):list { describe(name) })
}
```

Keep the discovery callback in the application entry: `describe` resolves
module names from the callback's defining module, so `checks` can resolve to
`tests/checks.slug`. Export tests for discovery without unexported-inspection
permission. The runner continues after failures, reports assertion failures
and unexpected errors separately, prints totals, and returns 0 or 1. Return
its status from `main`; ignoring it can make a failing suite exit successfully.
Pass `true` as the third argument to also print passing tests. Empty suites
and malformed tags fail. There is no directory scan: list the modules to test.

### Examples beside functions with `@testWith`

Attach input/result examples directly to a function:

```slug
@testWith(["Slug"], "Hello, Slug!", [""], "Hello, !")
export val greet = fn(name:str):str { "Hello, $name!" }
```

Each pair is a list of positional arguments followed by the expected return
value. Supply complete pairs; use `[]` for a zero-argument call. The runner
calls the function with each argument list and compares its result with the
expected value. A thrown error also fails that example.

Both `@test` and `@testWith` are metadata: importing a module or running its
source does not execute tagged tests automatically. The installed runner above
discovers and runs both. If writing custom tooling, `test.examples` reads
argument/result pairs and `test.check` executes one; `check` returns `nil` or
a failure record rather than failing the process itself.

For installed clutch suites, the `slug test` runner discovers `@testWith`
examples and zero-argument `@test` functions. A `@test` function passes when
it does not throw or return `false`. Follow that clutch's documented runner
command and test-file registration.

## Configuration and diagnostics

Read configuration with `cfg("app.greeting", "Hello")`. Dotted keys are
absolute; an undotted key is prefixed by the executing module's name.
Precedence, lowest first: library TOML, project `slug.toml`, `SLUG__...`
environment variables, then options after the program path. For example:

```toml
[app]
greeting = "Welcome"
```

```sh
"$SLUG_HOME/target/release/slug" src/main.slug --app.greeting=Hello
```

`cfg`'s fallback controls basic string conversion, not validation. Conversion
can fail and leave a string; validate settings before depending on their type.
See the runnable [configuration example](examples/README.md).

For machine-readable runner errors:

```sh
"$SLUG_HOME/target/release/slug" --diagnostic-format=json src/main.slug
```

Runner flags go before the program path; program options go after it. Resolve
parse and semantic diagnostics before runtime problems. Preserve useful source
locations and errors instead of hiding failures behind fallback values.
See [troubleshooting](troubleshooting.md) for common failures and concrete checks.
