# Zenith v0.2.0 Release Preparation

This is a local, pre-release working checklist. Keep it uncommitted until the
release workflow and public distribution policy are intentionally reviewed.

## Release goal

Publish Zenith v0.2.0 as an unsigned public beta with downloadable installers
for both supported desktop targets:

- macOS Apple Silicon: unsigned `.dmg`
- Windows x64: unsigned NSIS `-setup.exe`
- SHA-256 checksums and build metadata for every installer
- one GitHub prerelease created from the exact `v0.2.0` tag

The feature PR for issue #106 targets `develop`. Merging that PR does not publish
a release. Public distribution is a later, explicit `develop` to `main` release
flow followed by a version tag.

## Implemented release shape

The issue #106 feature branch extends `.github/workflows/release.yml` with a
shared source-verification job, parallel macOS/Windows packaging jobs, and one
final tagged-release publisher. Confirm that the reviewed PR retains this shape
and that no platform job can publish a competing release independently.

## Required release workflow changes

### macOS release job

- Keep `aarch64-apple-darwin` as the declared target.
- Build the release DMG from the tagged commit.
- Locate the DMG deterministically and fail if zero or multiple candidates are
  found.
- Generate `SHA256SUMS-macos-arm64.txt` and
  `BUILD_INFO-macos-arm64.txt` through the shared release checksum contract.
- Upload the DMG and metadata as a workflow artifact before publishing.
- Continue to label the build as unsigned and unnotarized.

### Windows release job

- Run on `windows-latest` with the stable x64 MSVC Rust toolchain.
- Install Node 20 and pnpm 9 with the lockfile cache.
- Run the same binding drift, frontend, Rust, and version checks used by the
  tagged macOS build.
- Build only the NSIS target for the first public Windows beta:

  ```powershell
  pnpm tauri build --target x86_64-pc-windows-msvc --bundles nsis --config .github/tauri.package-ci.json
  ```

- Locate exactly one `*-setup.exe` under the Tauri bundle output.
- Generate `SHA256SUMS-windows-x64.txt` with
  `scripts/release_checksums.cjs`; do not use PowerShell's default CRLF text
  writer for portable checksum manifests.
- Write `BUILD_INFO-windows-x64.txt` containing version, commit, tag, target,
  unsigned status, and UTC build time.
- Upload the installer, checksum, and build metadata as a workflow artifact.
- Generate the WinGet multi-file manifest from the final installer bytes, but do
  not submit it to the community repository until a later signed release.
- Do not add certificate secrets or a fake signing step. The initial Windows
  beta is intentionally unsigned and enables the SignPath application.

### Combined GitHub prerelease

- Create the GitHub Release only for `refs/tags/v*`, never for an ordinary
  `workflow_dispatch` run.
- Make publishing depend on successful macOS and Windows build jobs.
- Download both platform artifacts into separate directories and attach:
  - the macOS `.dmg`
  - the Windows `-setup.exe`
  - both checksum files
  - both build-info files
- Combine platform manifests with `scripts/release_checksums.cjs`, require LF
  line endings, and run `shasum -c SHA256SUMS.txt` against both binaries before
  the publish action.
- Use generated release notes and keep `prerelease: true` for v0.2.0.
- Ensure concurrent platform jobs do not independently create or overwrite the
  same GitHub Release. A single final publish job should own release creation.

## Unsigned Windows distribution policy

No Microsoft developer account or code-signing certificate is required to
build, download, or run the NSIS installer. However, a browser-downloaded
unsigned executable can show Microsoft Defender SmartScreen warnings and an
`Unknown publisher` label.

The release notes and download section must state:

> Windows beta builds are currently unsigned. Microsoft Defender SmartScreen
> may show “Windows protected your PC.” Verify the published SHA-256 checksum,
> then choose “More info” and “Run anyway” only if the file came from the
> official Zenith GitHub Release.

Also document these limitations:

- Managed company or school devices may block unsigned executables entirely.
- Antivirus reputation systems can produce warnings or false positives for new
  unsigned binaries.
- Users should never download mirrored installers without matching checksums.
- The application must not suggest that the Windows build is signed or trusted
  by Microsoft.

## SignPath Foundation gate after v0.2.0

Zenith is now MIT-licensed and `CODE_SIGNING_POLICY.md` defines the public
policy. SignPath Foundation requires the project to have already released the
artifact form it wants signed, so apply only after the v0.2.0 Windows NSIS asset
is public and verified.

Application preparation:

- Confirm `LICENSE` is MIT and all shipped components are compatible with an
  OSI-approved open-source release.
- Enable multi-factor authentication for every GitHub and SignPath maintainer.
- Protect the release source branch/tag and require pull-request review for
  release workflow, signing policy, and build-script changes.
- Link `CODE_SIGNING_POLICY.md` from the repository home page and release notes.
- Identify authors/reviewers/approvers and ensure the signing team owns and
  maintains this repository.
- Submit the existing v0.2.0 Windows installer, public release URL, repository
  URL, functional description, and signing-policy URL in the application.

After approval, record the exact SignPath organization, project, signing-policy,
and artifact-configuration identifiers as GitHub Actions variables/secrets.
Never invent placeholder identifiers in the committed release workflow. Install
the SignPath GitHub App with only the repository access it requires.

The follow-up signing PR must change the Windows pipeline to:

1. Build the application and current-user NSIS installer on a GitHub-hosted
   Windows runner from a protected version tag.
2. Upload the unsigned artifact for SignPath trusted-build-system origin
   verification.
3. Submit and approve the release-signing request according to the public
   policy.
4. Download the signed result without rebuilding or changing its contents.
5. Verify the executable and installer with `signtool verify /pa /all /v` and
   PowerShell `Get-AuthenticodeSignature`; require a valid timestamp and the
   expected SignPath publisher.
6. Fail closed if the request is denied, times out, or returns an unexpected
   signer. Never fall back to an unsigned public asset.
7. Compute checksums and WinGet hashes only from the verified signed bytes.
8. Publish signed assets through the existing single final release job.

Do not store `.pfx` files, passwords, tenant secrets, private keys, or signing
tokens in Git. Follow `CODE_SIGNING_POLICY.md` for rotation, revocation, and
incident response.

## WinGet submission gate after signing

- Regenerate the versioned multi-file manifest from the signed immutable asset.
- Run `winget validate --manifest <version-directory>` on Windows.
- Run Microsoft's `SandboxTest.ps1` with networking enabled against the public
  GitHub Release URL.
- Test interactive and `/S` installation, `winget install`, same-scope upgrade,
  Start Menu launch, Apps & Features metadata, and silent uninstall on clean
  Windows 10 and Windows 11 x64 machines.
- Open a separate PR to `microsoft/winget-pkgs`; do not make #106's feature PR
  depend on credentials or write access to that external repository.

## Version and source-of-truth checks

The release commit and tag must all agree on `0.2.0`:

- `package.json`
- `src-tauri/Cargo.toml`
- `src-tauri/tauri.conf.json`
- `Cargo.lock` package entry
- generated frontend version display/tests
- Git tag `v0.2.0`

Before opening the release PR:

```bash
just check-version
git diff --exit-code -- src/lib/bindings/tauri.ts Cargo.lock
```

The tagged workflow must reject any tag/manifests mismatch.

## Required pre-release validation

Run from a clean checkout of the intended release commit:

```bash
pnpm install --frozen-lockfile
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo check --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml
pnpm check
pnpm test -- --run
pnpm build
just build-fast
just test-release-installer
just check-version
```

CI must additionally pass:

- Frontend & IPC Contract
- Rust Checks (macOS)
- Rust Checks (Windows x64)
- Package Smoke (macOS)
- Package Smoke (Windows x64 NSIS)

Do not tag while any required check is pending, skipped unexpectedly, or failed.

## Release sequence

1. Finish issue #106 on `feature/106-...` with version `0.2.0`.
2. Open and review a PR from the feature branch into `develop`.
3. Merge only after all required CI checks pass and the user explicitly approves.
4. Complete final product smoke tests from updated `develop`.
5. Confirm the tagged release workflow produces both platform artifacts during
   a manual, non-publishing `workflow_dispatch` rehearsal.
6. Open a dedicated release PR from `develop` into `main`.
7. Confirm the release PR contains only intended v0.2.0 changes.
8. Merge the release PR after explicit approval.
9. Update local `main` with a fast-forward pull.
10. Create an annotated tag on the exact merged `main` commit:

    ```bash
    git tag -a v0.2.0 -m "Zenith v0.2.0"
    git push origin v0.2.0
    ```

11. Watch both platform build jobs and the final publish job.
12. Verify every uploaded artifact and checksum before announcing the release.
13. Apply to SignPath Foundation using the now-public Windows installer and the
    committed code-signing policy.
14. Implement signing verification in a follow-up PR only after SignPath issues
    real project and policy identifiers.

## Post-release smoke checks

### macOS

- Download the DMG from GitHub Releases, not workflow artifacts.
- Verify its SHA-256 checksum.
- Mount it, copy Zenith to Applications, and confirm the expected Gatekeeper
  warning for an unsigned beta.
- Confirm the app launches, both windows render, the tray works, and the version
  shown is 0.2.0.

### Windows

- Download the NSIS setup executable from GitHub Releases.
- Verify its SHA-256 checksum with `Get-FileHash`.
- Confirm SmartScreen/Unknown publisher messaging is expected and accurately
  documented.
- Install on a clean Windows 11 x64 environment with WebView2 available.
- Confirm the Start Menu entry, application launch, tray behavior, version, and
  uninstall entry.
- Exercise read-only features first, then one explicitly reviewed safe mutation.
- Confirm uninstall removes the application without deleting Zenith user data
  unless the installer explicitly offers and documents that choice.

## Rollback and incident response

- If artifacts are incorrect, immediately mark the release draft or prerelease
  as unavailable and remove the bad assets; do not reuse the same tag for a
  different commit.
- Fix forward with a new patch version such as `0.2.1` and a new tag.
- If a checksum mismatch occurs, treat the artifact as compromised until the
  workflow provenance and uploaded file are verified.
- Preserve failed workflow logs and artifact metadata for diagnosis.
- Do not force-move or recreate a published version tag.

## Final release approval gate

Before tagging, explicitly confirm all of the following:

- [ ] The unsigned transition scope for issue #106 is merged into `develop` and
      accepted without falsely closing its signing/WinGet-submission remainder.
- [ ] v0.2.0 version sources are consistent.
- [ ] macOS and Windows CI/package smoke checks pass.
- [ ] The tagged release workflow publishes both platforms.
- [ ] Windows unsigned-beta warnings are visible in release notes.
- [ ] Checksums and build metadata are included.
- [ ] The `develop` to `main` release PR is approved.
- [ ] The exact `main` commit to tag has been identified.
- [ ] The user has explicitly approved creating and pushing `v0.2.0`.
- [ ] After release, the SignPath Foundation application owner and reviewer are
      identified.

## v0.2.0 execution record — 2026-09-02

- PR #114 promoted the verified `develop` tree to `main` at
  `a2f4812f59343f45a7e5d3b7738ad7417f3c25e9`.
- Annotated tag `v0.2.0` resolves to that exact commit.
- Release run `33576449500` passed source verification, macOS ARM64 DMG,
  Windows x64 NSIS, WinGet generation, and the single publish job.
- The public prerelease is
  `https://github.com/jaeyoung0509/zenith/releases/tag/v0.2.0`.
- Downloaded DMG and EXE hashes matched the published values. The first Windows
  checksum manifest contained CRLF; only the checksum text assets were
  normalized to LF and re-uploaded. Installer binaries were not modified.
- Issue #115 and branch `feature/115-release-artifact-portability` cover the
  permanent LF-only checksum generator, publish-time verification, and Node 24
  GitHub Action upgrades.
- Remaining external step: submit the now-eligible project to SignPath
  Foundation using the owner/reviewer roles in `CODE_SIGNING_POLICY.md`.
