# Cache and Runtime Support Matrix

This matrix is the source of truth for cache ownership. `full` means Zenith can
delete a narrowly scoped generated directory, `project_only` requires direct
project markers, `tool_managed` invokes an owner CLI with fixed arguments, and
`advisory` is inventory/documentation only. `not_applicable` means there is no
generic language-owned cache worth guessing. Rebuildable entries are never
selected by default.

Physical allocated bytes are shown where the filesystem exposes them. Values
are typed as physical reclaimable, conservative lower bound, or informational.
Hard-linked/deduplicated provider stores are informational until the owner GC
runs; they are never presented as promised free-space recovery.

Each catalog entry also declares its backend owner family. Package-manager
stores belong to `package_managers`, compiler/build caches to `developer`, and
container resources to `containers`; a missing or incompatible family is a
catalog-load error. The interface category does not grant mutation authority.
Broad temporary-directory prefixes remain advisory because a matching name and
age do not establish who owns the contents or whether they are recoverable.
On macOS, Homebrew is excluded from the broad application-cache rule. A
dedicated owner-scoped provider offers direct, single-linked downloaded files
under the default `~/Library/Caches/Homebrew/downloads` for explicit Rebuild
review. It re-enumerates each selected file, checks identity and the running
owner, and reports the removal outcome. `api`, `bootsnap`, unfamiliar entries,
and the rest of the cache stay advisory. Homebrew's `brew cleanup` also manages
old installed formula versions, so its dry-run is a different operation and
must not be presented as the estimate for the download-file review. The
DotSlash stays excluded from broad cache deletion. Its separate owner-scoped
adapter offers only completely measured hash-addressed artifacts unchanged for
30 days, when Intensive cleanup is enabled and the user reviews each item.
The default is off, and an overridden `DOTSLASH_CACHE` root is unsupported.

The cross-workstream application, browser, automation, package-manager, and AI
decision record is maintained in
[USER_SPACE_CLEANER_COVERAGE.md](USER_SPACE_CLEANER_COVERAGE.md). That manifest
also states the system-maintenance boundary for this coverage.

## Programming-language ecosystems

| Priority | Language | Actual cache owner(s) | Zenith mode on macOS / Windows | Reason or consequence |
| ---: | --- | --- | --- | --- |
| 1 | TypeScript | npm, pnpm, Yarn, Bun; project build tools | npm/pnpm `tool_managed`; Yarn/Bun `advisory`; project outputs `project_only` | packages may download again; the language owns no global cache |
| 2 | JavaScript | npm, pnpm, Yarn, Bun; project build tools | same as TypeScript | shared with TypeScript; never count the same provider twice |
| 3 | Python | uv, pip, Poetry, Conda; virtual environments | uv/pip `tool_managed`; Poetry/Conda `advisory`; environments `project_only` | interpreter/version ownership must be preserved |
| 4 | Java | Gradle, Maven | project outputs `project_only`; shared stores `advisory` | Gradle owns automatic GC; Maven purge is project-scoped |
| 5 | C# | NuGet; MSBuild `bin`/`obj` | project outputs `project_only`; NuGet typed resources `tool_managed` | restore and compile; global packages remain a distinct explicit Rebuild unit |
| 6 | PHP | Composer | Composer cache `tool_managed`; `vendor` `project_only` | packages and metadata may download again |
| 7 | Shell | concrete tools invoked by scripts | `not_applicable` | shell itself has no language-owned cache |
| 8 | C++ | CMake, Conan, vcpkg and compiler caches | project outputs `project_only`; shared stores `advisory` | compile/link or dependency restore |
| 9 | Go | Go build and module caches | both `tool_managed` | `go clean -cache` recompiles; `-modcache` re-downloads modules |
| 10 | C | CMake, Conan, vcpkg and compiler caches | project outputs `project_only`; shared stores `advisory` | shared with the C++ native-build family |
| 11 | Kotlin | Gradle, Maven | project outputs `project_only`; shared stores `advisory` | shared JVM owner family, not a Kotlin-specific global cache |
| 12 | Rust | Cargo; rustup | Cargo typed owner provider / `project_only`; rustup `advisory` | crate download/compile; installer state is not generic cache |
| 13 | SQL | database/application engines | `not_applicable` | databases and journals are structured state, never language cache |
| 14 | Ruby | RubyGems, Bundler | project bundle `project_only`; shared gems `advisory` | installed gems and reusable downloads need different ownership |
| 15 | Dart | Dart/Flutter pub | project outputs `project_only`; shared pub cache `advisory` | owner GC/clean is interactive or version-sensitive |
| 16 | Swift | SwiftPM; Xcode | SwiftPM `.build` `project_only`; DerivedData `full` on macOS | project rebuild and Xcode re-index; unavailable on Windows |
| 17 | R | pak, renv | `advisory` | linked project libraries make blind shared-cache deletion unsafe |
| 18 | Lua | LuaRocks | project trees/shared store `advisory` | owner/version semantics vary |
| 19 | PowerShell | module managers and concrete commands | `not_applicable` | history, profiles, credentials and modules are user state |
| 20 | Objective-C | Xcode, CocoaPods, SwiftPM | Xcode generated data `full` on macOS; CocoaPods `advisory` | rebuild/re-index; unavailable on Windows |
| 21 | Scala | sbt, Ivy, Coursier, Maven | project/shared stores `advisory` | overlapping JVM stores require owner-aware accounting |
| 22 | Groovy | Gradle, Maven, Grape | project outputs `project_only`; shared stores `advisory` | shared JVM family; no broad `~/.gradle` recursion |
| 23 | Haskell | Cabal, Stack | `advisory` | global stores and project work require distinct owner contracts |
| 24 | Perl | CPAN, cpanm | `advisory` | installed modules, build work and download caches can coexist |
| 25 | Julia | `Pkg` depot | `advisory` | add `Pkg.gc()` only through a bounded Julia provider |
| 26 | Elixir / Erlang | Mix, Hex, Rebar3 | project `_build`/`deps` `project_only`; stores `advisory` | dependency restore and compile |
| 27 | Clojure | Clojure CLI, Leiningen, Maven | `advisory` | shares Maven artifacts and owner locks with the JVM family |
| 28 | OCaml | opam, dune | project outputs `project_only`; opam state `advisory` | switches contain installed packages, not disposable cache |
| 29 | Zig | Zig compiler cache | `advisory` | global/local layouts are version-sensitive; compile again |
| 30 | Solidity | Foundry, Hardhat and npm | project outputs `project_only`; shared downloads `advisory` | compiler/artifact ownership belongs to the concrete tool |

The priority list is intentionally language-complete while implementation is
owner-based. Shared providers are registered once: Java/Kotlin/Groovy/Scala/
Clojure use JVM owners; C/C++ use native-build owners; Swift/Objective-C use
Apple build owners; and TypeScript/JavaScript share Node/Bun owners. `advisory`
is a safety decision, not missing permission to recursively delete a familiar
dot-directory.

## Developer AI and LLM tools

| Tool family | Observed storage | Zenith disposition |
| --- | --- | --- |
| Codex, Claude Code, Gemini CLI, Cursor | roots can mix sessions, settings, credentials, logs, extensions and transient files | Only separately documented cache/log subpaths may be catalogued; broad roots remain Manual |
| Hugging Face Hub | content-addressed model and dataset revisions | Manual model inventory today; a future provider may use `hf cache prune --yes`, while targeted revision/model removal stays explicit |
| Ollama | named model manifests and blobs | Existing typed inventory/removal; never generic cache cleanup |
| LM Studio | models plus application/session state | Existing typed inventory; no documented generic owner cleanup command |
| Local compilation runtimes | kernels, autotune records, optimized engines and model weights | Generated kernels may be Rebuild only under an independently proven root; weights, sessions and configured engines are Manual |

Large size does not turn LLM state into cache. Credentials, conversations,
prompts, settings, model weights and user-selected revisions keep their typed or
Manual lifecycle even when they live beside disposable files.

### macOS explicit cache unit

The first filesystem example is Cursor's five named renderer/code/GPU cache
subtrees under `~/Library/Application Support/Cursor`:
`Cache`, `CachedData`, `Code Cache`, `GPUCache`, and `ShaderCache`. Zenith offers
each existing subtree as an explicit Rebuild review unit with no age or
intensive-scope gate. It checks Cursor and its helper processes immediately
before mutation; unreadable process state or a running owner skips that unit.
One excluded or protected descendant refuses the whole subtree. The verified
unit may contain cache databases, their companions and cache-local locks, but
settings, credentials, sessions, app bundles, executables, logs, extensions,
and neighboring Application Support data still refuse or remain outside its
scope. It is never added to automatic Safe selection or the Quick Panel.

The next reviewed unit is Brave Browser's `Default/Code Cache` under its
per-user `~/Library/Caches/BraveSoftware/Brave-Browser` tree. It is an explicit
Rebuild unit with the same protected-descendant and running-owner checks.
Other Brave profiles, HTTP cache, cookies, history, and offline content remain
outside that unit. The Default profile HTTP cache has a separate Manual
inventory row. The broad third-party cache rule excludes `BraveSoftware` to
prevent an overlapping generic deletion path.

The existing Windows Cursor rule keeps its Windows-only paths and current
strategy. CloudKit is inventoried by one macOS Manual signature and excluded
from broad third-party app-cache discovery; Zenith does not mutate that
service-owned store.

## GPU and local-AI runtimes

| Runtime / owner | Artifact role | macOS | Windows | Mode / risk |
| --- | --- | --- | --- | --- |
| Direct3D | `compiled_kernel` | unavailable | `%LOCALAPPDATA%/D3DSCache` | Zenith / Rebuild |
| NVIDIA driver | `compiled_kernel` | unavailable | per-user DXCache + GLCache | Zenith / Rebuild |
| CUDA JIT | `compiled_kernel` | unavailable | documented ComputeCache default | Zenith / Rebuild |
| PyTorch / TorchInductor / Triton | `compiled_kernel`, `autotune` | inactive `torchinductor_*` temp scope | same | Zenith / Rebuild |
| vLLM | compile artifacts; weights separate | configured roots `advisory` | WSL/container only `advisory` | advisory / Rebuild |
| SGLang | mixed root with Torch/Triton/DeepGEMM owners | `advisory` | WSL/container `advisory` | advisory / Manual root |
| llama.cpp / GGML | OpenCL `compiled_kernel`; GGUF/session separate | OpenCL cache `full` | OpenCL cache `full` | Zenith / Rebuild |
| TensorFlow / XLA | configured persistent compilation cache | `advisory` | `advisory` | advisory / Rebuild |
| JAX / XLA | trusted executable compilation cache | `advisory` | `advisory` | advisory / Rebuild; shared/world-writable rejected |
| ONNX Runtime | optimized model / provider cache | `advisory` | `advisory` | advisory / Manual |
| TensorRT / TensorRT-LLM | optimized engine, timing, autotune | unavailable | configured output `advisory` | advisory / Rebuild |
| OpenVINO | explicitly configured model cache | `advisory` | `advisory` | advisory / Rebuild |
| MLX / MLX-LM | model weights via Hugging Face/MLX | Manual | unavailable | dedicated model adapter / Manual |
| ROCm / MIOpen | versioned kernels; performance DB preserved | unavailable | unavailable | advisory until stable host support |
| Hugging Face Hub | shared model/download store | Manual | Manual | dedicated provider required / Manual |
| Ollama | model manifests and blobs | Manual | Manual | existing typed model workflow / Manual |
| LM Studio | model weights | Manual | Manual | existing typed model workflow / Manual |
| ComfyUI / diffusion | models, LoRA, VAE, inputs, outputs | Manual | Manual | user content; no guessed cleanup |

`model_weight`, `prompt_or_session_state`, and application-configured
`optimized_engine` are Manual. `compiled_kernel` and `autotune` can be Rebuild
only with independent ownership. `download_cache` is Rebuild or Manual when
shared. `runtime_memory` is observation-only and never a disk cleanup target.
SLM is a model-size label, not a storage owner.

## Provider contracts shipped in 0.3

- `Go build cache`: discover with `go env GOCACHE`, clear with `go clean
  -cache`.
- `Go module cache`: discover with `go env GOMODCACHE`, clear with `go clean
  -modcache`.
- `uv`: discover with `uv cache dir`, prune with `uv cache prune`.
- `pip`: discover with `pip3 cache dir`, clear with `pip3 cache purge`.
- `pnpm`: discover with `pnpm store path`, prune with `pnpm store prune`.
- `npm`: discover with `npm config get cache`, verify/prune unneeded entries
  with `npm cache verify` as an explicit Rebuild action.
- `Composer`: discover with `composer --no-interaction --no-plugins config
  --global cache-dir --absolute`, clear with `composer --no-interaction
  --no-plugins clear-cache`.
- `NuGet`: discover and clear the `http-cache`, `temp`, `plugins-cache`, and
  `global-packages` resources separately through `dotnet nuget locals`; the
  global package store is never presented as equivalent to a temporary cache.

All adapters use a resolved trusted executable, fixed arguments, bounded
output, current-user containment, symlink/reparse rejection, fresh path
discovery before and after mutation, active-owner checks, the global cleanup
operation gate, and the existing bounded one-shot scan/delete plan. Commands
have a 15-second timeout except uv prune, which may wait up to five minutes for
uv's cache lock. uv relies on that owner lock instead of blocking on unrelated
Python processes; pip, npm, and pnpm match their CLI entrypoints rather than
every Python or Node process. `UV_CACHE_DIR`, `PIP_CACHE_DIR`, npm's cache
override, and pnpm's store-directory override are captured once in the platform
environment, validated as existing cache directories, and applied identically
during inspection and execution. Other inherited path and project-config
variables remain stripped. A tool whose default cache directory does not yet
exist produces no cleanup row; an invalid existing path remains a scan gap.
Go additionally removes cache/path overrides and sets
`GOTOOLCHAIN=local`, so inspection cannot download another toolchain or redirect
authority. Composer disables plugins during discovery and cleanup so a cache
action cannot execute user- or project-supplied plugin code. Unavailable or
failing providers do not stop the broader scan; failures remain visible as
scan gaps.

Poetry, Conda, Yarn, Bun, Hugging Face, Gradle, Maven, Dart, Julia,
RubyGems, R package managers, Haskell, Zig, LuaRocks, CPAN, opam, Foundry and
mixed AI roots stay advisory until equally narrow contracts are implemented.
Most importantly, every `package_store` catalog entry is rejected at load time
if it attempts to use a generic filesystem-delete strategy. It must use a
backend owner provider, a fixed external command, or remain Manual.

## Sources

The priority baseline uses [GitHub Octoverse
2025](https://github.blog/news-insights/octoverse/octoverse-a-new-developer-joins-github-every-second-as-ai-leads-typescript-to-1/)
and the [Stack Overflow 2025 Developer
Survey](https://survey.stackoverflow.co/2025/technology). Ownership and cleanup
rules use these primary sources:

- [Microsoft Direct3D shader cache](https://learn.microsoft.com/en-us/windows/win32/api/d3d12/ne-d3d12-d3d12_shader_cache_flags),
  [NVIDIA shader cache](https://www.nvidia.com/content/Control-Panel-Help/vLatest/en-gb/mergedProjects/nv3dENG/Manage_3D_Settings_%28reference%29.htm), and
  [CUDA cache variables](https://docs.nvidia.com/cuda/cuda-programming-guide/05-appendices/environment-variables.html)
- [PyTorch compile cache](https://docs.pytorch.org/tutorials/recipes/torch_compile_caching_configuration_tutorial.html),
  [vLLM cache](https://docs.vllm.ai/en/latest/configuration/optimization/),
  [vLLM platforms](https://docs.vllm.ai/en/latest/getting_started/installation/gpu/), and
  [SGLang variables](https://github.com/sgl-project/sglang/blob/main/docs/references/environment_variables.md)
- [llama.cpp OpenCL cache](https://github.com/ggml-org/llama.cpp/blob/master/docs/backend/OPENCL.md),
  [JAX persistent cache](https://docs.jax.dev/en/latest/persistent_compilation_cache.html),
  [ONNX Runtime optimization](https://onnxruntime.ai/docs/performance/model-optimizations/graph-optimizations.html), and
  [MIOpen cache](https://rocm.docs.amd.com/projects/MIOpen/en/develop/install/build-source.html)
- [uv cache](https://docs.astral.sh/uv/concepts/cache/),
  [pnpm store](https://pnpm.io/cli/store),
  [npm cache](https://docs.npmjs.com/cli/v11/commands/npm-cache/),
  [pip cache](https://pip.pypa.io/en/stable/cli/pip_cache/), and
  [NuGet local resources](https://learn.microsoft.com/en-us/nuget/consume-packages/managing-the-global-packages-and-cache-folders)
- [Go module cache](https://go.dev/ref/mod),
  [Go cache management](https://go.dev/doc/manage-install), and
  [Composer command-line interface](https://getcomposer.org/doc/03-cli.md)
- [Gradle cache cleanup](https://docs.gradle.org/current/userguide/directory_layout.html),
  [Maven project purge](https://maven.apache.org/components/plugins/maven-dependency-plugin/examples/purging-local-repository.html),
  [Yarn cache clean](https://yarnpkg.com/cli/cache/clean),
  [Bun package-manager CLI](https://bun.sh/docs/pm/cli/pm), and
  [Dart pub cache](https://dart.dev/tools/pub/cmd/pub-cache)
- [Hugging Face cache management](https://huggingface.co/docs/huggingface_hub/guides/manage-cache)
  [Ollama CLI](https://docs.ollama.com/cli),
  [LM Studio CLI](https://lmstudio.ai/docs/cli), and
  [renv cache semantics](https://rstudio.github.io/renv/articles/package-install.html)

Review this matrix annually and whenever an owner changes its storage or cleanup
contract.
