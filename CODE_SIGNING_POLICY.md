# Code signing policy

Zenith is an MIT-licensed open-source project. Official release artifacts are
built from this repository by the GitHub Actions release workflow. The project
does not accept binaries or build scripts from private source repositories.

## Current transition

The first Windows x64 public beta is distributed as an explicitly unsigned
NSIS installer. GitHub Release notes, build metadata, and the Windows download
instructions must identify it as unsigned and explain the expected Microsoft
Defender SmartScreen warning. Self-signed certificates are not used for public
releases because they do not establish a publicly trusted publisher identity.

After the first Windows installer has been released, the maintainers will apply
for SignPath Foundation open-source code signing. Until that application is
approved and the trusted workflow is configured, no release may claim to be
signed.

After approval, official Windows signatures will carry this disclosure:

> Free code signing provided by
> [SignPath.io](https://about.signpath.io/), certificate by
> [SignPath Foundation](https://signpath.org/).

## Team roles

- Committer and reviewer: [jaeyoung0509](https://github.com/jaeyoung0509)
- Signing approver: [jaeyoung0509](https://github.com/jaeyoung0509)

Every release signing request requires manual approval by the signing approver.
Additional maintainers must be named here before receiving a release role.

## Privacy and end-user changes

Zenith will not transfer information to other networked systems unless
specifically requested by the user or the person installing or operating it.
Provider APIs and official provider CLIs may access their own network services
only when the user enables or invokes those integrations; their respective
privacy policies then apply. Zenith itself has no telemetry, analytics, or
background tracking service.

Cleanup, process termination, Keep Awake, and other system-changing actions are
presented to the user and require the bounded confirmations documented in the
repository's safety architecture. The NSIS installer provides a standard
Windows Apps & Features uninstaller.

## Signed release requirements

Once SignPath Foundation approves Zenith, Windows release signing must follow
all of these rules:

- Signing requests originate only from the reviewed release workflow in this
  repository and run on GitHub-hosted runners.
- The requested source revision is a protected `v*` tag whose version matches
  `package.json`, `src-tauri/Cargo.toml`, `Cargo.lock`, and
  `src-tauri/tauri.conf.json`.
- SignPath origin verification binds the request to the repository, workflow,
  commit, and release tag. Signing credentials and private keys never enter the
  repository or ordinary build logs.
- The application executable and NSIS installer are signed and timestamped.
  The release workflow verifies their Authenticode signatures before computing
  checksums or publishing artifacts.
- Only the verified signed installer is referenced by a WinGet manifest.
- A failed, denied, or unverifiable signing request fails closed. The workflow
  must not silently publish an unsigned installer as a signed release.

The SignPath organization, project, signing-policy, and artifact-configuration
identifiers will be configured only after approval. They must be stored as
GitHub Actions variables or secrets, never hard-coded as guessed values.

## Review and incident handling

Release workflow, signing policy, and dependency changes require pull-request
review. Maintainers responsible for signing must use multi-factor authentication
for GitHub and SignPath. If a release artifact, signing request, account, or
credential may be compromised, maintainers must stop publication, remove the
affected release asset, contact SignPath Foundation for certificate or signature
revocation guidance, and publish a corrected version rather than replacing an
immutable versioned asset.

Security concerns can be reported privately through GitHub Security Advisories
for this repository. General release problems can be reported through
[GitHub Issues](https://github.com/jaeyoung0509/zenith/issues).
