# Zenith Threat Model

This document records the security boundaries that Zenith actually enforces and
the platform ceilings it cannot remove. It is intentionally narrow: it covers
credential persistence, provider authorization, diagnostics redaction, and the
local inspection surface. Cleanup trust boundaries live in
[SAFETY.md](SAFETY.md).

## Assets

- Provider API keys and OAuth material held in the OS credential store.
- Subprocess output, provider errors, and log/audit entries that may quote
  secrets, request URLs, or filesystem locations.
- Project identity and local paths that cross into the WebView.
- Third-party agent configuration files that Zenith may rewrite during
  integration removal.

## Boundaries that are enforced

- **Credentials never cross IPC.** `SecretString` has no `Serialize` derive,
  its `Debug` and `Display` are fixed redactions, and no command returns a
  stored secret. Secrets are never passed on a command line or through a child
  environment.
- **Provider authorization is bound to one attempt.** The OpenRouter loopback
  callback requires a high-entropy `state` value carried in the callback URL
  (the shape OpenRouter echoes back), verifies the peer is loopback, and bounds
  the bytes it reads. An unsolicited request cannot abort a pending legitimate
  flow.
- **Disconnect is local and honest.** OpenRouter OAuth returns a non-expiring
  key, and OpenRouter's documented key-deletion API requires a management key
  that Zenith never holds, so an OAuth key cannot self-revoke. Disconnecting
  removes the key from the OS credential store, and the UI directs the user to
  delete it in the OpenRouter dashboard. If persistence fails after the
  provider issues a key, the flow fails with the same manual-revocation
  guidance instead of pretending the key was invalidated.
- **Third-party files keep their permissions.** Rewriting an agent settings
  file preserves the original owner-only mode, and a failed rename removes the
  temporary copy.
- **Diagnostics are redacted on every exit.** One shared pattern table backs
  both the log sanitizer and the AI Control Center secret scanner. Log and audit
  files are created owner-only on Unix; on Windows they inherit the user-profile
  ACL. Subprocess or provider error text is sanitized before it is returned
  across IPC.
- **Display paths are masked.** Identity paths render as `~/…` or
  `.../basename`; the diagnostics clipboard export cannot carry the home
  directory or user name. Storage and cleanup views that exist to show a
  location are explicit, documented exceptions.
- **An allowed command is not an authorization.** Tauri capabilities decide
  which window may call a command; the application service decides what that
  call may do, from backend state the interface cannot supply. No command
  accepts a path, a cleanup strategy, a filesystem identity, or an arbitrary
  PID, so a compromised renderer with every capability granted still reaches
  only the opaque IDs and inventories the backend already produced, under the
  same plan TTL, one-shot, scope, identity, and TOCTOU checks the dashboard
  runs. `CleanupService`, `StorageService`, and `SystemService` apply their
  capability and operation gates themselves, and the capability split is
  asserted against a reviewed allowlist (`capability_contract_tests.rs`) rather
  than trusted to review.
- **The desktop framework stays outside the trusted logic.** `zenith-core` and
  `zenith-platform` cannot depend on `tauri*` or on `zenith-desktop` at any
  depth, which `scripts/check_core_boundaries.cjs` refuses from the resolved
  dependency graph. A domain rule therefore cannot silently start reading a
  window, a `Channel`, or a capability file.

## Workspace-supplied repository content

Zenith inspects the projects an AI agent is observed working in. Those
directories are chosen by the work, not nominated by the user: a project root
comes from an observed agent process working directory, so the repositories
Zenith reads are the directories the user happens to work in. Zenith reads state
there; it does not execute or rewrite the configuration it finds.

- **One constructor builds every invocation and neutralizes fixed-name program
  hooks.** Git reads a repository's own `.git/config` whenever it
  operates on that repository, and several documented keys name a program Git
  then runs:
  `core.fsmonitor` is consulted by `git status` specifically, and `core.pager`,
  `core.hooksPath`, `diff.external`, and `core.sshCommand` are the same class.
  Those keys have fixed names, so `git::git_command` replaces each of them on
  the command line, where Git gives them precedence over any configuration file.
  Content diffs also pass `--no-ext-diff` and `--no-textconv`, which Git gates
  separately from configuration, and a content diff that omits `--no-ext-diff`
  fails closed rather than running the repository's program. The allowed
  behavior of an invocation is: report state for the project it was pointed at
  (status, `HEAD`, name-status, and file content already limited to 256 KiB per
  diff), plus the configuration listing that decides whether a driver definition
  is present, which reads configuration and runs nothing.
- **A driver definition is refused rather than neutralized.**
  `filter.<driver>.*`, `diff.<driver>.*`, and `merge.<driver>.*` name programs
  too, but the driver name is whatever the repository writes, so no command line
  can name the key in advance. `$GIT_DIR/info/attributes` is the same problem
  from the other side: it outranks every other attribute source and no
  command-line configuration replaces it. A repository carrying a non-empty
  `info/attributes`, or one whose own configuration defines a driver program —
  read through Git's own parser with includes expanded, so a file an `include`
  directive pulls in cannot hide it — therefore has its git-derived state
  refused: the status and diff invocations are never made, the reason is
  recorded in the diagnostics log, and the Control Center carries it as the
  summary's status message. The refusal pass is repeated immediately before
  every command is built, so an earlier `GitInspection` does not silently stay
  valid after the repository adds a driver definition or `info/attributes`.
  One value is
  weaker than that: `ProjectIdentity::is_dirty` is a boolean, so a refused or
  failed read shows as "no observed changes" in the project list while only the
  log carries the reason.
- **Nothing else is neutralized, because a checkout has to read the way the
  user's own shell reads it.** The machine's Git configuration, the user's own
  configuration, and the repository's attribute files are all honored. Removing
  them made Zenith's answer differ from `git status` in the user's terminal:
  with the attribute source pinned to the empty tree, a clean `text eol=crlf`
  checkout read as modified, and with `GIT_CONFIG_NOSYSTEM` set, a Windows
  checkout whose `core.autocrlf` came from the system configuration did too.
  `tests/git_boundary_tests.rs` holds that parity against the user's own git for
  both kinds of line-ending configuration, in a checkout and in a linked
  worktree. The residual is stated rather than hidden: a repository may *select*
  a driver program that the user's own configuration defines — git-lfs installed
  machine-wide is the ordinary case — which is the same program the user's own
  `git status` runs in that directory. A same-user process can also race any
  filesystem preflight by replacing repository metadata after the last check;
  this boundary reduces that window but is not process isolation.
- **A `.git` pointer must be tied to this checkout, not merely shaped like one.**
  `.git` may be a *file* naming the git directory that holds the repository,
  which is how a linked worktree and a submodule record it. The pointer file, the
  `HEAD` it resolves to, and the metadata used to validate the relation are read
  under a 4 KiB cap enforced on the open handle, so growth between a check and
  the read cannot exceed it. A target inside the project is validated by
  `SymlinkGuard`; a target outside it is accepted only through Git's own
  backlink — a linked worktree records `gitdir` naming this project's `.git`, a
  submodule records `core.worktree` naming this project — and anything else is
  refused and recorded rather than reported as "not a repository". A link at
  `.git`, including a Windows directory junction, is resolved and judged instead
  of followed.
- **Repository-derived strings are bounded before they cross IPC.** A branch
  name longer than Zenith's own 1 KiB bound is refused rather than truncated into
  a name Git never created, and a detached `HEAD` whose content is not an object
  id (40 hex digits, or 64 in a SHA-256 repository) is reported as no state
  instead of being echoed to the interface.

## Platform ceilings recorded, not fixed

- **macOS Keychain ACL is bound to a code-signing identity.** For an unsigned
  application in a user-writable bundle, the generic-password ACL is effectively
  path-based. An attacker who can replace the application bundle inherits access
  to the stored items. This is tied to the signing work tracked separately;
  until the application is signed with a stable identity, treat the Keychain
  item as protected against other users but not against a same-user bundle
  replacement.
- **Windows generic credentials are readable by any process running as the same
  user.** DPAPI protects the blob at rest but does not prompt for a per-process
  identity, so a same-user process can call `CredReadW` for the same target
  name. This is a property of the Windows Credentials API, not of Zenith.
- **Linux and other platforms have no credential store.** The credential
  store fails closed there: reads and writes return `StorageUnavailable`, and a
  plaintext file fallback is explicitly not acceptable. If the OpenRouter flow
  receives a key and cannot persist it, it fails with instructions to revoke the
  key manually in the OpenRouter dashboard; it cannot self-revoke an OAuth key.
- **Same-user process inspection is cooperative.** Agent activity and memory
  features rely on OS process metadata. They do not provide an isolation
  boundary against code already running as the same user.
- **Unsigned and unnotarized distribution.** Windows artifacts are unsigned and
  macOS artifacts are not notarized, which is a dated decision recorded in
  [CODE_SIGNING_POLICY.md](../CODE_SIGNING_POLICY.md). Do not assume
  operating-system attestation of the binary. Release trust currently rests on
  published SHA256 checksums, GitHub build provenance attestation, SPDX SBOMs,
  and a recorded Microsoft endpoint-protection review. Those bind an artifact
  to this repository, workflow, and commit, but none of them make the binary's
  behavior trustworthy to the operating system, and none of them confer
  SmartScreen reputation.
- **No updater and no background network activity.** Zenith never polls for
  updates, so a corrected release reaches a user only when the user opens the
  [releases page](https://github.com/jaeyoung0509/zenith/releases); the
  application exposes that URL through `PlatformContext.releases_url` and links
  to it. A compromised or vulnerable installation can therefore persist
  indefinitely after a fix exists, and no automatic remediation channel can be
  attacked or relied upon.

## Out of scope

- A compromised operating system, kernel, or debugger attached to Zenith.
- Network attackers who can terminate TLS or modify provider responses.
- Adversarial third-party configuration that the user explicitly executes
  outside Zenith. Zenith never executes or rewrites discovered configuration.
