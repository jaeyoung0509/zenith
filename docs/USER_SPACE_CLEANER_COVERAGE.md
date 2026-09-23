# User-space cleaner coverage

This manifest records what Zenith may inspect and mutate without crossing into
system maintenance. It is organized by storage owner, not by filename or the
programming language displayed in the interface.

## Boundary

This feature covers only current-user storage. It does not add cleanup targets
under `/System`, `/Library`, `/private/var`, Windows system directories,
machine-wide service stores, or another user's profile. It does not flush DNS,
rebuild Spotlight or LaunchServices, restart services, repair permissions,
vacuum application databases, or elevate privileges.

`CleanerFamily::System` is outside this coverage project. A system-owned
location can be observed by an existing advisory rule, but this manifest grants
it no mutation authority.

## Dispositions

| Mode | Authority | Result |
| --- | --- | --- |
| `tool_managed` | The owner exposes a bounded path query and cleanup command | Fixed backend provider; never a filesystem fallback |
| `full` | A platform or vendor documents a narrow subtree as reproducible cache | Opaque plan plus scan/plan/execution filesystem guards |
| `project_only` | Project markers prove both the root and generated unit | Developer Artifacts only |
| `advisory` | The store is mixed, version-sensitive, interactive, or insufficiently documented | Bytes may be reported; selection and deletion are disabled |
| `manual` | The unit contains user state or requires a semantic choice | Dedicated workflow or guidance only |

## Coverage matrix

| Workstream | Owner or unit | Platforms | Mode | Lifecycle guard | Consequence / non-targets | Evidence |
| --- | --- | --- | --- | --- | --- | --- |
| Applications | `~/Library/Caches/<app>` and sandbox `Library/Caches` entries | macOS | `full`, age-gated | Owning app not running; per-entry age and identity checks | Application Support, documents, preferences, saved sessions and containers themselves are not targets | [Apple cache directory contract](https://developer.apple.com/library/archive/documentation/FileManagement/Conceptual/FileSystemProgrammingGuide/AccessingFilesandDirectories/AccessingFilesandDirectories.html) |
| Applications | Narrow `Cache`, `Code Cache`, `GPUCache`, `ShaderCache` subtrees | macOS / Windows | `full`, age-gated | Owning app not running; whole unit must satisfy age policy | `CacheStorage`, Service Workers, local storage and profile state are excluded | Apple contract; Chromium profile boundary below |
| Browsers | Chromium and Firefox transport/code caches | Windows | `full`, Rebuild and opt-in | Browser not running; per-profile cache subtree only | Cookies, history, bookmarks, passwords, extensions, sessions, downloads and offline state are protected | [Chromium user-data and cache directories](https://chromium.googlesource.com/chromium/src/+/HEAD/docs/user_data_dir.md) |
| Automation | Playwright browser binaries | macOS / Windows | `advisory` | Playwright owns client references and garbage collection | Generic recursive deletion is forbidden | [Playwright browser management](https://playwright.dev/docs/browsers#managing-browser-binaries) |
| Developer | Xcode DerivedData and documented Xcode caches | macOS | `full`, Rebuild | Xcode/build processes not running | Archives, simulators and device data remain Manual | Apple developer-tool ownership; existing catalog policy |
| Developer | Project build outputs | macOS / Windows | `project_only` | Direct project markers and explicit review | Never promoted into Cleanup | `docs/ARCHITECTURE.md` Developer Artifacts contract |
| Package managers | Go build/module, Cargo, npm, pnpm, uv, Composer | macOS / Windows as supported | `tool_managed` | Owner process stopped; fresh discovery and post-action verification | Re-download or rebuild cost is Rebuild | `docs/CACHE_SUPPORT.md` provider contracts |
| Package managers | Bun | macOS / Windows | `advisory` | Current CLI cache discovery requires project context | Zenith does not invent a project context or infer deletion authority from the default path | [Bun package-manager CLI](https://bun.sh/docs/pm/cli/pm) |
| Package managers | pip download/wheel cache | macOS / Windows when `pip3` is available | `tool_managed` | pip/Python processes stopped | Packages and wheels may be downloaded or rebuilt | [pip cache commands](https://pip.pypa.io/en/stable/cli/pip_cache/) |
| Package managers | NuGet HTTP, temp, plugin and global-package resources | macOS / Windows when .NET is available | `tool_managed`, separate units | dotnet/MSBuild/Visual Studio/NuGet processes stopped | Global packages are not conflated with disposable HTTP/temp caches | [dotnet nuget locals](https://learn.microsoft.com/en-us/dotnet/core/tools/dotnet-nuget-locals), [NuGet cache semantics](https://learn.microsoft.com/en-us/nuget/consume-packages/managing-the-global-packages-and-cache-folders) |
| Package managers | Yarn | macOS / Windows | `advisory` | Future adapter must distinguish Classic, modern global cache and project Zero-Installs | Project `.yarn/cache` and offline mirrors are not cleanup targets | [Yarn cache clean](https://yarnpkg.com/cli/cache/clean), [Yarn cache strategies](https://yarnpkg.com/features/caching) |
| Package managers | Gradle | macOS / Windows | `advisory` | Gradle owns locking and periodic retention | Never recursively delete `~/.gradle/caches` | [Gradle-managed caches](https://docs.gradle.org/current/userguide/directory_layout.html) |
| Package managers | Maven and remaining mixed stores | macOS / Windows | `advisory` | Requires an owner-specific, non-project-destructive contract | Local repositories may contain installed or project-owned state | Source list in `docs/CACHE_SUPPORT.md` |
| AI / IDE | Cursor renderer/code/GPU cache subpaths | macOS / Windows | `full`, Rebuild | Cursor not running; exact subpaths only | Logs, extensions, sessions, settings and credentials are not automatic | Same narrow cache-subtree policy as other Electron applications |
| AI / CLI | Claude, Gemini, Codex, Aider and OpenCode roots | macOS / Windows where present | `advisory` | No automatic mutation | Sessions, prompts, auth, settings and logs may be mixed with transient data | No sufficiently narrow upstream cleanup contract is currently registered |
| AI / runtime | Documented compiled GPU/kernel caches | Platform-specific | `full`, Rebuild | Runtime processes stopped; exact documented root | Weights, datasets, sessions and optimized model engines are Manual | Primary runtime sources in `docs/CACHE_SUPPORT.md` |
| Models | Hugging Face detached revisions | Future provider | `advisory` today | Owner dry-run must prove the revision is detached | Whole models, datasets and selected revisions remain Local Models decisions | [Hugging Face cache CLI](https://huggingface.co/docs/huggingface_hub/guides/cli#hf-cache-prune) |

## Provider contract

Every command-backed provider must:

1. Resolve an executable only from trusted installation roots.
2. Remove path-changing environment overrides unless the described platform
   environment models them explicitly.
3. Use fixed, non-interactive arguments and bounded output/time.
4. Discover a single absolute path and validate that it belongs to the current
   user's approved cache roots.
5. Refuse cleanup while an owner process can be using the store.
6. Rediscover and compare the path immediately before mutation.
7. Run the owner command without falling back to filesystem deletion.
8. Rediscover and measure afterward; incomplete measurements never become an
   exact reclaimed-byte claim.

Missing tools and local provider failures affect only their own item. They do
not invalidate unrelated scan or plan targets.

## Interface contract

- Only `Safe` items may be selected by default.
- Re-download, rebuild, re-index, recompile, or first-launch performance costs
  are `Rebuild` and opt-in.
- Advisory and Manual units remain visible but unselected.
- The Storage overview shows each category's name, item count, and currently
  cleanable bytes. Engine risk tiers and incomplete-measurement diagnostics do
  not compete with that decision on the overview. The detail view explains
  concrete consequences when a user selects an item.
- Partial and inaccessible scans keep
  `selected_bytes <= cleanable_bytes <= observed_bytes` and state why a lower
  bound is being shown.
