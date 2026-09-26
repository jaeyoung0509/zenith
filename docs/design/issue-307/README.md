# Quick Panel partial scan explanations

## Cause

A partial inventory can contain no automatic Safe candidates. Quick Panel cleanup
uses backend `auto_cleanable` dispositions with positive cleanable bytes and the
user's enabled categories. Reviewable items and access-blocked observations do
not authorize automatic cleanup. Previously this state only offered Scan Again,
which could not explain unchanged access restrictions or reach reviewed cleanup.

A local read-only diagnostic reproduced a partial inventory with zero automatic
candidates and both access gaps and reviewable observations. Its process permissions
can differ from the installed application; it is supporting evidence, not an exact
capture of that application's inventory. No real cleanup was performed.

## Change

The empty partial state opens Details in the panel. The dialog shows typed gap
counts and exclusion reasons, points reviewable items to Storage, and retains an
explicit rescan action. Partial inventories with verified automatic candidates
still offer Review Safe. No backend authorization or capability was broadened.

## Verification

Regression tests cover typed explanations, reviewable/blocked exclusion, positive
byte automatic candidates, dialog actions, and the Quick Panel's empty partial
state. See the unified PR for combined build/test results. Native permission
changes and real filesystem deletion are intentionally not part of verification.

Browser QA on the unified branch used synthetic observations at 400 × 740:
Details opened as a modal, all actions fit without horizontal overflow, and Open
Storage dispatched navigation. The existing mocked partial inventory still opened
Review Safe with selectable verified candidates. `quick-details.png` records the
synthetic empty-candidate dialog; it does not demonstrate native glass rendering.
