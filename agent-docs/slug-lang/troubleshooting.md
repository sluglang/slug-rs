# Troubleshooting a Slug application

Start with the smallest failing application command. Run it from the documented
working directory with the existing `$SLUG_HOME/target/release/slug`. Preserve
its arguments, exit status, and diagnostic location. Fix the first relevant
source error before investigating downstream behavior.

Do not download, build, substitute, or upgrade the runtime, or stage native
clutches, without explicit user confirmation. If the installed environment
cannot support the task, report what is missing and ask how to proceed.

## Setup, imports, and files

| Symptom | What to check | Next step |
|---|---|---|
| `SLUG_HOME` is unset, or the runner is missing or not executable | Check the existing environment and `test -x "$SLUG_HOME/target/release/slug"`. | Report the missing setting or executable and ask for the intended installation. Do not guess a location or fall back to `PATH` or a debug build. |
| The shell cannot start the release runner | Check the exact executable path and shell error; confirm its platform matches the host. | Report the launch failure and obtain confirmation before changing the installation. |
| The entry program cannot be read (`entry_read`) | Check `pwd` and the entry path supplied to the runner. | Run from the application directory or supply the correct entry path. |
| An import cannot be found (`module_not_found`) | Check spelling, case, and the nearest `slug.toml`. `app.greeting` maps to `src/app/greeting.slug` in a project. | Put application modules under `src/`; inspect the installed library and clutch manifests for library modules. The project root itself is not an import search directory. |
| A module fails to load (`module`) | Read its nested diagnostic `cause`; the imported source may have a parse or type error. | Fix the reported module source or API use. Do not treat every module error as a missing installation. |
| A clutch cannot activate, or a native library cannot load (`clutch`, sometimes `module`) | Inspect the named clutch's installed `clutch.toml`, platform library, and diagnostic. | Report the missing library, incompatible platform, or dependency. Obtain confirmation before staging clutches or changing the installation. |
| An exported name or callable is unavailable | Read the installed module's exported declarations and exact signature. Compare its revision with this package's `VERSION`. | Use that installed API or report the capability gap. Upstream `main` may describe a newer API. |
| A file exists but opening it fails | Check the shell's working directory, the actual configured path, and access permissions. | Resolve runtime relative paths from the working directory. They are not automatically relative to the entry source or `slug.toml`. |
| `asset("name")` fails (`invalid_asset`) | Check the owning project's or clutch's `assets/` directory, exact filename case, and literal asset name. | Use a string literal naming a file within `assets/`, without `..`. `asset` embeds bytes at compile time; use `slug.io.fs` for a configurable runtime file. |

## Source and runtime behavior

| Symptom | What to check | Next step |
|---|---|---|
| Parse error (`expected_syntax`, `invalid_argument_order`) | Read the first source location. Check commas between collection entries, `=` for named arguments, and parenthesized `if` conditions. | Compare the construct with [language.md](language.md). Repetition uses collection functions or `recur`, rather than `for` or `while`. |
| A declaration or call fails type checking (`type_mismatch`, `no_matching_overload`, `annotation_mismatch`) | Inspect parameter order, annotations, nilability, and the function's final expression. | Narrow unions before using their values. An annotation checks a value; it does not convert it. Read the installed signature before changing caller types. |
| A JSON value cannot be used as a map or list | `json.parse` returns the `Json` union, which includes scalars and `nil`. | Narrow with `match`, using a map or list type constraint and an explicit alternative for other shapes. |
| A collection appears unchanged after an update | Check whether the returned value from `:+`, `+`, `-`, or `copy` was retained. | Bind the new value or assign it to a `var`. These operations do not mutate the original collection. |
| An assignment is rejected (`immutable_assignment`, `lost_update`) | Check whether the target is a `val`; for `lost_update`, inspect calls on the right side that can assign the same `var`. | Use `var` only when reassignment is intended. For a lost update, evaluate the call first, then read and update the binding. |
| `recur` is rejected (`invalid_control_flow`) | Check whether further work follows it, including arithmetic around its result. | Move accumulated work into arguments so `recur(...)` is the final operation of the current function branch. |
| A match is incomplete or a case is unreachable | Check `nil` and other members of the subject's union; check earlier catch-all patterns. | Cover the missing alternatives and put specific cases before general ones. A plain name binds; `^name` compares with an existing binding. |
| A function named `main` is rejected or the expected entrypoint does not run | Check that it is local and top-level, with exactly one eligible signature: `fn()`, `fn(args:list)`, or `fn(args:map)`. | Correct its signature or call the ordinary function explicitly from the eligible entrypoint. Imported `main` functions are not invoked automatically. |
| Output looks successful but the process exits nonzero | Inspect the final value returned by `main`; a number becomes an exit status. | End successful work with `nil`, an output call, or an intentional status. Fractional or out-of-range numbers produce `exit_status`. |
| A failure disappears after logging or cleanup | Check `defer onerror`: completing its handler without throwing recovers from the error. | End an observation-only handler with `throw error`. Use ordinary `defer` for cleanup that must run on either outcome. |
| Configuration is ignored or has an unexpected type | Check absolute versus module-local keys and precedence: library TOML, project TOML, environment, then program options. | Use a dotted key when sharing settings across modules. Options go after the entry path. Validate the returned value: failed fallback-based conversion can leave a string. |
| A task or channel operation waits indefinitely | Check for a receiver, sender, task dependency, or channel closure that can never occur. Normal nursery exit joins its children. | Reduce the example and inspect both ends of each dependency. A zero-capacity channel needs a matching receiver; a drained closed channel receives `nil`. |
| A fork or concurrency operation is rejected (`fork`, `fork_write`, `capability_unavailable`) | Check shared writes, sendability, native thread safety, module initialization, and the installed host's concurrency capabilities. | Initialize dependencies before forking and return or send supported values. Do not replace the runner or rebuild with different features without confirmation. |

## Tests

| Symptom | What to check | Next step |
|---|---|---|
| `@test` or `@testWith` never runs | Tags are metadata; imports alone do not execute tests. Check which modules and declarations the test entry discovers. | Use `slug.test.runner` with an application-defined discovery callback, as in the [module example](examples/README.md). |
| A tagged test is absent from `describe` | By default, only exported declarations are returned. | Export the intended test functions and inspect the correct test module. Do not add unexported-inspection permission just to work around discovery. |
| A `@test` function fails without throwing | Check whether its final expression returns `false`. | Make the boolean expectation explicit with an assertion. A `@test` must take zero arguments and pass without throwing or returning `false`. |
| A `@testWith` example is skipped or fails unexpectedly | Check complete argument-list/result pairs, positional argument order, and the expected return value. | Use `[]` for zero arguments and keep every pair complete. `test.examples` produces no examples for an odd tag argument count; validate tag shape rather than counting that as a pass. |
| A failed check still produces a successful test-process exit | `test.runTest` and `test.check` return failure records instead of throwing them. | Assert that each result is `nil`, or collect failures and explicitly fail at the end. Count executed tests so empty discovery cannot silently pass. |

## Capture useful diagnostics

From the application directory, substitute its actual entry and arguments:

```sh
"$SLUG_HOME/target/release/slug" --diagnostic-format=json src/main.slug
```

The runner writes fatal JSON diagnostics to standard error. Program output
remains separate. Match diagnostics by `code`, not message wording; inspect
nested `cause` diagnostics when imports or error unwinding wrap a failure.
See the upstream [diagnostic codes](https://github.com/sluglang/slug-rs/blob/main/docs/reference/diagnostic-codes.md)
and [diagnostic format](https://github.com/sluglang/slug-rs/blob/main/docs/reference/diagnostics.md).
Use the installed revision when it differs from upstream `main`.

For a handoff, record the runner path, working directory, command and arguments,
exit status, and relevant diagnostic code and source location. Include the
smallest reproducing source when needed. Omit secret configuration values.
