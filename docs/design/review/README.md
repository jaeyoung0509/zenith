# Issue 280 visual review

Captured on 2026-09-25 from the browser preview with mocked IPC data. These
images show rendered application UI, not design concepts or native macOS
window chrome. Values and provider states in the after images are preview data.

| Surface | Image | Viewport |
| --- | --- | --- |
| Storage before (user-provided running app capture) | [before-storage.png](before-storage.png) | 962 × 656 capture |
| Overview | [overview.png](overview.png) | 960 × 660 |
| Storage | [storage.png](storage.png) | 960 × 660 |
| Storage category detail | [storage-detail.png](storage-detail.png) | 960 × 660 |
| Performance | [performance.png](performance.png) | 960 × 660 |
| AI Activity | [ai-activity.png](ai-activity.png) | 960 × 660 |
| Settings | [settings.png](settings.png) | 960 × 660 |
| Containers | [containers.png](containers.png) | 960 × 660 |
| Quick Panel | [quick-panel-360.png](quick-panel-360.png) | 360 × 520 |
| Quick Panel stress width | [quick-panel-320.png](quick-panel-320.png) | 320 × 520 |

The browser preview cannot execute native cleanup or show Tauri's overlay title
bar, tray anchor, or Finder/Dock icon. Native bundle creation is verified by
`just build-fast`; those window behaviors need a manual macOS pass.
