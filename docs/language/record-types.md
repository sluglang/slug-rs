# Record Types

A record type describes a value by the fields it has, whatever else it holds.
It is structural: it has no declaration and no identity of its own. The
rationale is in [Structural record types](../decisions/2026-11-22-structural-record-types.md).

```slug
type Named = {name: str}
type Player = {
  name: str
  hp: num
  guild: str|nil
}
type Box<T> = {value: T}

val greet = fn(who:Named):str { "hi " + who.name }
greet({name: "Ada", hp: 3})       // a map with a `name` string
greet(Pet { name: "Rex" })        // a struct whose schema has `name`
```

## Syntax

`{field: Type, ...}` is a type annotation form, written wherever an
annotation may be. A field is named by an identifier, a reserved word other
than `true` and `false`, or a string literal without interpolation: `{"first name": str}`. Fields are
separated by commas or newlines, and a trailing comma is allowed. A record
type has at least one field, and names each field once; `{}` and
`{a: num, a: str}` are source errors. Field order does not matter:
diagnostics show the fields sorted by name.

## Conformance

- **A map** conforms when, for each field, its value at that string key
  conforms to the field's type. A key the map lacks reads `nil` there, so a
  field whose type includes `nil` accepts a map without that key, and one
  whose type excludes `nil` requires it. There are no optional fields:
  `guild: str|nil` is how a field may be absent.
- **A struct** conforms when, for each field, its schema has a field of that
  name whose value conforms. A struct has every field of its schema, so a
  field it lacks never reads `nil`; a struct without the field does not
  conform.
- **Width.** Other keys, of any kind, and other fields do not matter:
  `{name: "Ada", hp: 3}` conforms to `{name: str}`. There are no exact record
  types.
- **Depth.** Field types are covariant, as values are immutable:
  `{pos: {x: num, y: num}}` conforms to `{pos: {x: num}}`.

Statically, a type conforms to a record type as follows.

| Type | Conforms to `{f: T, ...}` when |
| --- | --- |
| A record type | each field `f` it has conforms to `T`; a field it lacks may hold anything, so `T` must accept any value |
| A map literal's type | each key `f` it has holds a `T`; a key it lacks reads `nil`, so `T` must include `nil` |
| `map<K, V>` | `V|nil` conforms to each `T`, since the map may lack any key |
| `struct<S>` | `S` has each field `f`, with a type that conforms to `T` |
| `map` or `struct` | always: the checker does not know the contents |

A record type conforms to `any`, and to another record type as above; it
does not conform to `map<K, V>` or `struct<S>`, since its value may be
either.

## Map literals

A map literal whose keys are all distinct string constants, as in
`{name: "Ada", hp: 3}`, has a record type that lists each key with the type
of its value, and that is also its map type, `map<str, str|num>`. So a
literal may be passed where a record type or a map type is expected. A
literal with any other key, and the empty literal `{}`, has a map type as
before. The checker keeps the record type through bindings, `+` of two such
literals, which gives the right one's type at a shared key, and `copy`, which
gives each replaced field its new type. A `var` declared from a map literal
takes its map type, since later values need not have the same keys.

## Use

Reading a field of a record type, as `p.name` or `p["name"]`, gives its type.
A field that a map literal's type lacks reads `nil`, as its map type says;
one that a record type does not list may hold anything. A nil test narrows a
field of an immutable binding as it does a map's: `if (p.guild != nil) {
p.guild }` reads a `str`. A map pattern binds a listed field with its type.

Generic aliases make generic record types, as `type Box<T> = {value: T}`, and
a generic function's type arguments are inferred from a record's field types:
`unbox({value: "s"})` for `fn<T>(box:Box<T>):T` gives `str`. A recursive alias
may refer to itself in a field, as `type Tree = {value: num, children:
list<Tree>}`, since a field is inside a type constructor.

A mismatch names the first field that does not conform:

```text
expected {name: str}, got {name: num}: field `name` is num, but must be str
expected {name: str}, got map<str, str>: field `name` may be missing, which reads `nil`, but must be str
```

## Match constraints

A record type is a runtime-checkable match constraint when its field types
are. The check reads each listed field: a map's value at the string key, or
`nil` when the key is absent, and a struct's field of that name, failing when
its schema has none. It walks nested field types as `list<T>` and
`map<K, V>` constraints walk elements, once per match.

```slug
val describe = fn(value) match {
  r: {name: str} => r.name
  r: {weight: num} => "weighs ${r.weight}"
  _ => "?"
}
```

A case binding takes the record type. On a subject of closed type, a record
constraint covers each member whose every value conforms, as
[closed-match coverage](match-and-destructuring.md) requires, and a case
whose record type no member could conform to is unreachable.

## Not yet

- A record type is not checked where an unannotated value reaches an
  annotation; that is [stage 6 of the typed-Slug plan](../planning/typed-slug.md).
- A record type gives a value no fixed layout.
