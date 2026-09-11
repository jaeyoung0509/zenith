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
  callback requires a high-entropy `state` value, verifies the peer is
  loopback, and bounds the bytes it reads. An unsolicited request cannot abort a
  pending legitimate flow.
- **Issued credentials are either persisted or revoked.** If the credential
  store cannot persist a freshly issued OpenRouter key, Zenith revokes the key
  at the provider. Disconnecting attempts provider revocation and reports when
  it could not.
- **Third-party files keep their permissions.** Rewriting an agent settings
  file preserves the original owner-only mode, and a failed rename removes the
  temporary copy.
- **Diagnostics are redacted on every exit.** One shared pattern table backs
  both the log sanitizer and the AI Control Center secret scanner. Log and audit
  files are created owner-only, and subprocess or provider error text is
  sanitized before it is returned across IPC.
- **Display paths are masked.** Identity paths render as `~/…` or
  `.../basename`; the diagnostics clipboard export cannot carry the home
  directory or user name. Storage and cleanup views that exist to show a
  location are explicit, documented exceptions.

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
  plaintext file fallback is explicitly not acceptable. The OpenRouter flow
  retrieves a key and then revokes it if persistence fails.
- **Same-user process inspection is cooperative.** Agent activity and memory
  features rely on OS process metadata. They do not provide an isolation
  boundary against code already running as the same user.
- **Unsigned distribution.** Authentication, notarization, and auto-update
  integrity are tracked outside this document; until signing lands, do not
  assume operating-system attestation of the binary.

## Out of scope

- A compromised operating system, kernel, or debugger attached to Zenith.
- Network attackers who can terminate TLS or modify provider responses.
- Adversarial third-party configuration that the user explicitly executes
  outside Zenith. Zenith never executes or rewrites discovered configuration.
