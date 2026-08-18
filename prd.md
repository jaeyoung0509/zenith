좋아. 기존 PRD에서 **SwiftUI/UniFFI를 완전히 제거하고**, 실제 구현자가 그대로 따라갈 수 있도록 기능별 구현 원칙과 Acceptance Criteria까지 넣어서 다시 잡았어.

특히 네가 말한 향후 **“특정 앱 실행 중이면 절전 방지”**도 처음부터 확장 가능한 구조로 넣었다. 다만 macOS에서 `시스템 Sleep 방지`, `디스플레이 Sleep 방지`, `화면 잠금 방지`는 서로 같은 개념이 아니므로, Zenith는 나중에 앞의 두 개를 정확하게 지원하는 방향으로 설계하는 게 좋다. Apple은 IOKit의 power assertion API로 시스템 동작을 요청할 수 있다. 

# Zenith

## macOS AI & Developer System Manager

**Product Requirements Document — v0.2**

---

# 1. 제품 정의

## 1.1 한 줄 정의

> 개발하면서 쌓이는 AI 도구, 패키지 매니저, 빌드 캐시, Docker, 로컬 LLM 데이터를 메뉴바에서 빠르게 확인하고 안전하게 정리하는 초경량 macOS 유틸리티.

Zenith는 CleanMyMac의 축소판이 아니다.

Zenith가 해결하려는 문제는 보다 구체적이다.

```text
"내 SSD 500GB가 왜 이렇게 빨리 찼지?"

              ↓

Claude / Cursor / Gemini
Cargo / Go / npm / uv
Docker
Ollama / HuggingFace
Xcode
각종 개발 캐시

              ↓

Zenith

"개발 관련 데이터가 84.7GB 있습니다."

              ↓

Safe        18.3 GB
Rebuild      9.2 GB
Manual      57.2 GB
```

사용자는 Finder를 뒤지거나 검색해서 어떤 폴더를 지워도 되는지 알아낼 필요가 없다.

---

# 2. 제품 철학

Zenith의 핵심은 세 단어다.

> **Clean · Clear · Quiet**

## Clean

정말 불필요한 데이터만 제거한다.

## Clear

왜 삭제 가능한지와 삭제하면 어떤 일이 생기는지 설명한다.

## Quiet

사용하지 않을 때는 거의 아무 일도 하지 않는다.

---

# 3. 기술 스택

```text
Tauri 2
+
Svelte 5
+
TypeScript
+
Vite
+
Rust
```

Tauri 2는 HTML/JS/CSS로 컴파일되는 프론트엔드 프레임워크를 사용할 수 있으며 Rust 백엔드와 WebView UI를 결합하는 구조다. system tray와 desktop window도 지원한다. 

Svelte에서는 Svelte 5의 runes API를 사용한다.

```text
$state
$derived
$effect
```

Svelte 5에서는 runes가 반응성의 핵심 API다. 

---

# 4. 의도적으로 사용하지 않는 기술

## SvelteKit

사용하지 않는다.

Zenith에는:

- SSR
- 서버 라우트
- API 서버
- 웹 배포
- SEO

가 필요하지 않다.

따라서:

```text
Svelte 5
+
Vite
```

만 사용한다.

---

## SwiftUI

사용하지 않는다.

## UniFFI

사용하지 않는다.

## Electron

사용하지 않는다.

## 별도 backend server

사용하지 않는다.

---

# 5. 전체 시스템 구조

```text
┌────────────────────────────────────────────┐
│                 macOS                       │
│                                             │
│  Menu Bar                                  │
│      ● Zenith                              │
│          │                                 │
│          ▼                                 │
│  ┌───────────────────────┐                 │
│  │ Quick Panel           │                 │
│  │ Svelte 5              │                 │
│  └──────────┬────────────┘                 │
│             │                              │
│             │ Open Zenith                  │
│             ▼                              │
│  ┌───────────────────────────────┐         │
│  │ Desktop Dashboard             │         │
│  │ Svelte 5                      │         │
│  └───────────────┬───────────────┘         │
│                  │                         │
│           Tauri IPC                       │
│                  │                         │
│  ┌───────────────▼───────────────┐         │
│  │ Rust Core                     │         │
│  │                               │         │
│  │ Scanner                       │         │
│  │ Signature Registry            │         │
│  │ Safety Engine                 │         │
│  │ Cleaner                       │         │
│  │ Docker Adapter                │         │
│  │ System Metrics                │         │
│  │ Keep Awake Engine (future)    │         │
│  └───────────────────────────────┘         │
└────────────────────────────────────────────┘
```

Tauri command를 통해 Svelte에서 Rust 함수를 직접 호출할 수 있다. 

---

# 6. 프로세스 모델

별도의 Rust daemon을 실행하지 않는다.

Tauri 자체의 Rust process 안에서 필요한 작업만 실행한다.

```text
Idle

Svelte WebView
+
Tauri Core

Scanner           OFF
Disk polling      OFF
Docker polling    OFF
Memory polling    OFF
Network           OFF
```

사용자가 메뉴를 열거나 Scan을 누를 때만 필요한 작업을 시작한다.

---

# 7. Repository 구조

```text
zenith/
│
├── src/
│   ├── lib/
│   │   ├── components/
│   │   ├── stores/
│   │   ├── models/
│   │   └── utils/
│   │
│   ├── routes/
│   │   ├── quick/
│   │   ├── dashboard/
│   │   ├── category/
│   │   └── settings/
│   │
│   └── App.svelte
│
├── src-tauri/
│   │
│   ├── src/
│   │   ├── commands/
│   │   ├── scanner/
│   │   ├── cleaner/
│   │   ├── safety/
│   │   ├── signatures/
│   │   ├── docker/
│   │   ├── metrics/
│   │   ├── power/
│   │   └── models/
│   │
│   ├── Cargo.toml
│   └── tauri.conf.json
│
├── signatures/
│   ├── ai.toml
│   ├── developer.toml
│   ├── containers.toml
│   └── models.toml
│
└── package.json
```

---

# 8. UI 구조

Zenith에는 UI가 두 개 있다.

## A. Quick Panel

메뉴바 아이콘 클릭 시 표시한다.

## B. Desktop Dashboard

상세 관리가 필요할 때 연다.

---

# 9. Menu Bar

평상시 메뉴바에는 아이콘 하나만 표시한다.

```text
Wi-Fi   Bluetooth   ●   Battery
                   ↑
                 Zenith
```

다음은 표시하지 않는다.

```text
❌ CPU %
❌ RAM %
❌ 디스크 %
❌ 12GB Cleanable
```

이유는:

- 메뉴바 공간 절약
- polling 최소화
- 시각적 noise 제거

때문이다.

Tauri는 tray icon과 tray click event를 제공한다. 

---

# 10. Quick Panel

크기:

```text
width: 360px
height: 약 480px
```

메뉴바 아이콘을 클릭하면 아이콘 바로 아래 표시한다.

Tauri의 Positioner plugin은 tray icon 기준 위치 배치를 지원하므로 이를 활용한다. 

---

## UI

```text
┌─────────────────────────────────┐
│ Zenith                       ⚙  │
│                                 │
│ Mac                             │
│ ███████████████░░░  341 / 494G │
│                                 │
│ Can clean                       │
│ 18.4 GB                         │
│                                 │
│ AI Tools                3.2 GB  │
│ Developer               8.3 GB  │
│ Docker                  6.9 GB  │
│ Local Models           41.2 GB  │
│                                 │
│ ─────────────────────────────── │
│ Memory                  Normal  │
│ 12.4 / 16 GB                    │
│                                 │
│       [ Scan Again ]            │
│                                 │
│       Clean 18.4 GB             │
│                                 │
│       Open Zenith →             │
└─────────────────────────────────┘
```

---

# 11. Quick Panel 동작

메뉴를 열 때:

```text
show panel
    ↓
load lastScan
    ↓
collect lightweight system metrics
    ↓
render
```

**전체 disk scan은 자동 실행하지 않는다.**

마지막 스캔 결과를 즉시 보여준다.

예:

```text
18.4 GB

Last scan
2 hours ago
```

---

# 12. Desktop Dashboard

`Open Zenith` 클릭 시 일반 desktop window를 연다.

```text
┌──────────────────────────────────────────────────┐
│ Zenith                                      ⚙   │
├──────────────────────────────────────────────────┤
│                                                  │
│ Storage                                          │
│                                                  │
│ █████████████████████████░░░                     │
│ 341 GB / 494 GB                                  │
│                                                  │
│ Reclaimable                                      │
│ 18.4 GB                                          │
│                                                  │
│ AI Tools                           3.2 GB         │
│ Developer                          8.3 GB         │
│ Docker                             6.9 GB         │
│ Local Models                      41.2 GB         │
│                                                  │
│              [ Scan ]                            │
│                                                  │
├──────────────────────────────────────────────────┤
│ Memory                                           │
│                                                  │
│ Pressure                      Normal              │
│ Compressed                     2.1 GB             │
│ Swap                           0.8 GB             │
│                                                  │
│ Cursor                         2.8 GB             │
│ Chrome                         2.2 GB             │
│ Docker                         1.6 GB             │
└──────────────────────────────────────────────────┘
```

---

# 13. 디자인 원칙

Zenith UI는 cleaner 프로그램 특유의:

```text
❌ 네온 그라데이션
❌ "BOOST!"
❌ "YOUR MAC IS AT RISK"
❌ 엄청 큰 게이지
❌ 과도한 애니메이션
❌ 빨간 경고
```

를 사용하지 않는다.

대신:

```text
macOS Settings
+
Linear
+
Raycast
```

중간 정도의 밀도를 목표로 한다.

---

# 14. Svelte 상태 구조

Pinia/Redux 같은 별도 상태관리 라이브러리를 넣지 않는다.

Svelte 5 runes를 사용한다.

```ts
let scan = $state<ScanState>({
  status: 'idle',
  progress: 0,
  categories: []
});
```

파생 값:

```ts
let reclaimable = $derived(
  scan.categories.reduce(
    (total, category) =>
      total + category.selectedBytes,
    0
  )
);
```

---

# 15. Scanner Engine

Scanner가 Zenith의 핵심이다.

전체 `$HOME`을 무작정 재귀 탐색하지 않는다.

```text
Signature Registry
        ↓
Known Roots
        ↓
Scanner
```

---

# 16. Signature Registry

예:

```toml
id = "go.build"
name = "Go Build Cache"

category = "developer"

risk = "safe"

paths = [
  "~/Library/Caches/go-build"
]
```

또는:

```toml
id = "cargo.registry"
name = "Cargo Registry Cache"

category = "developer"

risk = "rebuild"

paths = [
  "~/.cargo/registry/cache"
]
```

---

# 17. Signature 데이터 모델

Rust:

```rust
pub struct Signature {
    pub id: String,
    pub display_name: String,

    pub category: Category,

    pub paths: Vec<PathPattern>,
    pub exclusions: Vec<PathPattern>,

    pub risk: RiskTier,
    pub strategy: CleanStrategy,

    pub min_age_days: Option<u32>,
}
```

---

# 18. Category

```rust
pub enum Category {
    Ai,
    Developer,
    Container,
    Model,
    System,
}
```

---

# 19. RiskTier

```rust
pub enum RiskTier {
    Safe,
    Rebuild,
    Manual,
}
```

---

# 20. CleanStrategy

중요하다.

모든 데이터를 `remove_dir_all()`로 삭제하지 않는다.

```rust
pub enum CleanStrategy {

    DeleteContents,

    DeleteDirectory,

    ExternalCommand,

    DockerPrune,

    Manual,
}
```

---

# 21. Scan Pipeline

```text
scan()
 │
 ▼
load signatures
 │
 ▼
expand ~
 │
 ▼
validate root
 │
 ▼
walk directory
 │
 ├── symlink → do not follow
 │
 ├── blacklist → skip
 │
 └── normal → measure
 │
 ▼
aggregate
 │
 ▼
ScanResult
```

---

# 22. 스캔 진행 상황

Scan 버튼을 눌렀을 때 화면이 멈추면 안 된다.

```text
Scanning...

✓ Gemini
✓ Claude
● Cargo
○ Go
○ Docker

8.3 GB found
```

---

# 23. Rust → Svelte Streaming

단순히:

```text
invoke("scan")
↓
10초
↓
response
```

방식으로 구현하지 않는다.

Tauri Channel을 사용한다.

현재 Tauri 문서에서도 streaming 데이터 전달에는 Channel이 권장된다. 

Rust:

```rust
#[derive(Clone, Serialize)]
#[serde(tag = "type")]
enum ScanEvent {

    Started,

    CategoryStarted {
        category: String,
    },

    ItemFound {
        item: ScanItem,
    },

    CategoryFinished {
        category: String,
        bytes: u64,
    },

    Finished {
        result: ScanResult,
    },
}
```

---

# 24. Svelte 처리

```ts
const events = new Channel<ScanEvent>();

events.onmessage = (event) => {
    handleScanEvent(event);
};

await invoke('scan', {
    onEvent: events
});
```

Svelte는 이벤트를 받는 즉시 UI를 업데이트한다.

---

# 25. ScanResult

```rust
pub struct ScanResult {

    pub started_at: SystemTime,

    pub finished_at: SystemTime,

    pub categories: Vec<CategoryResult>,

    pub safe_bytes: u64,

    pub rebuild_bytes: u64,

    pub manual_bytes: u64,
}
```

---

# 26. Size 계산

두 값을 구분한다.

```rust
pub struct FileSize {
    pub logical: u64,
    pub allocated: Option<u64>,
}
```

Zenith의 UI에서는 가능하면:

```text
Estimated reclaimable
```

을 allocated 기준으로 계산한다.

단순 logical file size를:

> "지우면 이만큼 확보됩니다."

라고 보여주지 않는다.

---

# 27. AI Tools

초기 지원:

```text
Claude Code
Cursor
Gemini CLI
Codex
Aider
OpenCode
```

---

# 28. AI 삭제 정책

자동 삭제 대상으로 취급할 수 있는 것:

```text
logs
temporary data
reconstructible indexes
download caches
old transient files
```

절대 자동 삭제하지 않는 것:

```text
credentials
OAuth
API keys

settings

MCP configuration

project settings

user prompts

source files
```

---

# 29. Developer

v0.1:

```text
Go

Cargo

npm
pnpm
yarn
bun

pip
uv

Xcode
```

---

# 30. Detail UI

```text
Developer

8.3 GB

✓ Go Build Cache
  3.1 GB
  Safe

✓ Cargo Cache
  1.7 GB
  Safe

✓ uv
  840 MB
  Safe

○ pnpm Store
  1.2 GB
  Re-download required

○ Xcode DerivedData
  1.4 GB
  Rebuild required
```

---

# 31. Docker

Docker filesystem 내부를 Zenith가 직접 조작하지 않는다.

```text
Zenith
   ↓
Docker Adapter
   ↓
Docker API / CLI
```

---

# 32. Docker Scan

UI:

```text
Docker

Total
18.3 GB

Images
7.2 GB

Build cache
8.1 GB

Stopped containers
1.4 GB

Volumes
1.6 GB
```

---

# 33. Docker Clean 정책

기본 cleanup:

```text
dangling images
unused build cache
stopped containers
```

자동 삭제 금지:

```text
running containers
named volumes
actively referenced images
```

Volumes는 항상 Manual이다.

---

# 34. Local Models

지원:

```text
Ollama
HuggingFace
LM Studio
MLX
```

---

# 35. Model UI

```text
Local Models

41.2 GB

Ollama

[ ] llama3:70b
    18.2 GB

[ ] qwen3:32b
     9.8 GB

[ ] gemma3
     4.2 GB
```

모델은 절대 Quick Clean에 자동 선택되지 않는다.

---

# 36. Safety Engine

Scanner와 Cleaner 사이에 반드시 Safety Engine이 존재한다.

```text
Scanner
   ↓
Candidate[]
   ↓
Planner
   ↓
Safety Engine
   ↓
DeletePlan
   ↓
Cleaner
```

---

# 37. 절대 금지 구조

이런 코드는 만들지 않는다.

```rust
if is_cache(path) {
    std::fs::remove_dir_all(path)?;
}
```

Scanner는 삭제 권한이 없다.

---

# 38. DeletePlan

```rust
pub struct DeletePlan {

    pub id: Uuid,

    pub targets: Vec<DeleteTarget>,

    pub expected_reclaim: u64,

    pub risk: RiskSummary,
}
```

---

# 39. DeleteTarget

```rust
pub struct DeleteTarget {

    pub path: PathBuf,

    pub signature_id: String,

    pub expected_bytes: u64,

    pub risk: RiskTier,

    pub identity: FileIdentity,
}
```

---

# 40. TOCTOU 보호

스캔과 삭제 사이에 파일이 바뀔 수 있다.

따라서 scan 시:

```text
path
device
inode
type
```

을 기록한다.

삭제 직전 다시 `lstat()`하여 비교한다.

```text
scan

foo/cache
inode = 1234

       ↓

user/application changes filesystem

       ↓

clean

foo/cache
inode = 9821

       ↓

ABORT
```

---

# 41. Symlink 규칙

symlink를 따라가지 않는다.

예:

```text
~/.cache/foo/sensitive
             ↓
          symlink
             ↓
          ~/.ssh
```

이 경우:

```text
symlink 자체
```

만 처리 가능하며 `.ssh` 내부로 traversal하지 않는다.

---

# 42. Hard Blacklist

다음 경로는 Signature보다 우선한다.

```text
/System

/bin
/sbin
/usr

~/.ssh
~/.gnupg
~/.aws

~/Library/Keychains

.git
```

---

# 43. User Content

다음은 Generic Cleaner 대상에서 제외한다.

```text
Desktop
Documents
Pictures
Movies
Music
```

Zenith는 사용자 콘텐츠를:

> 불필요한 파일

이라고 판단하지 않는다.

---

# 44. Cleaner

Cleaner는 DeletePlan만 입력으로 받는다.

```rust
pub fn execute(
    plan: DeletePlan
) -> CleanResult
```

임의의 `PathBuf`를 받지 않는다.

즉 아래 API는 만들지 않는다.

```rust
delete(path: PathBuf)
```

---

# 45. 삭제 실패

삭제 실패 시 시스템을 억지로 변경하지 않는다.

```text
remove
  │
  ├─ success
  │
  └─ error
       │
       ├─ PermissionDenied
       ├─ ChangedSinceScan
       ├─ NotFound
       ├─ InUse
       └─ Unknown
```

---

# 46. 하지 않을 것

```text
chmod -R 777

chflags -R 0

xattr -cr

sudo rm -rf

kill -9
```

Zenith의 목적은:

> 어떻게든 삭제한다

가 아니다.

> 안전하다고 판단된 것만 삭제한다

이다.

---

# 47. Clean Result

삭제 후 실제 disk free space를 다시 측정한다.

```text
Clean Complete

11.8 GB reclaimed

Go             3.1 GB
Cargo          2.0 GB
Gemini         1.2 GB
Docker         5.5 GB

Failed
12 MB

[View Details]
```

---

# 48. Memory Inspector

Zenith는 RAM Cleaner를 만들지 않는다.

Memory Inspector를 만든다.

---

# 49. Memory Quick View

```text
Memory

Normal

12.1 / 16 GB

Compressed
2.3 GB

Swap
0.7 GB
```

---

# 50. Process View

```text
Memory

Cursor

2.8 GB
19 processes

Chrome

2.1 GB
28 processes

Docker

1.5 GB
```

---

# 51. Memory polling

Popover가 열려 있을 때만:

```text
2~3 sec
```

정도로 갱신한다.

Popover가 닫히면 timer를 해제한다.

Desktop Memory 화면에서도 화면이 활성화된 경우에만 polling한다.

---

# 52. Local Persistence

SQLite를 사용하지 않는다.

Tauri Store 정도면 충분하다.

Tauri의 Store plugin은 앱 재시작 사이에도 key-value state를 파일에 저장할 수 있다. 

저장 대상:

```text
lastScan

excludedSignatures

selectedCategories

launchAtLogin

window preferences
```

---

# 53. Launch At Login

Settings:

```text
General

[✓] Launch Zenith at login
```

Tauri에는 공식 Autostart plugin이 있으므로 별도 LaunchAgent 코드를 직접 구현하지 않는다. 

---

# 54. Settings

```text
General

Launch at login        ON


Cleaning

AI Tools               ON
Developer              ON
Docker                 ON

Local Models           OFF


Safety

Rebuild caches         OFF


Appearance

Follow System          ON
```

---

# 55. Logging

로그에는 가능하면 사용자의 전체 filename을 남기지 않는다.

```text
SCAN
category=go
files=18293
bytes=3212332121
duration=713ms
```

---

# 56. Network

v0.1에서 Zenith는 네트워크가 필요 없다.

```text
Telemetry       없음

Analytics       없음

Account         없음

Backend         없음

Cloud           없음
```

---

# 57. 성능 원칙

전체 `$HOME`을 계속 scan하지 않는다.

Quick Scan은:

```text
Known paths only
```

이다.

---

# 58. 목표 성능

arm64 release build 기준으로 측정한다.

목표:

```text
Idle CPU

≈ 0%


Background disk IO

0


Quick Scan

P50 < 2 sec

P95 < 10 sec


UI

60fps 목표
```

RAM은 구현 후 release build에서 실제 측정하여 budget을 정한다.

WebView를 사용하는 Tauri이므로 처음부터 비현실적인 20~25MB hard limit을 설정하지 않는다.

---

# 59. Rust concurrency

무제한 parallel traversal을 하지 않는다.

```text
Roots
   ↓
bounded worker pool
   ↓
metadata
   ↓
aggregation
```

파일 수만큼 task를 생성하지 않는다.

---

# 60. Async 정책

**Tokio를 무조건 넣지 않는다.**

실제 async I/O 필요가 생길 때 도입한다.

파일 스캔은:

```text
sync filesystem API
+
bounded parallel worker
```

로 충분하면 그대로 유지한다.

---

# 61. Error Model

```rust
pub enum ZenithError {

    PermissionDenied,

    PathNotAllowed,

    ChangedSinceScan,

    SignatureMismatch,

    ToolUnavailable,

    ExternalCommandFailed,

    Io,
}
```

---

# 62. 사용자 메시지

절대:

```text
EPERM: errno 1
```

만 보여주지 않는다.

예:

```text
Could not clean Xcode cache

macOS denied access to this folder.

[Reveal in Finder]
```

---

# 63. 테스트 전략

Zenith에서 가장 중요한 부분이다.

실제 `$HOME`을 이용한 destructive test는 금지한다.

---

# 64. Test Fixture

```text
/tmp/zenith-fixture/

├── .cargo/
├── .cache/
├── Library/
├── .ssh/
├── .git/
└── Projects/
```

---

# 65. 반드시 통과해야 하는 테스트

```text
delete("/")
→ rejected


delete("~")
→ rejected


delete("~/.ssh")
→ rejected


cache/symlink → ~/.ssh
→ ~/.ssh traversal 없음


scan 이후 inode 변경
→ deletion rejected


unknown signature
→ deletion rejected
```

---

# 66. Signature Release Rule

Signature 하나를 추가할 때:

```text
앱 설치

↓

데이터 생성

↓

대상 경로 확인

↓

삭제

↓

앱 재시작

↓

캐시 재생성 확인

↓

설정 유지 확인

↓

로그인 유지 확인

↓

merge
```

한다.

---

# 67. v0.1 범위

## MUST

```text
Tauri 2

Svelte 5

Rust

macOS arm64


Tray Icon

Quick Panel

Desktop Dashboard


Targeted Scan

AI caches

Developer caches

Docker inspection

LLM model inventory


Risk Tier

DeletePlan

Safety Engine

Clean

Clean Result


Memory Inspector

Settings

Launch at Login
```

---

# 68. v0.1에서 하지 않을 기능

```text
Project 전체 검색

node_modules hunter

.venv hunter

duplicate files

unused applications

scheduled cleaning

cloud sync

automatic update

Intel support

App Store

privileged helper
```

---

# 69. v0.2 — Project Waste Scanner

다음 단계에서는 사용자가 개발 폴더를 등록한다.

```text
Project Roots

~/dev

~/Projects

~/workspace
```

그리고:

```text
Projects

48.2 GB reclaimable


old-dashboard

node_modules
6.1 GB

Last modified
148 days ago


rust-test

target
8.4 GB

Last modified
91 days ago


agent-experiment

.venv
3.2 GB

Last modified
182 days ago
```

를 보여준다.

---

# 70. Git-aware Safety

Project Waste Scanner에서는 Git 상태도 검사한다.

다음 상황에서는 자동 삭제 금지:

```text
uncommitted source files

unknown build directory

active repository

currently running process
```

`target`, `.venv`, `node_modules` 등 명확하게 재생성 가능한 디렉터리만 별도로 처리한다.

---

# 71. v0.3 — Keep Awake

Zenith가 단순 Cleaner에서:

> Developer Mac Utility

로 확장되는 첫 번째 기능이다.

---

# 72. Keep Awake 목표

사용자가 특정 프로그램을 등록할 수 있다.

예:

```text
Keep Awake

When these apps are running:

[✓] Docker Desktop

[✓] Claude

[✓] Terminal

[ ] Cursor
```

---

# 73. 사용 사례

예를 들어:

```text
Claude Code

긴 agent 작업 실행 중

↓

사용자 자리 비움

↓

Mac idle

↓

sleep

↓

작업 중단
```

을 방지한다.

---

# 74. Keep Awake Rule

```rust
pub struct AwakeRule {

    pub app_id: String,

    pub executable: String,

    pub behavior: AwakeBehavior,

    pub enabled: bool,
}
```

---

# 75. AwakeBehavior

```rust
pub enum AwakeBehavior {

    PreventSystemSleep,

    KeepDisplayAwake,
}
```

두 옵션을 구분한다.

---

# 76. Prevent System Sleep

의미:

```text
Mac 자체가 idle sleep으로 들어가지 않도록 함
```

예:

```text
Claude agent
Docker task
ffmpeg render
model inference
```

같은 장시간 작업에 적합하다.

---

# 77. Keep Display Awake

의미:

```text
디스플레이도 sleep하지 않도록 요청
```

별도 옵션으로 제공한다.

기본값은 OFF.

---

# 78. 화면 Lock과의 차이

Zenith UI에서는:

```text
Prevent Lock
```

이라는 표현을 사용하지 않는다.

macOS의:

```text
System Sleep

Display Sleep

Screen Lock
```

은 동일한 기능이 아니기 때문이다.

Zenith가 power assertion으로 관리할 것은:

```text
Prevent System Sleep

Keep Display Awake
```

두 개다.

OS나 조직의 보안 정책으로 설정된 화면 잠금을 우회하는 기능은 만들지 않는다.

---

# 79. macOS 구현

Rust의 macOS 모듈에서 IOKit API를 사용한다.

개념적인 구조:

```text
AppWatcher

    ↓

matching process running?

    ↓ YES

PowerAssertion.acquire()

    ↓

IOKit


matching process exited?

    ↓

PowerAssertion.release()
```

Apple의 `IOPMAssertionCreateWithName`은 power management system에 특정 시스템 동작을 요청하는 API다. 

---

# 80. RAII로 구현

power assertion을 수동 ID 관리로 여기저기 흩뿌리지 않는다.

```rust
pub struct PowerAssertion {
    id: IOPMAssertionID,
}
```

생성:

```rust
impl PowerAssertion {

    pub fn acquire(
        behavior: AwakeBehavior
    ) -> Result<Self, PowerError> {

        // IOPMAssertionCreateWithName

    }
}
```

해제:

```rust
impl Drop for PowerAssertion {

    fn drop(&mut self) {

        // IOPMAssertionRelease(self.id)

    }
}
```

이 방식이면 Rust object가 사라질 때 assertion도 반드시 release되도록 설계할 수 있다.

---

# 81. Keep Awake State Machine

```text
Disabled
   │
   ▼
Watching
   │
   │ matching app detected
   ▼
Active
   │
   │ matching apps gone
   ▼
Watching
```

Zenith 종료 시:

```text
Active
 ↓
Drop PowerAssertion
 ↓
macOS normal power behavior
```

로 돌아와야 한다.

---

# 82. Process Watcher

Keep Awake rule이 하나도 없으면 process watcher 자체를 실행하지 않는다.

```text
rules.empty()

→ watcher OFF
```

Rule이 존재하면:

```text
약 5 sec
```

간격으로 앱 상태를 검사한다.

매 100ms 같은 aggressive polling은 금지한다.

---

# 83. Keep Awake 메뉴바 표현

Keep Awake가 활성화되었을 때만 작은 상태를 보여준다.

```text
Zenith

Keep Awake
Active

Claude is running

System sleep prevented

[Disable]
```

---

# 84. Quick Manual Keep Awake

추후 다음 기능도 추가할 수 있다.

```text
Keep Awake

○ 30 minutes

○ 1 hour

○ Until I turn it off
```

즉 `caffeinate` 같은 기능을 Zenith UI에서 제공하는 것이다.

---

# 85. v0.4 — Smart Rules

향후에는:

```text
When Claude runs

→ Prevent sleep


When Docker runs AND container exists

→ Prevent sleep


When ffmpeg runs

→ Prevent sleep


When battery < 15%

→ release assertion
```

같은 rule engine으로 확장 가능하다.

---

# 86. 향후 전체 구조

최종적으로 Zenith는 다음 세 축을 갖는다.

```text
                 Zenith

        ┌──────────┼──────────┐

        ▼          ▼          ▼

      Clean      Inspect     Assist

        │          │          │

      Cache      Storage    Keep Awake

      Docker     Memory     App Rules

      Models     Process    Automation
```

---

# 87. 제품 범위 원칙

하지만 이 순서를 반드시 지킨다.

```text
Cleaner

↓

잘 작동함

↓

매일 직접 사용함

↓

불편한 점 발견

↓

Inspector 강화

↓

Keep Awake

↓

Automation
```

처음부터 만능 Mac manager를 만들지 않는다.

---

# 88. 구현 순서

## Phase 0 — Skeleton

```text
Tauri 2

Svelte 5

Tray

Quick Panel

Desktop Window
```

완료 조건:

> 메뉴바에서 Zenith를 열고 Desktop Dashboard까지 이동 가능.

---

## Phase 1 — Scanner

구현:

```text
Signature registry

Path expansion

Directory walker

Size aggregation

ScanResult

Tauri Channel
```

완료 조건:

> 실제 Mac에서 Go/Cargo/npm/uv 등 캐시 크기가 표시된다.

---

## Phase 2 — Safety

구현:

```text
Blacklist

Symlink protection

DeletePlan

TOCTOU validation

Risk tier
```

완료 조건:

> Cleaner가 임의 Path를 직접 삭제할 방법이 없다.

---

## Phase 3 — Cleaner

구현:

```text
Plan execution

Progress

Error handling

Reclaim measurement
```

완료 조건:

> UI에서 선택한 cache만 삭제할 수 있다.

---

## Phase 4 — AI

```text
Claude

Cursor

Gemini

Codex

Aider/OpenCode
```

Signature 추가.

완료 조건:

> 설정/인증 정보에 영향을 주지 않고 cache만 정리 가능.

---

## Phase 5 — Docker / Models

```text
Docker inspection

Docker prune

Ollama

HuggingFace

LM Studio
```

완료 조건:

> stateful 데이터는 자동 선택되지 않는다.

---

## Phase 6 — Memory

```text
Memory summary

Processes

Compressed memory

Swap
```

완료 조건:

> 메뉴바에서 현재 메모리 문제의 원인을 빠르게 파악 가능.

---

## Phase 7 — Polish

```text
Animations

Empty states

Error states

Dark mode

Settings

Launch at Login

DMG
```

---

## Phase 8 — Keep Awake

Cleaner가 안정화된 후 구현한다.

```text
Process watcher

Power assertion

App rules

Manual duration

Menu bar status
```

---

# 89. UX Acceptance Criteria

Zenith를 켠 사용자가 **설명을 읽지 않고도** 다음을 할 수 있어야 한다.

### 10초 이내

```text
얼마나 공간이 남았는지 확인
```

### 20초 이내

```text
무엇이 공간을 먹는지 확인
```

### 30초 이내

```text
안전한 캐시 정리
```

---

# 90. Safety Acceptance Criteria

다음 테스트가 하나라도 실패하면 release하지 않는다.

```text
~ 삭제 불가능

/ 삭제 불가능

~/.ssh 삭제 불가능

~/.aws 삭제 불가능

.git 삭제 불가능

symlink traversal 불가능

unknown path 삭제 불가능

scan 이후 변경된 path 삭제 불가능

Tier 3 자동 선택 불가능
```

---

# 91. Performance Acceptance Criteria

release build 기준:

```text
Idle

CPU ≈ 0

filesystem polling 없음


Menu Closed

Memory polling 없음


Scanning

UI freeze 없음


Cleanup

진행 상황 표시
```

---

# 92. v1의 최종 경험

사용자는 메뉴바의 Zenith를 클릭한다.

```text
Zenith

18.4 GB cleanable

AI             3.2 GB

Developer      8.3 GB

Docker         6.9 GB
```

Clean을 누른다.

```text
Cleaning...

Go

Cargo

Gemini

Docker
```

몇 초 후:

```text
Done

17.9 GB reclaimed
```

끝이다.

상세 정보가 궁금할 때만 Desktop Dashboard를 연다.

---

# 93. 제품의 최종 방향

Zenith는:

> Mac을 최적화한다고 주장하는 프로그램

이 아니다.

Zenith는:

> **내 개발 환경에서 무엇이 리소스를 사용하고 있는지 이해하고, 내가 필요 없는 것만 안전하게 정리할 수 있게 해주는 도구**

다.

첫 번째 버전에서는 **Storage Cleaner**를 아주 잘 만든다.

두 번째로 **Resource Inspector**를 만든다.

그 뒤에:

```text
특정 앱 실행
        ↓
Zenith 감지
        ↓
Keep Awake
        ↓
작업 완료
        ↓
자동 해제
```

같은 작은 자동화를 하나씩 붙인다.

Zenith가 계속 커지더라도 핵심 원칙은 유지한다.

> **작고, 조용하고, 사용자가 요청한 일만 한다.**

이 버전이면 구현하면서 애매한 부분이 상당히 줄어들 거야. 특히 아키텍처에서 중요한 건 **Svelte는 UI 상태와 인터랙션만 담당하고, filesystem/Docker/process/power 같은 OS 로직은 무조건 Rust에 둔다**는 경계를 유지하는 거야.

그리고 Keep Awake는 나중에 꽤 좋은 기능이 될 것 같아. `Claude/Codex가 긴 작업 중 → 자동으로 sleep assertion 획득 → 작업 종료 → Rust RAII로 자동 release`까지 되면, 단순 청소앱보다는 **개발자용 작은 Mac 컨트롤 센터**라는 제품 방향이 생겨. 
