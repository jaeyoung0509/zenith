# Code signing policy

Zenith is an MIT-licensed open-source project. Official release artifacts are
built from this repository by the GitHub Actions release workflow. The project
does not accept binaries or build scripts from private source repositories.

## Current transition

The Windows x64 public beta is distributed as an explicitly unsigned NSIS
installer. GitHub Release notes, build metadata, and the Windows download
instructions identify it as unsigned and explain the expected Microsoft
Defender SmartScreen warning. Self-signed certificates are not used for public
releases because they do not establish a publicly trusted publisher identity.

The macOS ARM64 beta is unsigned and not notarized. See
[macOS notarization](#macos-notarization) for the dated decision and the exact
user-facing instructions that must accompany an unnotarized build.

After the first Windows installer has been released, the maintainers will apply
for SignPath Foundation open-source code signing. Until that application is
approved and the trusted workflow is configured, no release may claim to be
signed.

After approval, official Windows signatures will carry this disclosure:

> Free code signing provided by
> [SignPath.io](https://about.signpath.io/), certificate by
> [SignPath Foundation](https://signpath.org/).

## Reputation reality

SignPath Foundation issues **organization-validated** certificates. That
establishes a verifiable publisher identity and removes the "unknown publisher"
label, but it does not transfer Microsoft Defender SmartScreen reputation.
SmartScreen reputation is accumulated per certificate and per file hash over
time; a freshly issued certificate starts with none.

Consequences that release notes, README, and download instructions must state:

- The SmartScreen unknown-publisher warning is expected to persist **after the
  first signed release**. Signing is not a switch that makes the warning
  disappear.
- No release note, download page, or application string may claim that signing
  removes the warning or that Microsoft trusts the build.
- Until reputation accumulates, users verify releases by SHA256 checksum and
  GitHub build provenance rather than by publisher reputation alone.

## macOS notarization

**Decision (2026-09-12): macOS public beta artifacts are not notarized.** Zenith
does not hold a paid Apple Developer ID certificate, and nothing in the release
workflow submits the DMG to Apple's notary service or staples a ticket to it.
This is a deliberate, dated decision, not an omission; the plan below records
what has to change if notarization is adopted.

What an unnotarized build means for a user:

- Gatekeeper refuses the first launch. A downloaded, quarantined build shows
  "*Zenith* cannot be opened because the developer cannot be verified" or, for
  some download paths, "*Zenith* is damaged and can't be opened".
- The application bundle is not stapled, so Gatekeeper cannot validate it
  offline even after a network check.
- No operating-system attestation of the binary is available on macOS; the
  Keychain ceiling recorded in [THREAT_MODEL.md](docs/THREAT_MODEL.md) still
  applies.

What the user-facing instructions must say (currently
[README.md](README.md#opening-unsigned-beta-builds-on-macos)):

- The build is unsigned and not notarized, and the warning is expected.
- The exact way past it for one application: Finder → **Applications** →
  right-click **Zenith.app** → **Open** → **Open** again, or System Settings →
  Privacy & Security → **Open Anyway** after the first blocked attempt.
- Never instruct the user to disable Gatekeeper globally, to remove quarantine
  from every downloaded file, or to run `sudo spctl --master-disable`. Clearing
  the quarantine attribute on this one bundle (`xattr -cr /Applications/Zenith.app`)
  may be documented as an alternative, together with the fact that it disables
  Gatekeeper's malware check for that bundle.
- Verify the SHA256 checksum from the GitHub Release before overriding any
  warning.

Plan if notarization is adopted: obtain an Apple Developer ID Application
certificate, store its credentials only as GitHub Actions secrets, submit the
built DMG with `notarytool`, require the notarization result before checksum
generation, verify with `spctl --assess --type execute --verbose` and
`stapler validate`, and fail closed if notarization or stapling does not
succeed. No artifact may claim notarization before that verification exists.
The notarized release must also delete the Gatekeeper workaround instructions
(right-click **Open**, System Settings **Open Anyway**, and `xattr -cr`) from
README.md and any other user-facing document, replacing them with the normal
install steps.

## Release artifact verification

Every published artifact is accompanied by:

- `SHA256SUMS.txt`, combined from the per-platform manifests and the SBOM
  manifest by `scripts/release_checksums.cjs`, which the publishing job verifies
  with `shasum -c` against the downloaded bytes before publishing.
- `SHA256SUMS-macos-arm64.txt` and `SHA256SUMS-windows-x64.txt`, covering the
  macOS DMG and both Windows installers respectively, plus
  `SHA256SUMS-sbom.txt` for the SBOM.
- `SBOM-zenith.spdx.json`, the SPDX software bill of materials generated from
  the locked `Cargo.lock`, `package.json`, and `pnpm-lock.yaml` of the tagged
  commit — the exact dependency set the release was built from.
- GitHub build provenance attestation created by
  `actions/attest-build-provenance` for the installers and SBOM, binding them
  to this repository, the release workflow, and the tagged commit. Verify with
  `gh attestation verify <file> --repo jaeyoung0509/zenith`.
- `endpoint-review.json`, the recorded endpoint-protection submission result
  described below.
- `BUILD_INFO-*.txt`, recording target, version, commit, unsigned status, and
  UTC build time.

An unsigned artifact must never be described as signed, and a signed artifact
must never be published before its Authenticode signature has been verified
from the exact bytes that are uploaded.

## Endpoint-protection review gate

The release process submits the built installers to Microsoft's
endpoint-protection analysis before publication. The analysis is performed
manually through the vendor portal; it cannot run automatically in CI, so the
gate is explicit about who does what:

1. The `release-approval` GitHub environment is attached to the publishing job
   and must require reviewer approval. A maintainer downloads the installers
   from the workflow run artifacts, submits
   `Zenith-windows-x64-setup.exe` and `Zenith-windows-x64-setup-machine.exe` to
   Microsoft's submission portal, and waits for the result.
2. The maintainer records the outcome in the environment's variables:
   `ENDPOINT_REVIEW_STATUS` (`clear` or `detected`),
   `ENDPOINT_REVIEW_REFERENCE` (portal submission identifier),
   `ENDPOINT_REVIEW_REVIEWER` (GitHub login), and `ENDPOINT_REVIEW_DATE`, then
   approves the environment gate.
3. The publishing job runs `node scripts/endpoint_review.cjs record`, which
   hashes the exact artifacts being published and writes
   `endpoint-review.json` next to them.
4. The publishing job runs `node scripts/endpoint_review.cjs verify` immediately
   before the release step. Publication happens only when the recorded status
   is `clear`, the version and commit match the release, and every artifact hash
   matches the reviewed bytes.

What blocks publication: a recorded detection, a missing or unset
`ENDPOINT_REVIEW_STATUS`, an unreadable or malformed record, a version or commit
mismatch, or any artifact whose bytes differ from the reviewed ones. The gate
fails closed, and the release environment must not be bypassed.

## Team roles

- Committer and reviewer: [jaeyoung0509](https://github.com/jaeyoung0509)
- Signing approver: [jaeyoung0509](https://github.com/jaeyoung0509)
- Dependency-incident triager: [jaeyoung0509](https://github.com/jaeyoung0509)

Every release signing request and every `release-approval` environment approval
requires manual approval by the named approver. Additional maintainers must be
named here before receiving a release role.

## Privacy and end-user changes

Zenith will not transfer information to other networked systems unless
specifically requested by the user or the person installing or operating it.
Provider APIs and official provider CLIs may access their own network services
only when the user enables or invokes those integrations; their respective
privacy policies then apply. Zenith itself has no telemetry, analytics, or
background tracking service and performs no background network activity,
including update checks.

Cleanup, process termination, Keep Awake, and other system-changing actions are
presented to the user and require the bounded confirmations documented in the
repository's safety architecture. The NSIS installers provide a standard
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
- The application executable and both NSIS installers are signed and
  timestamped. The release workflow verifies their Authenticode signatures with
  `signtool verify /pa /all /v` and `Get-AuthenticodeSignature` before computing
  checksums or publishing artifacts. Checksums, WinGet hashes, provenance
  attestations, and the SBOM are computed only from the verified signed bytes.
- Only the verified signed per-user installer is referenced by a WinGet
  manifest.
- The endpoint-protection review gate must pass for the signed artifacts, and
  the recorded provenance and SBOM must describe the signed bytes.
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

## Dependency and supply-chain incidents

Zenith ships no updater and performs no background network activity, so a
dependency-originated fix can only reach users through a new release that they
can find on the Releases page. The triage path is therefore explicit:

- **Who triages.** The dependency-incident triager named under
  [Team roles](#team-roles) owns every advisory found by CI, Dependabot, or an
  external report, including advisories in transitive dependencies.
- **Detection.** `.github/workflows/ci.yml` runs `just supply-chain` on every
  push and pull request: `cargo deny check advisories bans licenses sources`
  enforces [deny.toml](deny.toml), `cargo audit` re-checks the full lockfile
  (including optional and development subtrees), and
  `pnpm audit --audit-level=high` covers the frontend lockfile. A RustSec
  vulnerability or an npm advisory at or above `high` fails the job.
- **Timebox.** A failing advisory is triaged within five business days of the
  CI failure or report. A vulnerability that is reachable in a shipped code
  path is fixed and released as a patch version within thirty days; a lower
  severity or unreachable advisory may hold until the next scheduled release.
- **Accepting an advisory.** An advisory that cannot yet be fixed is accepted
  only in `deny.toml`, with the advisory id (or exact `crate@version` for a
  yanked release), a written reason, and the platform or code-path reasoning
  that makes it non-impacting. Every accepted entry is reviewed whenever the
  dependency set changes and whenever the advisory's own data changes. There is
  no blanket severity downgrade: entries are individual and reviewable.
- **Reaching users.** A fixed release is published as a new immutable versioned
  tag; release assets are never replaced in place. The application exposes the
  release URL through `PlatformContext.releases_url` and the interface links to
  it, so a user who suspects a defect can open the Releases page and compare
  versions. Embargoed reports are coordinated through GitHub Security
  Advisories, and the advisory is published together with the fixed release.

## Antivirus guidance

Zenith never asks users to add an antivirus exclusion, and no repository
document may instruct one. The only Windows security setting a feature may
require is Controlled Folder Access: the Large Files inspector and Trash plans
operate on folders that Controlled Folder Access protects, and the application
explains that specific setting when an operation is denied. General exclusion
of the install directory, the application, or the user profile is not an
acceptable workaround and must not be documented or suggested.
