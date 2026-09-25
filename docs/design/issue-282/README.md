# Issue 282 visual review

The before Quick Panel image is the running-app screenshot supplied with the
issue request. The before sidebar image is the prior White & Mint browser
preview. After images were captured from `just dev-web` at version 0.3.55 with
mocked IPC data on 2026-09-25. The revised Quick Panel captures show provider
names without mixed brand artwork and a 20 px clipped panel radius.

| Surface | Before | After |
| --- | --- | --- |
| Quick Panel, default 400 × 740 | [Deployed card layout](before-deployed-quick.png) | [All default sections](after-quick-400.png) |
| Quick Panel, 320 × 520 | [Running app](before-quick.png) | [Top](after-quick-320.png) · [AI and lower sections](after-quick-320-ai.png) |
| Quick Panel, 360 × 520 | Same running-app reference | [Full panel](after-quick-360.png) |
| Main sidebar, 960 × 660 | [Previous preview](before-sidebar.png) | [Updated preview](after-sidebar.png) |

At the new 400 × 740 default size, the five preview providers, system metrics,
cleanup summary, and Keep Awake control fit without scrolling. The native
window shrinks to the active display's work area when needed; at smaller sizes,
the body scrolls independently of its fixed header and footer.
The browser preview verifies rendered layout and mocked states. At 360 px,
`html`, `body`, and `#app` all compute to transparent backgrounds; the panel
computes to a 20 px radius and clip path, with no horizontal overflow. The already
running native Zenith window reported version 0.3.54 during this review, so it
does not verify the new 0.3.55 interface. Native window placement, tray
anchoring, and cleanup execution require a separate app pass; `just build-fast`
verifies that the current frontend is embedded in the debug `.app` bundle.
