# PR #24 Code Review

- PR: https://github.com/jaeyoung0509/zenith/pull/24
- Base / head: `develop` ← `feature/23-settings-ui-reliability`
- Reviewed scope: settings persistence and theme lifecycle, reusable selection controls, Storage CTA layout, Memory process search, Quick Panel provider projection, and added tests
- Review disposition: **Request changes before merge**
- Code changes from this review: **None** — this document contains review feedback only

## Executive summary

PR #24 addresses the visible requirements of #23 in a generally sensible direction. The cleanup and process-termination safety boundaries remain intact, the Memory search is implemented as a local derived filter, the primary Storage CTA no longer repeats the selected byte count, and the Quick Panel correctly avoids an automatic AI refresh when no providers are configured.

However, two functional issues should be fixed before merge:

1. Settings rows are reordered without keyed `{#each}` blocks. After disabling a provider, Checkbox component instances are reused for different providers, so the rendered checked state can disagree with the saved state and a subsequent click can toggle the wrong provider.
2. Settings save failure handling can reject an event-handler promise and an older failed save can roll back newer optimistic state. The new success-path test does not cover either failure mode.

There are also several test-quality and visual consistency gaps. In particular, some new tests only verify objects/functions declared inside the test itself rather than production behavior.

## Findings

### [P1] Key Settings rows by stable IDs before allowing immediate reorder

**Affected code**

- `src/routes/dashboard/SettingsView.svelte`
- `orderedDashboardTabs()` list
- `orderedSections()` list
- `orderedProviders()` list

All three lists place enabled entries first and disabled entries last, but render them with unkeyed blocks:

```svelte
{#each orderedProviders() as provider}
```

When a checkbox changes, `settings.quick_panel_ai_providers` changes immediately and `orderedProviders()` moves that row. Because the block is unkeyed, Svelte is allowed to reuse the existing `Checkbox` component at each array index for a different provider.

**Observed reproduction**

1. Start with all AI providers enabled.
2. Disable Claude Code, OpenCode, and OpenRouter in sequence.
3. Observe the row order and checkbox states after each click.
4. Compare the visible state with `quick_panel_ai_providers` persisted by the mock API.

An observed state was:

- persisted IDs: `['codex', 'antigravity']`
- UI: Antigravity row was ordered with enabled entries but displayed unchecked

This also explains the contradictory state visible in the supplied Settings screenshot: Antigravity appears near the enabled group while its checkbox looks disabled.

**Impact**

- The UI can claim a provider is disabled when it is saved as enabled.
- Rapid sequential clicks may operate on a row whose component identity no longer matches its provider.
- Quick Panel output may appear inconsistent with Settings even when persistence itself succeeded.

**Recommended change**

Key every reorderable row by its stable ID:

```svelte
{#each orderedProviders() as provider (provider.id)}
{#each orderedSections() as option (option.id)}
{#each orderedDashboardTabs() as tabOption (tabOption.id)}
```

Add an interaction-level regression test that toggles multiple providers and asserts both the visible checked state and final saved ID order after each reorder. A pure `toggleOrdered()` unit test does not exercise this bug.

### [P1] Make save rollback revision-aware and do not leak rejected event promises

**Affected code**

- `src/lib/stores/settings.svelte.ts`
- `SettingsStore.save()`

The save queue serializes writes, but each caller independently handles its own `currentSave` failure:

```ts
try {
  await currentSave;
} catch (error) {
  this.error = ...;
  this.settings = this.persistedSettings ?? previousSettings;
  this.applyTheme(this.settings.theme);
  throw error;
}
```

There are two related problems.

#### A. Rejected UI event promises

Settings handlers generally invoke async store methods without awaiting/catching them, for example:

```svelte
onchange={() => settingsStore.toggleQuickPanelProvider(provider.id)}
```

Re-throwing from `save()` therefore creates an unhandled rejection after the store has already populated `settingsStore.error`. The component has an error UI, so propagating an unhandled event promise adds noise without giving the user a better recovery path.

#### B. An older failure can overwrite newer optimistic state

Consider two rapid changes:

1. Save A captures snapshot A.
2. Save B captures snapshot B, which includes A plus a newer change.
3. A fails.
4. A's catch handler restores `persistedSettings`/`previousSettings` even though B is already queued.
5. B may later persist snapshot B, but its success path does not restore `this.settings` to B.

The final disk/backend state can therefore be newer than the rendered Settings state.

**Recommended change**

- Track a monotonically increasing save revision/generation.
- Only the latest failed revision may roll back the optimistic UI.
- If an older revision fails while a newer full snapshot is queued, allow the newer save to complete without clobbering current UI state.
- On latest failure, populate `settingsStore.error`, roll back/apply the last persisted theme, and resolve the UI event operation instead of rethrowing an unhandled rejection.
- Clear the error when the latest queued snapshot succeeds.
- Inject or otherwise mock the settings persistence dependency so tests can deterministically reject the first or latest write.

**Required regression cases**

- Latest save fails: UI rolls back, theme rolls back, error is visible, and no unhandled rejection escapes.
- Save A fails and newer Save B succeeds: final UI and persisted snapshot both equal B.
- Save A and Save B both fail: latest failure rolls back to the last known persisted snapshot.
- Three rapid successful saves: the final saved snapshot contains all three changes in order.

### [P2] The snapshot helper does not actually call Svelte's `$state.snapshot`

**Affected code**

- `src/lib/utils/settings.ts`

The helper checks `globalThis.$state?.snapshot`, but `$state` is a Svelte compiler rune, not a normal runtime global API. In normal app execution this condition is expected to be false:

```ts
const snap = typeof (globalThis as any).$state?.snapshot === 'function'
  ? (globalThis as any).$state.snapshot(settings)
  : settings;
```

The explicit field-by-field copying that follows does create plain arrays and plain `awake_rules`, so the original `structuredClone(Proxy)` failure is probably avoided. The misleading runtime probe should still be removed or replaced.

**Recommended change**

Choose one clear strategy:

- call `$state.snapshot(this.settings)` inside the `.svelte.ts` store where the rune is compiled, then validate/serialize the result; or
- keep the explicit typed serializer and document that it intentionally unwraps every field without relying on `$state.snapshot`.

Do not imply that a `globalThis.$state` runtime path is being exercised. Add a test using an actual `$state` value from the store rather than only a plain object fixture.

### [P2] New tests do not consistently exercise production code

**Affected code**

- `src/test/controls.test.ts`
- `src/test/quickPanel.test.ts`
- `src/test/settings.test.ts`

#### Controls test

`controls.test.ts` constructs plain objects and asserts their own literal values:

```ts
const switchProps = { color: 'peer-checked:bg-emerald-500', ... };
expect(switchProps.color).toContain('emerald');
```

This test still passes if `Switch.svelte` is changed back to a white track or loses its accessible input entirely.

Render the actual `Switch.svelte` and `Checkbox.svelte` components using an SSR/DOM-capable test, then assert the actual accessible name, `checked`/`disabled` attributes, semantic classes, and checked icon/thumb state.

#### Quick Panel provider test

`quickPanel.test.ts` declares a private `projectProviders()` implementation inside the test and tests that duplicate. `QuickPanel.svelte` does not import that function, so the production projection can regress while the test remains green.

Extract the projection to a production utility used by `QuickPanel.svelte`, or add a component test that renders the panel with configured IDs and a usage snapshot.

#### Settings save test

The rapid-save test uses the normal mock API and only checks final in-memory fields. It does not assert how many persistence calls occurred, which snapshots were sent, or any failure behavior. Mock/inject persistence and assert the actual queued snapshots.

### [P2] Light theme leaves a hard-coded dark document body

**Affected code**

- `index.html`

The root document body remains permanently dark:

```html
class="... bg-[#121216] text-[#fafafa] ..."
```

The dashboard normally covers the body, but the hard-coded colors can remain visible in native titlebar/overscroll/transparent rounded-corner areas, especially in the Quick Panel. Once the `dark` class is removed, the document background should follow the same semantic tokens as the app.

**Recommended change**

Use `bg-background text-foreground` on the body and retain the initial dark HTML class only as the current startup fallback. Verify main and Quick windows in native Light mode, including rounded transparent edges and overscroll.

### [P2] Storage toolbar still wraps a single utility into an orphaned second row at 960 × 660

**Affected code**

- `src/routes/dashboard/StorageView.svelte`

The primary CTA and duplicated byte label are improved. At the documented 960 × 660 baseline, however, the left utility group can still wrap only `Disk Utility` onto a second line while the cleanup CTA remains on the right. This looks accidental rather than like a deliberate responsive composition.

**Recommended change**

Keep the primary CTA right-aligned, but make the secondary toolbar fit or stack intentionally. Options include:

- use a labeled group on one row with tighter spacing;
- move Disk Utility to the mounted-volumes header; or
- use an accessible icon-only Disk Utility action with both `aria-label` and tooltip.

Recheck safe-only, mixed Safe/Rebuild, zero-selection, scanning, and cleaning states at 960 × 660 and at the supported narrow breakpoint.

### [P3] Memory row key should use an explicit delimiter

**Affected code**

- `src/routes/dashboard/MemoryView.svelte`

The row key is currently:

```svelte
(proc.pid + proc.name)
```

Because one operand is a string, this concatenates values without a boundary. Different PID/name pairs can theoretically form the same key. Use a delimited key such as `` `${proc.pid}:${proc.name}` ``. This is low risk because real process names make collisions unlikely, but the change is trivial and clarifies intent.

## What is implemented well

### Safety boundaries are preserved

- No Rust cleanup planner/executor or signature-scope code is changed.
- Storage remains UI/layout work; the frontend does not receive path, strategy, or identity authority.
- Memory search only filters the backend-provided `top_processes` collection.
- Quit still sends the selected process group name through the existing allowlisted backend flow; no arbitrary PID kill command is introduced.
- Hidden Quick Panels still avoid memory polling and provider collection according to the existing activation lifecycle.

### Memory search behavior is appropriately local

- `filterProcesses()` is pure and case-insensitive.
- Name and partial PID matching are covered.
- The query remains independent from the 2.5-second polling updates.
- No-results and clear actions are present and accessible.
- The termination confirmation path is unchanged.

### Quick Panel zero-provider behavior is correct

- Automatic AI usage refresh is skipped when no providers are enabled.
- `Configure in Settings.` is rendered before the loading/snapshot branches, preventing stale provider rows from appearing in that state.

### Storage CTA copy is clearer

- The selected byte total is no longer repeated inside the primary button.
- `Clean Safely` and `Review & Clean` correctly distinguish safe-only and mixed Rebuild selections.
- Risk composition remains visible adjacent to the CTA.

## Verification performed during review

### Automated checks on the submitted PR revision

- `cargo check`: passed
- `cargo test`: passed (`34` unit tests passed, `1` ignored; `19` safety tests passed; `7` integration/unit tests passed)
- `pnpm check`: passed with `0` errors and `0` warnings
- `pnpm test -- --run`: passed (`8` files, `45` tests on the original submitted revision)
- `pnpm build`: passed
- `git diff --check`: reported one trailing blank line at EOF in `src/lib/stores/settings.svelte.ts`; fix before merge

### Browser-preview observations

- Storage CTA label no longer duplicates the byte total.
- Memory name filtering and PID filtering remained active across a polling interval.
- Theme selection worked once the settings save path completed.
- Zero configured providers produced `Configure in Settings.` in the Quick Panel.
- The unkeyed provider list reproduced a visible/saved checkbox mismatch during sequential toggles.

Browser preview cannot prove the native multi-webview activation boundary, titlebar rendering, or app-config persistence. Those require a Tauri build.

## Required manual verification before merge

1. Build/open the native debug `.app` with `just build-fast` / `just run-fast`.
2. In the main window, switch System → Light → Dark and recreate the main window after each selection.
3. Change macOS appearance while System is selected; confirm both main and Quick webviews update.
4. Enable only Codex, close the main window, and reopen the persistent Quick Panel; confirm only Codex is shown.
5. Disable every provider; confirm no provider CLI/API refresh occurs and the configuration empty state appears.
6. Toggle multiple providers/sections/tabs rapidly and verify visible checkbox state, saved JSON state, and order agree after every reorder.
7. Force a settings write failure (test seam or unwritable fixture) and confirm rollback/error behavior without an unhandled rejection.
8. Verify Storage at 960 × 660 in safe-only, mixed-risk, zero-selection, scanning, and cleaning states.
9. Search Memory by mixed-case name and partial PID, wait through multiple polling ticks, clear the query, and exercise normal Quit confirmation on a permitted user app.

## Merge recommendation

Do not merge until the two P1 findings are resolved and covered by regression tests:

- stable keyed identity for reorderable Settings rows;
- revision-aware, non-leaking settings save failure handling.

After those fixes, address the P2 test gaps and native Light-mode/Storage layout verification, rerun the complete handoff suite, and require a green macOS CI result. No merge action was performed as part of this review.
