# Homebrew cleanup operation boundary

Zenith currently offers direct, verified files in Homebrew's `downloads`
directory as individually selected Rebuild units. It leaves API/bootsnap
metadata and unrecognized entries advisory. This is a **deep download purge**,
not Homebrew's narrower `brew cleanup` operation. The two amounts must never
be added as if their target sets were disjoint.

On 2026-09-26, a read-only `brew cleanup --dry-run --prune=7` on the audited
Mac named old Cellar formula versions, while the selected direct download
units were under the cache directory. The published [Homebrew command
reference](https://docs.brew.sh/Manpage#cleanup-options-formula-cask-)
describes the command's age and cache behavior, but does not provide a
machine-readable candidate format or a transaction token that binds a dry-run
to execution. Human-readable `Would remove:` lines and warning text may change
between Homebrew versions. A preview of the cache root would also overstate
what the command will remove.

For that reason this change does **not** expose `brew cleanup` as a cleanup
button. A safe adapter needs all of the following before it can be enabled:

1. Resolve a trusted Homebrew executable and its version; use fixed, bounded
   dry-run arguments with updates and autoremove disabled.
2. Parse a complete, version-qualified set of exact candidate paths and sizes.
   Unknown lines, partial output, timeout, or an unexpected root block the
   action instead of becoming a zero-byte preview.
3. Bind the reviewed set, age option, and roots into a private one-shot plan.
   Re-run the preview before execution and refuse if targets or identities
   changed. Refuse an active or unprovable Homebrew owner state.
4. Run only Homebrew's fixed cleanup command after explicit confirmation. Do
   not fall back to recursive deletion or silently include autoremove.
5. Report the attempted target set, verified remaining candidates, and an
   independently measured disk-free delta as distinct values. A command exit
   code alone cannot prove reclaimed bytes.

The current lifecycle-provider interface receives a provider ID at execution,
not the exact dry-run set the user reviewed. Adding this action without a
private candidate-set authorization would allow Homebrew's target list to
change between review and execution. The direct download provider already
binds each reviewed file to its identity and measured size, then rechecks both
before removal. Keep the narrower command unavailable until its equivalent
contract and isolated tests exist. Windows and Linux have no Homebrew adapter
in Zenith.
