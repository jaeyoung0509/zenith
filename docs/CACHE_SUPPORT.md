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

## Programming-language ecosystems

| Language | Provider family / evidence | macOS | Windows | Cleanup consequence |
| --- | --- | --- | --- | --- |
| JavaScript / TypeScript | Node manifests; npm, pnpm, Yarn, Bun | pnpm `tool_managed`; project `node_modules`; others `advisory` | same | package download / project restore |
| Python | project manifest + `pyvenv.cfg`; uv, pip | uv `tool_managed`; venv `project_only`; pip `advisory` | same | download, wheel build, environment restore |
| Java | Maven/Gradle project markers | `project_only`; shared stores `advisory` | same | dependency download / build |
| C | CMake generated metadata; native-build family | `project_only` | `project_only` | compile and link |
| C++ | CMake generated metadata; native-build family | `project_only` | `project_only` | compile and link |
| C# | `.sln`/`.csproj` plus `bin`/`obj`; NuGet | `project_only`; NuGet `advisory` | same | restore and compile |
| Go | Go build/module cache family | `full` / `project_only` | `full` / `project_only` | compile or module download |
| Rust | Cargo target/registry/git; rustup downloads | `full` / `project_only` | same | crate download / compile |
| PHP | Composer manifest and generated vendor metadata | `project_only` | `project_only` | `composer install` |
| Ruby | Gemfile plus Bundler project tree | `project_only` | `project_only` | `bundle install` |
| Kotlin | shared JVM Gradle/Maven family | `project_only`; stores `advisory` | same | dependency download / build |
| Swift | SwiftPM `.build`; Xcode DerivedData | `project_only` / `full` | unavailable | rebuild and re-index |
| Objective-C | shared Xcode/CocoaPods/SPM family | Xcode `full`; CocoaPods `advisory` | unavailable | rebuild and re-index |
| Dart | pubspec and Flutter generated metadata | `project_only`; pub GC `advisory` | same | package restore |
| Scala | shared JVM sbt/Ivy/Coursier family | `advisory` | `advisory` | dependency download / compile |
| Clojure | shared JVM Lein/Clojure/Maven family | `advisory` | `advisory` | dependency download / compile |
| R | renv shared cache and symlinked libraries | `advisory` | `advisory` | package restore; never remove linked library blindly |
| Julia | depot and `Pkg.gc()` | `advisory` | `advisory` | package/artifact download and precompile |
| Elixir / Erlang | Mix manifest plus `_build`/`deps`; Hex/Rebar | `project_only`; stores `advisory` | same | dependency restore / compile |
| Haskell | Cabal/Stack owned work and stores | `advisory` | `advisory` | dependency download / compile |
| Zig | versioned project/global caches | `advisory` | `advisory` | compile |
| Lua | LuaRocks project trees and owner cache | `advisory` | `advisory` | package restore |
| HCL / Terraform | `.terraform.lock.hcl` or `.tf` plus `.terraform` | `project_only` | `project_only` | provider/module download; state is never touched |
| SQL | no language-owned generic cache | `not_applicable` | `not_applicable` | application/database specific |
| Shell | no language-owned generic cache | `not_applicable` | `not_applicable` | tool specific |
| HTML / CSS | no language-owned generic cache | `not_applicable` | `not_applicable` | framework/build-tool specific |

Shared providers are counted once: Java/Kotlin/Scala/Clojure use the JVM family;
C/C++ use native-build providers; Swift/Objective-C use Apple build providers;
and JavaScript/TypeScript use Node providers.

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

- `uv`: discover with `uv cache dir`, prune with `uv cache prune`.
- `pnpm`: discover with `pnpm store path`, prune with `pnpm store prune`.

Both adapters use a resolved trusted executable, fixed arguments, a 15-second
timeout, bounded output, current-user containment, symlink/reparse rejection,
fresh path discovery before and after mutation, active-process refusal, the
global cleanup operation gate, and the existing bounded one-shot scan/delete
plan. Unavailable or failing providers are skipped without failing the broader
scan. npm, pip, NuGet, Yarn, Hugging Face, Gradle, Maven, Bun, Dart, Julia,
Haskell, Zig, LuaRocks, and mixed AI roots stay advisory until equally narrow
contracts are implemented.

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
  [npm cache](https://docs.npmjs.com/cli/v7/commands/npm-cache/),
  [pip cache](https://pip.pypa.io/en/stable/cli/pip_cache/), and
  [NuGet local resources](https://learn.microsoft.com/en-us/nuget/consume-packages/managing-the-global-packages-and-cache-folders)
- [Gradle cache cleanup](https://docs.gradle.org/current/userguide/directory_layout.html),
  [Maven project purge](https://maven.apache.org/components/plugins/maven-dependency-plugin/examples/purging-local-repository.html),
  [Yarn cache clean](https://yarnpkg.com/cli/cache/clean),
  [Bun global cache](https://bun.com/docs/pm/global-cache), and
  [Dart pub cache](https://dart.dev/tools/pub/cmd/pub-cache)
- [Hugging Face cache management](https://huggingface.co/docs/huggingface_hub/guides/manage-cache)
  and [renv cache semantics](https://rstudio.github.io/renv/articles/package-install.html)

Review this matrix annually and whenever an owner changes its storage or cleanup
contract.
