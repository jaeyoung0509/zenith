# AI Control Center

AI Control Center is a local control plane built on the canonical Project
Cockpit session and project identities. It does not run a second process or
project classifier. The dashboard has four sections: Overview, Usage &
Budgets, Resource Autopilot, and Safety Posture.

## Provider capability matrix

Every observation carries its source kind, scope, observation time, freshness
window, reporting period/reset, quality, and any partial error. Values from
different sources are never presented as equivalent.

| Provider/capability | Source | Scope | Behavior |
| --- | --- | --- | --- |
| Codex subscription | Official local Codex app-server | Subscription | Live quota windows when available; retained as stale after a failed refresh |
| OpenAI API | Optional organization capability | Organization/API key | Separate from Codex subscription; unavailable until explicitly connected |
| OpenCode | Official local CLI statistics | Local sessions | Local estimate, not a provider bill |
| OpenRouter | Official API through the existing OAuth flow | API key | Live authoritative spend/limit data when connected |
| Antigravity | Existing local/external observation | Individual subscription | Primary individual Google coding product; no quota is implied |
| Gemini Code Assist | Optional organization capability | Organization | Standard/Enterprise/API usage only; distinct from Antigravity |
| Anthropic API | Optional organization capability | Organization | Distinct from a Claude individual subscription |
| Claude individual | Manual/external | Subscription | Zenith never scrapes `/usage`, the TUI, or credential files |
| Cursor Teams/Enterprise | Optional organization capability | Organization | Admin adapter capability only |
| Cursor individual | Manual/external | Subscription | No private editor state inspection |
| xAI API | Optional organization capability | Organization | Distinct from Grok Build |
| Grok Build | Manual/external | Subscription | No undocumented quota is inferred |

Manual entries are stored in validated Zenith settings and are always labelled
Manual. Money uses integer micro-units internally. “Zenith local budget alert”
means a local notification threshold; it never changes provider billing,
credits, or hard limits. A budget that combines authoritative, estimated, or
manual sources is labelled as mixed-source. Weekly and monthly alert periods
are stored independently and remain local Zenith policy.

## Resource policy and actions

Resource attribution consumes only canonical `AgentSession` and
`ProjectIdentity` records. Unassigned sessions are visible but have no mutable
authority. Keep Awake automation is off by default, works only for verified
sessions, optionally requires AC power, treats unknown power as ineligible, and
releases its assertion when the final verified session ends.

Battery, memory pressure, session completion, orphan-process, development-port,
and cleanup signals produce recommendations only. Native notifications are
limited to the explicitly enabled battery, memory, and completion preferences.
Port release and cleanup continue through their existing typed plan/preview
workflows. A Control Center recommendation creates an opaque, expiring,
one-shot navigation preview; consuming it does not perform the downstream
mutation.

## Safety and privacy boundaries

Safety inspection is user initiated and limited per run to registered canonical
project roots, 2,000 files, 1 MiB per file, and depth 8. It does not follow
symlinks or cross filesystems, and it prunes generated/vendor directories. Only
recognized MCP and tool configuration shapes are normalized. Results can
contain a category, relative path, line number, server name, transport,
permission/sandbox label, command basename, or domain. They never contain
secret bytes, raw arguments, headers, environment values, full commands, email
addresses, tokens, or credentials. Configuration is never executed or rewritten.

Git state records a baseline on first observation and reports only metadata for
changes after that baseline. Pre-existing changes are excluded. Full diff text
is fetched only after an explicit click, is size bounded, and is not persisted.

Zenith introduces no Control Center credential persistence. Existing OAuth
material remains in memory and credential files are never exposed. Any future
managed organization adapter must use the macOS Keychain and must fail closed;
plaintext fallback is forbidden.

The local audit file is bounded to 1,024 entries and 512 KiB, sanitized before
write, retained for 1–365 days, and uses opaque project references. Corrupt or
oversized audit data recovers to an empty store. Zenith sends no Control Center
telemetry or analytics.

## Quick Panel and failure behavior

The Quick Panel reads only the last cached compact summary (active sessions,
budget alerts, safety finding count, and quality) when activated. It never
refreshes providers, scans projects, evaluates Git, or polls while hidden.

Provider failures retain the last successful observation as Stale and expose a
partial error. Missing permissions and scan boundaries produce Partial results.
Missing/corrupt settings deserialize to safe defaults: all automation and
notifications remain off, AC-only remains on, and no finding is dismissed.
