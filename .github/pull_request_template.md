## Summary

<!-- Describe the change and link its issue(s), e.g. Closes #123. -->

## Version

<!-- Required: old → new and the just recipe used, or a documentation-only no-bump reason. -->

- [ ] Shipped behavior/assets have one version bump for this PR (not each commit).
- [ ] `just check-version` passes and all synchronized manifests are included.

## Verification

<!-- Record actual results. Leave unrun checks unchecked and explain why. -->

- [ ] `cargo check` and `cargo test`
- [ ] `pnpm check`, `pnpm test -- --run`, and `pnpm build`
- [ ] `just build-fast` after the final version/asset change; bundle version checked
- [ ] For branding: `pnpm icons:check` and packaged icon checked (or not applicable)

## Visual evidence and limits

<!-- Link UI/icon previews where relevant. State which platforms/states were verified,
     current CI status, and whether the installed app was replaced. -->
