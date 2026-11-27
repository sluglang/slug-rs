# Desktop API reference

This hand-maintained companion to the [core reference](api-reference.md)
documents common optional desktop clutches. Signatures omit bodies and are
not runnable declarations. Type names belong to the module shown; names such
as `tcp` and `sqlite` in signatures denote imported module bindings.

Use the existing `$SLUG_HOME/target/release/slug` with its matching installed
clutches. Missing providers, native libraries, or system dependencies require
reporting the setup issue; obtain explicit user confirmation before downloads,
builds, runtime substitution, or native staging. Source links follow upstream
`main`; consult the installed revision when it differs from [VERSION](VERSION).

## Files — `slug.io.fs`

[Source](https://github.com/sluglang/slug-rs/blob/main/clutch/slug.io.fs.clutch/modules/fs.slug).

```text
resource File
type Stat = {kind: str, size: num, modifiedMs: num}
openRead = fn(path:str):File
openWrite = fn(path:str):File
openAppend = fn(path:str):File
readLine = fn(file:File):str|nil
read = fn(file:File, maxBytes:num = 65536):bytes|nil
write = fn(file:File, content:str|bytes):num
sync = fn(file:File):nil
close = fn(file:File):nil
stat = fn(path:str):Stat|nil
list = fn(directory:str):list<str>
makeDirectory = fn(path:str, parents:bool = false):nil
rename = fn(from:str, to:str):nil
remove = fn(path:str):nil
removeAll = fn(path:str):nil
temporaryDirectory = fn(prefix:str = "slug-"):str
separator:str
join = fn(parts:list<str>):str
```

Paths are relative to the shell's working directory unless absolute.
`openWrite` creates or truncates; `openAppend` creates or appends.
Register `defer { fs.close(file) }` immediately after opening a handle.
Closing is idempotent; using a closed handle fails. `read` and `readLine` share
the file position and return `nil` at EOF. Binary reads may be short; keep
reading until EOF. `maxBytes` is from 1 through 16 MiB. `readLine` removes the
newline and rejects oversized lines, NUL, and invalid UTF-8. `write` writes all
content and returns its byte length; `sync` waits for device persistence.

`stat` returns `nil` for absence and does not follow a final symlink. Its
`kind` is `"file"`, `"directory"`, `"link"`, or `"other"`; `size` is bytes
and `modifiedMs` is Unix-epoch milliseconds. `removeAll` is recursive: use
only for paths the task authorizes deleting. `join` joins path components;
it does not establish that a path stays inside an application directory.
I/O failures generally throw `native.io` errors. See the [file example](examples/README.md).

## TCP — `slug.net.tcp`

[Source](https://github.com/sluglang/slug-rs/blob/main/clutch/slug.net.tcp.clutch/modules/tcp.slug).
`Listener` and `Connection` are exported schema-backed types. Treat them as
opaque handles and close them explicitly.

```text
listen = fn(host:str, port:num, backlog:num = 128):Listener
localPort = fn(listener:Listener):num
accept = fn(listener:Listener, timeout:num|nil = nil):Connection|nil
connect = fn(host:str, port:num, timeout:num = 10000):Connection
read = fn(connection:Connection, timeout:num|nil = nil):bytes|nil
chunks = fn(connection:Connection):chan<bytes|num>
write = fn(connection:Connection, data:bytes|str):nil
setWriteTimeout = fn(connection:Connection, milliseconds:num):nil
shutdownWrite = fn(connection:Connection):nil
peer = fn(connection:Connection):str
close = fn(value:Listener|Connection):nil
```

Timeouts are milliseconds. `accept` returns `nil` after listener closure;
`read` returns `nil` at end of stream. Specified accept/read timeouts throw
`net.timeout` rather than returning `nil`. Received chunks are at most 16 KiB
and do not preserve peer write boundaries. `chunks` exposes the same stream
consumed by `read`; a number on that channel indicates a system error.
`shutdownWrite` ends sending while leaving reception available. Register
cleanup for listeners and connections, including after errors.

## HTTP server — `slug.net.http`

[Source](https://github.com/sluglang/slug-rs/blob/main/clutch/slug.net.http.clutch/modules/http.slug).
This is a server API built on TCP, not an HTTP client API.

```text
type Request = {
    method: str, path: str, query: map<str, str>,
    headers: map<str, str>, params: map<str, str>
}
type Options = {
    idleTimeout: num|nil, headerTimeout: num|nil,
    maxHeaderBytes: num|nil, maxBodyBytes: num|nil,
    maxConnections: num|nil
}
serve = fn(listener:tcp.Listener, handler:fn, options:Options = {}):nil
body = fn(request:Request, maxBytes:num|nil = nil):bytes
read = fn(request:Request, maxBytes:num):bytes|nil
multipart = fn(request:Request, maxBytes:num):map
form = fn(request:Request):map<str, str>
mediaType = fn(name:str):str
router = fn(routes:list):fn
```

`serve` runs until its TCP listener closes. It calls the handler for each
request. Defaults: idle/header timeouts 5000 ms, header limit 8192 bytes,
body limit 1048576 bytes, and unlimited connections. Options can be omitted.
Paths and query parameters are decoded; header names are lowercased.

Return a string for a 200 text response, or a map with `status` and optional
`headers` and `body`. A body can be a string, bytes, or a channel of strings
or bytes for streaming; close the channel to end the response. Client
disconnect closes that channel, so further sends fail. Invalid responses or
handler errors produce a 500 response unless an `http.reject` specifies another
status.

`router` takes `[method, pattern, handler]` entries. Patterns support `:name`
segments and a trailing `*rest`. The first matching route wins; GET also
handles HEAD. A matching path with the wrong method gives 405; no path gives
404. Parameters appear in `request.params`.

Read bodies deliberately with a limit. `body` caches the whole bytes value but
cannot follow streaming `read` or `multipart`. `read` returns chunks then
`nil`; its limit bounds the total body, not each returned chunk. `form` decodes
URL-encoded fields. `multipart` returns a reader whose `next()` returns a
part or `nil`. Parts provide `name`, nullable `filename`, `contentType`,
`headers`, `read()` for byte chunks, and `text(maxBytes)` for text. Moving to
the next part skips remaining bytes in the previous part. Malformed, wrong-type,
or oversized bodies can throw `http.reject` with a 400, 415, or 413 status.

## SQLite

[`slug.db.sqlite` source](https://github.com/sluglang/slug-rs/blob/main/clutch/slug.db.sqlite.clutch/modules/sqlite.slug):

```text
resource Database
open = fn(path:str):Database
close = fn(database:Database):num
exec = fn(database:Database, sql:str, ...parameters:any|nil):num
query = fn(database:Database, sql:str, ...parameters:any|nil):list<map>
```

`open(":memory:")` creates an in-memory database. `exec` executes one statement
and returns affected rows; `query` returns all rows as column-name maps.
Bind values to `?` placeholders by passing positional parameters after SQL;
do not interpolate application data into SQL. Supported bindings are `nil`,
numbers, strings, and bytes. Rows map SQL NULL to `nil`, integer/real to
`num`, text to `str`, and blobs to `bytes`. Failures throw `sqlite.error`
or `sqlite.bind`. Close prepared statements before their database.

[`slug.db.sqlite.statement` source](https://github.com/sluglang/slug-rs/blob/main/clutch/slug.db.sqlite.clutch/modules/statement.slug):

```text
resource Statement
prepare = fn(database:sqlite.Database, sql:str):Statement
close = fn(statement:Statement):num
exec = fn(statement:Statement, ...parameters:any|nil):num
query = fn(statement:Statement, ...parameters:any|nil):list<map>
```

Prepared statements can be reused; execution resets them and clears their
bindings. Register statement cleanup after preparation and database cleanup
after opening. Closed handles cannot be reused.

[`slug.db.sqlite.transaction` source](https://github.com/sluglang/slug-rs/blob/main/clutch/slug.db.sqlite.clutch/modules/transaction.slug):

```text
begin = fn(db:sqlite.Database):num
commit = fn(db:sqlite.Database):num
rollback = fn(db:sqlite.Database):num
transaction = fn<T>(db:sqlite.Database, work:fn<T, sqlite.Database>):T
```

`transaction` begins, calls `work(db)`, commits, and returns its result. On a
work failure it attempts rollback and rethrows. A rollback failure can replace
the active failure under the ordinary deferred-error rules.

## Secure randomness and hashing

[`slug.random.secure` source](https://github.com/sluglang/slug-rs/blob/main/clutch/slug.random.secure.clutch/modules/secure.slug):

```text
bytes = fn(count:num):bytes
token = fn(size:num = 16):str
seed = fn():num
```

`bytes` accepts 1 through 256 whole bytes. `token` encodes `size` random bytes
as twice that many lowercase hex digits. `seed` returns a random 32-bit unsigned
integer. These use the OS generator and cannot be seeded for replay. Failures
throw `random.range` or `random.os`.

[`slug.crypto.hash` source](https://github.com/sluglang/slug-rs/blob/main/clutch/slug.crypto.clutch/modules/hash.slug):

```text
resource Sha256
sha256 = fn():Sha256
update = fn(hash:Sha256, data:bytes|str):nil
finish = fn(hash:Sha256):bytes
sha256Of = fn(data:bytes|str):bytes
hex = fn(data:bytes):str
```

`sha256Of` computes a 32-byte digest at once; `sha256`, `update`, and `finish`
support incremental input. Finishing twice or updating after finishing throws
`hash.finished`. `hex` returns lowercase hex text.

[`slug.crypto.schnorr` source](https://github.com/sluglang/slug-rs/blob/main/clutch/slug.crypto.clutch/modules/schnorr.slug):

```text
verify = fn(publicKey:bytes, message:bytes, signature:bytes):bool
```

This module verifies BIP-340 signatures. Public keys are 32-byte x-only keys;
signatures are 64 bytes. Wrong lengths throw `schnorr.length`; invalid keys or
signatures return `false`. Messages may be any length. It does not export signing.

## Templates and Markdown

[`slug.text.mustache` source](https://github.com/sluglang/slug-rs/blob/main/clutch/slug.text.mustache.clutch/modules/mustache.slug):

```text
parse = fn(template:str):map
render = fn(template:str|map, data:any|nil, partials:map<str, str> = {}):str
```

Parse once and reuse the resulting template map with `render`, or pass template
text directly. Normal interpolations are HTML-escaped; triple braces and `&`
interpolations insert unescaped content. Missing partials render as empty text;
malformed templates or partials throw `mustache.syntax`.

[`slug.text.markdown` source](https://github.com/sluglang/slug-rs/blob/main/clutch/slug.text.markdown.clutch/modules/markdown.slug):

```text
toHtml = fn(source:str, options:map = {}):str
parse = fn(source:str, options:map = {}):Document
transform = fn(document:Document, rewrite:fn<Node, Node>, options:map = {}):Document
render = fn(document:Document, options:map = {}):str
toText = fn(document:Document, options:map = {}):str
```

The document API exposes a typed `Node` enum and `Document`, `Position`, and
`Span` record aliases with matching struct constructors. `transform` replaces
one node at a time, children first. Rendering validates constructed trees and
always escapes raw HTML and checks link/image URLs. Heading nodes accept an
application-assigned `id`; link/image nodes expose resolved destinations and
source provenance. See the
[document contract](https://github.com/sluglang/slug-rs/blob/main/docs/reference/markdown-documents.md)
for variants, source spans, and independent tree/output budgets. Internal parser
helper modules are omitted from this compact reference.

## Runnable SQLite example

Requires the existing SQLite clutch and its native dependency. Run as a source
program with the configured release runner; the database lives only in memory.

```slug
val sqlite = import("slug.db.sqlite")
val test = import("slug.test")

val main = fn() {
    val database = sqlite.open(":memory:")
    defer { sqlite.close(database) }
    sqlite.exec(database, "CREATE TABLE greeting (name TEXT)")
    sqlite.exec(database, "INSERT INTO greeting (name) VALUES (?)", "Slug")
    test.assertEqual(sqlite.query(database, "SELECT name FROM greeting"), [{name: "Slug"}])
    println("SQLite API example passed")
}
```
