# Core API reference

This is a hand-maintained snapshot of common application APIs, read from the
reference implementation's public declarations. See [VERSION](VERSION) for the
source baseline, [desktop APIs](api-desktop.md) for files, networking, databases,
and text rendering, and [library discovery](library.md) for other modules.
The application runner below is an addition to that baseline; installations
need `lib/slug/test/runner.slug` to use it. Game, arcade, GPIO, and Markdown
parser internals are outside this reference.
No generation or synchronization automation is included.

Use the existing `$SLUG_HOME/target/release/slug`. Compare unfamiliar APIs with
the installed source before relying on them: an older runner or library may
not implement this snapshot. Do not download, build, substitute a runtime, or
stage clutches without explicit user confirmation.

## Reading signatures

Signature blocks use Slug declaration notation with bodies omitted; they are
reference material, not runnable programs. Names on the left are module exports.
For example, after `val std = import("slug.std")`, call `std.map(values, f)`.
Parameter names are also named-argument labels. Defaults, variadic markers,
generic parameters, and return annotations are preserved. When a declaration
has no return annotation, the notes explain its behavior.

`fn<R, P1, ...>` puts the result type first. `T|nil` can be absent; `any`
excludes `nil`, while `any|nil` accepts every value. Annotations do not convert
values. Resource and exported type names are local to their module; qualify
them through an imported module binding when needed. Source links follow
upstream `main`; replace it with the revision in `VERSION` for baseline reading.

## Implicit builtins

[Builtin contract](https://github.com/sluglang/slug-rs/blob/main/lib/slug/builtin.slug).
These names are already available; do not import `slug.builtin`.

```text
print = fn(...values):nil
println = fn(...values):nil
len = fn(value:str|bytes|list|map):num
keys = fn<K, V>(m:map<K, V>):list<K>
chan = fn<T>(capacity:num = 0):chan<T>
close = fn<T>(channel:chan<T>):nil
```

`print` and `println` write and flush space-separated values; only `println`
adds a newline. `len` counts Unicode scalars for strings, bytes for bytes, and
elements for lists/maps. Map key order is unspecified. A channel capacity of
zero is a rendezvous; capacity must be a non-negative integer. Closing is
idempotent; receives drain queued values before returning `nil`.

`Error` is an implicit schema with required `msg:str`, `type:str = "Error"`,
and `code`, `data`, and `cause` defaulting to `nil`:
`Error {type: "app.input", msg: "name is required"}`.

The runtime also supplies these operations, described in the
[language specification](https://github.com/sluglang/slug-rs/blob/main/docs/language/language-specification.md):

| Operation | Result and restrictions |
|---|---|
| `cfg(key, default)` | Immutable configuration value or fallback; `key` is a string. Fallback-based string conversion is not validation. |
| `int(value)` | Truncates a finite number toward zero. |
| `stacktrace(error)` | A string for the active error; use inside its error handler. |
| `describe(module, unexported = false)` | List of declaration metadata; initializes the named module. Unexported inspection requires permission. |
| `import("module.name")` | Module exports; module names and resolution are host-controlled. |
| `asset("name")` | Compile-time embedded bytes from the owning project's or clutch's assets; requires a literal name. |

## Collections — `slug.std`

[Source](https://github.com/sluglang/slug-rs/blob/main/clutch/slug.core.clutch/modules/std.slug).

```text
get = fn<K, V>(m:map<K, V>, k:K):V|nil
put = fn<K, V>(m:map<K, V>, k:K, v:V):map<K, V>
remove = fn<K, V>(m:map<K, V>, k:any|nil):map<K, V>
map = fn<T, U>(vs:list<T>, f:fn<U, T>, acc:list<U> = []):list<U>
each = fn<T>(vs:list<T>, f:fn<any|nil, T>):nil
filter = fn<T>(vs:list<T>, f:fn<bool, T>, acc:list<T> = []):list<T>
reduce = fn<T, A>(vs:list<T>, v:A, f:fn<A, A, T>):A
flatMap = fn<T>(vs:list<T>, f:fn<any|nil, T>):list
compact = fn<T>(xs:list<T|nil>, acc:list<T> = []):list<T>
find = fn<T>(xs:list<T>, f:fn<bool, T>):T|nil
contains = fn(xs:list, v:any|nil):bool
any = fn<T>(xs:list<T>, f:fn<bool, T>):bool
all = fn<T>(xs:list<T>, f:fn<bool, T>):bool
range = fn(start:num, end:num):list<num>
range = fn(end:num):list<num>
times = fn(n:num, f:fn<any|nil, num>):nil
```

Collection changes produce new values. `map` collects callback results; `each`
discards them. `reduce` calls `f(accumulator, element)` in list order. `flatMap`
concatenates returned lists and treats a scalar as one element. `compact`
removes `nil`. `find` returns the first matching element or `nil`.
`any([], predicate)` is false; `all([], predicate)` is true.
`range(end)` starts at zero; both ranges exclude `end` and are empty if the
end is no greater than the start. Use integer bounds for ranges and `times`.

The module also exports a result enum and extraction helpers:

```text
enum Result<T, E> { Ok { value: T }; Error { error: E } }
or = fn<T, E>(result:Result<T, E>, fallback:T):T
orElse = fn<T, E>(result:Result<T, E>, recover:fn<T, E>):T
orThrow = fn<T, E>(result:Result<T, E>, toError:fn<any, E>):T
```

`or` uses an already-evaluated fallback on error; `orElse` calls its recovery
function only on error; `orThrow` throws the converted error. These helpers
consume a `Result`, not a thrown exception. Many existing APIs still throw or
return `nil`; check each API rather than assuming it returns `Result`.

## Strings — `slug.string`

[Source](https://github.com/sluglang/slug-rs/blob/main/clutch/slug.core.clutch/modules/string.slug).

```text
ord = fn(character:str):num
chr = fn(code:num):str
fromBytes = fn(data:bytes):str
toBytes = fn(text:str):bytes
sort = fn(strings:list<str>):list<str>
join = fn(strings:list<str>, separator:str = ""):str
toInt = fn(text:str):num|nil
chars = fn(text:str):list<str>
split = fn(text:str, separator:str):list<str>
isDigit = fn(c:str):bool
isLetter = fn(c:str):bool
isSpace = fn(c:str):bool
trim = fn(text:str):str
lower = fn(text:str):str
upper = fn(text:str):str
```

`ord` requires one Unicode scalar; `chr` requires a valid scalar code point.
`toBytes` encodes UTF-8; invalid UTF-8 passed to `fromBytes` throws
`string.utf8`. `toInt` accepts signed decimal integers and returns `nil` for
invalid or out-of-range input, including surrounding spaces or fractions.
`split` requires a non-empty separator and preserves empty pieces.
`chars` returns Unicode scalar strings. `isDigit`, `isLetter`, `lower`, and
`upper` use ASCII rules; `trim` removes spaces, tabs, CR, and LF at both ends.
`sort` returns a stable ascending copy using Slug string ordering.

## Bytes — `slug.bytes`

[Source](https://github.com/sluglang/slug-rs/blob/main/clutch/slug.core.clutch/modules/bytes.slug).

```text
find = fn(data:bytes, needle:bytes, from:num = 0):num|nil
findAny = fn(data:bytes, set:bytes, from:num = 0):num|nil
toHex = fn(data:bytes):str
fromHex = fn(text:str):bytes
```

Search offsets count bytes. A missing match is `nil`; `from` outside zero
through the data length throws `bytes.range`. `toHex` produces lowercase hex;
`fromHex` accepts either case but requires complete pairs without whitespace
or a `0x` prefix, otherwise it throws `bytes.hex`.

## Numbers — `slug.math`

[Source](https://github.com/sluglang/slug-rs/blob/main/clutch/slug.core.clutch/modules/math.slug).

```text
add = fn(left:num, right:num):num
div = fn(left:num, right:num):num
sqrt = fn(value:num):num
abs = fn(value:num):num
min = fn(left:num, right:num):num
max = fn(left:num, right:num):num
```

`div` truncates the quotient toward zero and preserves exact integer division
for large integers; `/` can produce a fractional quotient. Division by zero,
unrepresentable numeric results, and invalid square roots fail with checked
errors. See the source for each operation's range checks.

## JSON and TOML

[`slug.fmt.json` source](https://github.com/sluglang/slug-rs/blob/main/clutch/slug.core.clutch/modules/json.slug):

```text
type Json = nil|bool|num|str|list<Json>|map<str, Json>
DEFAULT_MAX_DEPTH = 1000
parse = fn(text:str, options:map = {}):Json
parseStrict = fn(text:str, options:map = {}):Json
stringify = fn(value:any|nil):str
```

Narrow `Json` before using it as a list or map. `parseStrict` also rejects
duplicate object keys at any depth. Parsing accepts `options.maxDepth`, a
positive integer; unknown options or invalid depth settings throw
`json.options`. Invalid syntax throws `json.parse`; excessive depth throws
`json.limit`. `stringify` produces compact JSON with map keys sorted, and
throws `json.type` for unsupported values, including bytes, structs, enums,
functions, or maps with non-string keys.

[`slug.fmt.toml` source](https://github.com/sluglang/slug-rs/blob/main/clutch/slug.core.clutch/modules/toml.slug):

```text
type Toml = bool|num|str|list<Toml>|map<str, Toml>
parse = fn(text:str):map<str, Toml>
```

This parser supports the TOML subset used by manifests: strings, decimal
integers, booleans, arrays, inline tables, dotted keys, and table headers.
Floats, dates/times, multiline strings, non-decimal integers, and arrays of
tables are unsupported. Invalid or unsupported input throws `toml.parse`
with one-based line and column in its data. There is no exported TOML writer.

## Tests — `slug.test`

[Source](https://github.com/sluglang/slug-rs/blob/main/clutch/slug.core.clutch/modules/test.slug).

```text
type Failure = {msg: str, expected: any|nil, actual: any|nil, error: any|nil}
fail = fn(message:str, expected:any|nil = nil, actual:any|nil = nil)
assertEqual = fn(actual:any|nil, expected:any|nil, message:str = "values differ"):nil
assertNotEqual = fn(actual:any|nil, unexpected:any|nil, message:str = "values are equal"):nil
assertTrue = fn(value:any|nil, message:str = "expected true"):nil
assertFalse = fn(value:any|nil, message:str = "expected false"):nil
assertNear = fn(actual:num, expected:num, tolerance:num, message:str = "values are not close"):nil
assertThrows = fn(work:fn, type:str|nil = nil):any|nil
attempt = fn(work:fn):{threw: bool, value: any|nil}
examples = fn(tag:{arguments: list}):list<{arguments: any|nil, expected: any|nil}>
check = fn(function:fn, arguments:any|nil, expected:any|nil):Failure|nil
runTest = fn(function:fn):Failure|nil
```

Assertions throw `test.failure` on failure; `fail` always throws. `assertTrue`
and `assertFalse` compare to the exact boolean. `assertNear` uses absolute
tolerance. `assertThrows` calls a zero-argument function and optionally checks
the thrown value's `type`; it returns the payload. `attempt` catches a thrown
value into `{threw: true, value}` or returns `{threw: false, value}`.

`@test` marks a zero-argument function: throwing or returning `false` fails.
`@testWith([arguments...], expected, ...)` supplies complete input/result pairs.
`examples` reads pairs but does not run them; an odd argument count produces
no examples. Validate that shape in a runner. `check` and `runTest` return
failure records instead of throwing; turn those into a failing test-process
exit. Tags alone do not run tests. See [test wiring](getting-started.md#test-functions-with-test).

## Application runner — `slug.test.runner`

Installed source: `$SLUG_HOME/lib/slug/test/runner.slug`.

```text
run = fn(modules:list<str>, discover:fn<list, str>, verbose:bool = false):num
```

Pass module names and an application-defined callback
`fn(name:str):list { describe(name) }`. Defining it in the test entry preserves
relative resolution of sibling test files. Only exported declarations are
discovered by default. The runner executes `@test` and `@testWith` tags,
continues after failures, and reports totals. Assertion failures and wrong
results count as failures; unexpected throws count as errors, including
`throw nil`. Discovery failures, malformed tags, and non-function tagged
values are errors too. Setup/tag errors do not count as executed tests.

Module names run once in supplied order, declarations in metadata order, and
examples in tag order. `verbose` prints passing tests as well as failures.
The result is 0 for success and 1 for any failure/error or no tests; return it
from the application's `main`. No filesystem scanning or legacy argument
parser is involved. The separate `slug test` tool still runs clutch suites.

## Channels — `slug.channel`

[Source](https://github.com/sluglang/slug-rs/blob/main/lib/slug/channel.slug).
Creation and closing use the implicit `chan` and `close` builtins.

```text
send = fn<T>(channel:chan<T>, payload:T):chan<T>
trySend = fn<T>(channel:chan<T>, payload:T):chan<T>|nil
recv = fn<T>(channel:chan<T>, timeout:num = 0):T|nil
tryRecv = fn<T>(channel:chan<T>):T|nil
await = fn<T>(handle:task<T>, timeout:num = 0):T
```

`send` blocks until accepted and returns the channel. Sending `nil` or sending
to a closed channel fails. `trySend` returns `nil` if it cannot send immediately.
`recv` waits for a value or returns `nil` once closed and drained. `tryRecv`
returns `nil` when no value is ready; it does not distinguish empty from closed.
Positive timeouts are milliseconds and throw `TimeoutError`; zero waits
indefinitely. `await` returns the task result or propagates its error.

## Time and seeded randomness

[`slug.time` source](https://github.com/sluglang/slug-rs/blob/main/clutch/slug.core.clutch/modules/time.slug):

```text
nowMs = fn():num
monotonicMs = fn():num
sleepMs = fn(milliseconds:num):nil
```

`nowMs` is Unix-epoch wall-clock milliseconds. Use differences of
`monotonicMs` readings for durations. `sleepMs` accepts non-negative whole
milliseconds and suspends only the calling task.

[`slug.random` source](https://github.com/sluglang/slug-rs/blob/main/clutch/slug.core.clutch/modules/random.slug):

```text
generator = fn(seed:num):Generator
type Generator = {
    next: fn<num>, below: fn<num, num>, between: fn<num, num, num>,
    float: fn<num>, pick: fn<any|nil, list>, shuffle: fn<list, list>,
    state: fn<num>
}
```

The returned record's call shapes are `next()`, `below(n)`, `between(low, high)`,
`float()`, `pick(items)`, `shuffle(items)`, and `state()`. The same integer
seed gives the same sequence. `below` excludes its upper bound; `between`
includes both endpoints; `float` is at least zero and less than one.
Invalid ranges or an empty `pick` throw `random.range`. The generator is
stateful and unsuitable for secrets; see secure randomness in [desktop APIs](api-desktop.md).

## Standard input and signals

[`slug.io.stdin` source](https://github.com/sluglang/slug-rs/blob/main/clutch/slug.core.clutch/modules/stdin.slug):

```text
readLines = fn():chan<str>
readLine = fn():str|nil
prompt = fn(message:str):str|nil
confirm = fn(message:str, default:bool = false):bool
```

Input is one shared line stream, not a broadcast. Lines exclude the trailing
newline; EOF closes the channel and makes `readLine` return `nil`. `prompt`
writes without a newline before reading. `confirm` uses its default for empty,
unrecognized, or exhausted input. Native reader startup may throw `native.io`.

[`slug.os.signal` source](https://github.com/sluglang/slug-rs/blob/main/clutch/slug.core.clutch/modules/signal.slug):

```text
notify = fn(names:list<str>):chan<str>
```

Names are `"int"` and `"term"`. The channel receives those strings so the
application can shut down deliberately. Unknown/empty names throw `signal.name`;
registration failure throws `signal.error`. After delivery, the next signal
of that kind ends the process immediately unless it is registered again.

## Runnable core example

Run as a source program with the installed runner and core clutch:

```slug
val std = import("slug.std")
val string = import("slug.string")
val json = import("slug.fmt.json")
val test = import("slug.test")

val main = fn() {
    val numbers = std.map(["1", "2", "3"], fn(text:str):num {
        string.toInt(text) ?? 0
    })
    test.assertEqual(std.reduce(numbers, 0, fn(a:num, b:num):num { a + b }), 6)
    match json.parseStrict('{"name":"Slug"}') {
        fields:map => test.assertEqual(fields["name"], "Slug")
        _ => test.fail("expected a JSON object")
    }
    println("core API example passed")
}
```
