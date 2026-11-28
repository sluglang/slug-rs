# Slug language essentials

Each Slug code block below can run as its own program. This guide covers common
application patterns; consult the upstream sources linked in [README](README.md)
for detailed rules and implementation availability.

## Bindings, expressions, and calls

```slug
val greet = fn(name:str, title:str = "Mx"):str {
    "Hello $title $name"
}
var count = 0
count = count + 1
println(greet(name = "Slug", title = "Dr"))
println(if (count > 0) { "ready" } else { "waiting" })
```

Use `val` for immutable bindings and `var` for reassignment. A block evaluates
to its final expression; an empty block is `nil`. Function declarations use
`val name = fn(...) { ... }`. Named arguments use `=`, while annotations and
map fields use `:`. `if` requires a parenthesized condition and brace blocks.
Newlines usually separate statements; use commas between collection entries.

Double quotes support escapes and `$name` or `${expression}` interpolation.
Single quotes are raw strings. `//` begins a line comment. Documentation blocks
use `/** ... */`, with `*` at the start of each non-empty content line.

## Types and narrowing

Common annotations are `num`, `str`, `bool`, `bytes`, `nil`, `list<T>`,
`map<K, V>`, and unions such as `str|nil`. `any` excludes `nil`; `any|nil`
includes every value. `fn<R, P1, ...>` puts the result type first.
Annotations check values; they do not coerce them. A `var`'s inferred type
constrains later assignments, so annotate a binding that intentionally holds
several kinds of value.

```slug
val display = fn(value:str|nil):str {
    match value {
        nil => "missing"
        text => text
    }
}
println(display(nil))
println(display("ready"))
```

Match cases use `=>` and are separated by newlines or semicolons. `_` is a
wildcard; ordinary names bind values, rather than comparing to an existing
variable. Use a pinned pattern (`^name`) to compare to an existing value.
Type constraints such as `items:list` narrow a case's value. Cover all possible
inputs; use a deliberate fallback when the input is open-ended.

## Immutable collections and records

```slug
val original = [1, 2]
val appended = original :+ 3
val combined = original + [3, 4]
val user = {name: "Slug", active: true}
val updated = user + { active: false }
val field = "score"
val scores = {[field]: 7}
println([original, appended, combined])
println([user.active, updated.active, scores["absent"] ?? 0])
```

Lists and maps are immutable values. Appending (`:+`), concatenating lists
(`+`), merging maps (`+`), and removing a map key (`-`) return new
values; `+` also sets a struct's fields, as `user + { active: false }`. Retain the result or reassign a `var`. A right-hand map merge overwrites
matching keys. Map order is unspecified. A missing map key returns `nil`.
`??` evaluates its fallback only for `nil`, preserving `false`, `0`, and `""`.

Bare map keys are strings: `{name: value}`. Computed keys need brackets:
`{[key]: value}`. Dot access reads string keys; there is no implicit receiver
when calling a function held in a map. Lists, strings, and bytes support negative
indices and slices such as `[1:3]`. String positions count Unicode scalars.

Use structural record annotations for fixed fields, and schemas when values
need nominal identity:

```slug
type Named = {name: str}
val label = fn(value:Named):str { value.name }
val User = struct { name:str, active:bool = true }
val user = User { name: "Slug" }
val disabled = user + { active: false }
println(label(disabled))
```

`struct<User>` denotes instances of that particular schema. `+` on a struct
checks each field it sets against the schema and keeps the schema. It binds
less tightly than `.`, so read a field of the result as `(user + { active: false }).active`. Declare shared
schemas once at module level. Defaults evaluate when the schema is created.

## Repetition and pipelines

```slug
val {map, each, reduce} = import("slug.std")
val doubled = [1, 2, 3] /> map(fn(n:num):num { n * 2 })
each(doubled, fn(n:num) { println(n) })
println(reduce(doubled, 0, fn(total:num, n:num):num { total + n }))
val sumTo = fn(n:num, total:num = 0):num {
    if (n == 0) { total } else { recur(n - 1, total + n) }
}
println(sumTo(10))
```

There are no `for`, `while`, or `loop` constructs. Use `map` for transformed
results, `filter` for selection, `reduce` for an accumulator, and `each` for
effects whose results you do not need. `/>` supplies its left side as the
call's leading argument. `recur(...)` restarts the current function without
growing its stack and must be in tail position.

## Errors and cleanup

```slug
val positive = fn(value:num):num {
    if (value <= 0) { throw "expected a positive number" }
    value
}
val safe = fn():num {
    defer onerror(error) { return 0 }
    positive(-1)
}
println(safe())
```

`throw` propagates a value. Use `defer { cleanup() }` to run cleanup when its
enclosing scope exits. `defer onsuccess` runs only after success.
`defer onerror(error)` recovers if its handler completes without throwing;
its result becomes the function result. A handler that only logs must end
with `throw error` to preserve failure. Slug has no `try`/`catch` syntax.
For expected invalid input, consider an explicit result value instead of
recovering from an unrelated runtime fault.

## Modules and concurrency

Imports use names such as `import("app.greeting")`; only declarations prefixed
with `export` are public. Prefer a module binding or explicit destructuring
over wildcard imports. Modules initialize once and imports observe live exported
bindings. Keep initialization dependencies simple and avoid cyclic use of values
before they initialize. See the [multi-module example](examples/README.md).

```slug
val main = fn() {
    val channels = import("slug.channel")
    nursery {
        val messages = chan<str>()
        val sender = spawn { channels.send(messages, "hello") }
        println(channels.recv(messages))
        channels.await(sender)
        close(messages)
    }
}
```

`spawn` shares captured bindings; the nursery owns and joins its children.
`fork` sees a snapshot and restricts shared writes, transferred values, native
calls, and module initialization. Initialize dependencies before forking.
Channels carry non-nil messages; receive returns `nil` after a closed channel
drains. `select` supports receive, send, task completion, and timers. Awaiting a
task uses `slug.channel.await`, not an `await` expression keyword. Consult the
runtime requirements before designing cancellation or parallel work.
