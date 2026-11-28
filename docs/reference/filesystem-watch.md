# Filesystem watching

`slug.io.fs` provides native directory-change subscriptions. The first backend
uses macOS FSEvents; on other platforms `watch` throws `native.io` with
`data.code == "unsupported"`. Watching is an optional desktop library feature,
not a new language construct or a polling service.

## API

```slug
val fs = import("slug.io.fs")

val main = fn() {
    val watcher = fs.watch(".", recursive = true, exclude = ["dist", ".git"])
    defer fs.close(watcher)
    val changes = fs.ready(watcher)
    val consume = fn() {
        select { recv changes }
        match fs.drain(watcher) {
            nil => nil
            batch:fs.WatchBatch => {
                if (batch.rescan) { println("rescan all inputs") }
                else { println(batch.paths) }
                recur()
            }
        }
    }
    consume()
}
```

- `watch(directory:str, recursive:bool = false, exclude:list<str> = []):Watcher`
  establishes monitoring before returning. Install it before scanning initial
  inputs. The root must exist and be a directory, not a final symbolic link.
- `ready(watcher:Watcher):chan<bool>` returns the same buffered notification
  channel on every call. Its booleans are coalesced wakeups, not event counts.
  One consumer owns receiving and draining. Do not send into this channel.
- `drain(watcher:Watcher):WatchBatch|nil` takes pending invalidations without
  waiting for an event. `WatchBatch` is `{paths:list<str>, rescan:bool}`.
  An open subscription may return an empty batch after a redundant wakeup.
  An explicitly closed subscription returns nil. A terminal failure throws on
  every drain, including after close. Always drain after receiving, even if
  the channel receive returned nil.
- `close(watcher)` is idempotent, discards pending paths, closes readiness, and
  requests native cancellation. It does not wait for the worker to finish.
  Keep the usual `defer fs.close(watcher)` ownership discipline. Resource
  destruction provides fallback cancellation.

## Invalidation semantics

Paths are relative to the watched root, use the filesystem's separator, and
may identify files or directories that no longer exist. The empty path names
the root. A directory path invalidates its subtree. Ordering, exact delivery
count, and rename pairing are unspecified. A rename invalidates the affected
paths when known; loss of location detail requires a full rescan. These are
hints to inspect current state, not a filesystem transaction log.

Recursive subscriptions cover existing and newly created descendants. A
nonrecursive subscription reports only the root and its immediate children.
Descendant symbolic links are not traversed. Ancestor links use ordinary
filesystem resolution; this API does not provide canonicalization, containment,
or protection against hostile concurrent path changes. If the watched root is
removed, moved, or replaced, draining throws `native.io` with
`data.code == "watch_lost"`. The subscription does not automatically reattach.

Exclusions are relative subtree paths using `/` separators, including on a
host with a different native separator. Empty paths, absolute paths,
backslashes, empty components, `.` and `..`, and trailing slashes are invalid.
Matching is byte-exact and uses component boundaries: excluding `dist` also excludes
`dist/page.html`, but not `distance.txt`. There are no glob patterns. Excluded
descendants do not produce path invalidations; a broad ancestor invalidation
or global event loss may still require inspecting the root. Exclusions do not
make the watch a security boundary.

Pending detail is limited to 1024 distinct paths and 1 MiB of path bytes,
including terminators. Excess events, operating-system overflow, allocation
failure while accumulating paths, or paths that cannot become UTF-8 Slug
strings discard pending detail and set `rescan: true`. Ignore `paths` on a
rescan and rediscover current inputs. The flag persists until a successful
drain, even if the notification channel is full and no more changes arrive.
Native exclusion storage has the same count and byte bounds.

Startup errors throw before `watch` returns. Later terminal errors remain
available through `drain` and close readiness. Errors follow the filesystem
`native.io`/`native.alloc` convention; `watch_lost` and `unsupported` supplement
the ordinary filesystem codes. Explicit close wakes waiters immediately;
receiver loss or VM shutdown also causes native cancellation. Native workers
never wait for a consumer to drain a full notification channel.

## Application policy

Keep collecting invalidations while rebuilding. Serialize builds, coalesce
bursts with a timer, and schedule another pass if changes arrived during a
build. A full rebuild from freshly read inputs is a valid first consumer.
On failure, retain the last successful published snapshot. Snapshot replacement
and request consistency belong to the application.

Install exclusions for output and staging locations before publishing. Broad
directory invalidations can include changes caused by output; applications
that write inside their watched root should reconcile relevant input state
before scheduling another build, rather than treating every wakeup as proof
that source contents changed. Configuration changes may require replacing the
subscription before writing to a new output location.

The backend follows Apple's
[FSEvents lifecycle](https://developer.apple.com/library/archive/documentation/Darwin/Conceptual/FSEvents_ProgGuide/UsingtheFSEventsFramework/UsingtheFSEventsFramework.html)
and uses the existing Slug [native producer boundary](native-abi.md).
