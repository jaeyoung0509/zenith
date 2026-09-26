# DotSlash cache review decision

DotSlash has a whole-cache `dotslash -- clean` command, but no command to
remove only old objects. The upstream source was inspected at
[`1f94ba967f2c`](https://github.com/facebook/dotslash/tree/1f94ba967f2c) on
2026-09-26. Its [source layout](https://github.com/facebook/dotslash/blob/1f94ba967f2c/src/artifact_location.rs)
maps each downloaded artifact to one hash-addressed directory and keeps a
separate per-artifact lock. The [owner's discussion](https://github.com/facebook/dotslash/issues/114)
warns that deleting individual files by access time can break multi-file
artifacts. Zenith's adapter therefore considers a **complete artifact
directory** as one unit. No owner source was copied into Zenith.
No DotSlash executable was installed on the audited Mac, so a local CLI
version or a command preview could not be recorded.

The user can enable **Intensive cleanup** in Settings. The default is off.
When enabled, only artifact directories in the default per-user macOS cache
that have had **no entry modified for 30 days** become reviewable Rebuild
items. Modified time does not establish last use. No item is automatically
selected, and the plan requires explicit confirmation. A reviewed item moves
to Trash; the user must empty Trash to free disk space. DotSlash may download
and unpack it again if the user needs it.

The adapter refuses a linked, special, recently changed, partially measured,
or excessively deep object. It refuses a missing or malformed owner lock, a
running DotSlash process, or an executable currently running from the object.
Planning re-enumerates the store and captures directory identity. Execution
checks the original root and hash shape, obtains the owner's existing lock,
rechecks identity, contents, age, size, and processes, then moves only that
directory. The `locks` tree is never cleaned. The user-owned
`DOTSLASH_CACHE` override is recognized as unsupported rather than silently
examining the default location or following an arbitrary override path.

On the audited Mac, a reference preview listed about 537 MB under the default
cache. The earlier generic trial could not completely measure protected
entries. A later read-only owner-provider scan observed 537,231,360 bytes
across two complete objects and found none eligible under the 30-day rule.
Zenith leaves any unknown bytes out of eligible totals. No live cache was removed. Disposable fixtures
cover old and recent objects, owner activity, linked content, a changed object
after planning, absent or occupied locks, incomplete measurement, and an
overridden root.
