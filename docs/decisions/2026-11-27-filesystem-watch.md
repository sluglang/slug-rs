# Filesystem watch invalidations

## Status

Accepted

## Context

Development servers need to notice source changes without repeatedly scanning
the filesystem. Native notifications can combine changes or lose detail, and
a slow consumer must not turn a full channel into silent stale state.

## Decision

`slug.io.fs` owns native directory subscriptions as resources. A buffered
boolean channel announces readiness; draining the subscription produces a
bounded batch of relative path invalidations or an explicit full-rescan flag.
The flag and terminal errors remain in subscription state independently of
channel capacity. Native workers use the existing
[typed producer capability](2026-10-11-typed-native-producers.md); they never
enter the VM or retain Slug values.

Subscriptions establish monitoring before returning, support recursive
watching and relative subtree exclusions, and do not traverse descendant
symbolic links. Root removal or replacement is terminal. Close is idempotent,
wakes consumers, and requests cancellation without waiting on native work.

The first backend uses macOS FSEvents. Other platforms return a checked
unsupported error; there is no implicit polling fallback. The public contract
is specified in [filesystem watching](../reference/filesystem-watch.md).

## Consequences

Applications can select between filesystem readiness, timers, and shutdown
using existing channels. Debouncing, rebuilding, filtering application inputs,
and publishing snapshots remain application responsibilities. Renames are
invalidations rather than portable paired operations. A consumer must rescan
after detail loss and must drain after channel closure to observe errors.

Native pending memory is bounded. Backend callbacks and the resource lifetime
must remain synchronized through cancellation and producer revocation.

## Migration

Existing file operations are unchanged. `fs.close` also accepts a `Watcher`.
