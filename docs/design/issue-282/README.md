# Issue 282 visual review

The before Quick Panel image is the running-app screenshot supplied with the
issue request. The before sidebar image is the prior White & Mint browser
preview. After images were captured from `just dev-web` at version 0.3.55 with
mocked IPC data on 2026-09-25.

| Surface | Before | After |
| --- | --- | --- |
| Quick Panel, 320 × 520 | [Running app](before-quick.png) | [Top](after-quick-320.png) · [AI and lower sections](after-quick-320-ai.png) |
| Quick Panel, 360 × 520 | Same running-app reference | [Full panel](after-quick-360.png) |
| Main sidebar, 960 × 660 | [Previous preview](before-sidebar.png) | [Updated preview](after-sidebar.png) |

The Quick Panel's body scrolls independently of its fixed header and footer.
The browser preview verifies rendered layout and mocked states. The already
running native Zenith window reported version 0.3.54 during this review, so it
does not verify the new 0.3.55 interface. Native window placement, tray
anchoring, and cleanup execution require a separate app pass; `just build-fast`
verifies that the current frontend is embedded in the debug `.app` bundle.
