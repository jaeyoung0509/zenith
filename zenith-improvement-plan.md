# Zenith UI/UX 개선 + 신규 기능 통합 로드맵

범위: 1차 종합 리뷰(report.md) 이후 "무엇을 고치고 무엇을 넣을까"를 5차원 병렬 검토 — 디자인(improve-design.md) · 비판적사고(improve-critic.md) · 아키텍처(improve-architecture.md) · 코드리뷰/FE(improve-code.md) · 보안(improve-security.md). 원본은 같은 폴더.
표기: [D]디자인 [C]비판 [A]아키 [F]FE코드 [S]보안. 라인번호는 각 에이전트 실측.

---

## 0. 한 줄 결론

**새 기능보다 "이미 앱 안에 있는 최고 패턴을 전 경로에 강제"하는 게 가장 싸고 효과 크다.** 5차원 전부 같은 결론. 확인 다이얼로그(`CleanupReviewDialog`)·2단계 graceful→force(`DevelopmentServersView`)·부분측정 동의(`DeveloperArtifactsView`)·lease 재검증(`dev_ports`)·roving tabindex(`SegmentedTabs`)가 각각 **한 곳에서만** 쓰이고 나머지는 각자 다시 만들었다.

---

## 1. 5차원 수렴 지점 (3차원 이상 독립 지적)

| 주제 | 수렴 내용 | 차원 |
|---|---|---|
| **영구삭제 무확인 경로 8개** | Quick Clean · Category Detail Clean · Docker prune 4종 · Models 삭제 · (Volumes만 확인 있음). Trash로 복구 가능한 경로엔 3단계 안전장치, 실제 영구삭제엔 0~1단계 → **안전 예산이 정확히 거꾸로** | D·C·A·F·S |
| **확인은 FE가 아니라 Rust가 강제** | FE 다이얼로그는 경계 아님(SAFETY.md:3). 플랜에 `stage: Created→Confirmed` + `execute_clean`이 Confirmed 아닌 플랜 거부. 최상위 티어(prune -a·모델 영구삭제)는 백엔드 챌린지 문자열 왕복. raw형 3경로(docker/model/terminate)는 plan 단계 자체 신설 필요 | A·S·D |
| **확인 피로 방지 = 티어 정확화** | 확인 횟수 늘리지 말고 재배치. Safe=요약 1회, Rebuild=목록 확인, 영구삭제=타이핑. `system.intensive.*` risk=safe는 **오분류**라 rebuild로 강등 | C·S·D |
| **Quick Panel 삭제 권한 제거** | `quick.json:15-16` `create-delete-plan`·`execute-clean` 제거. 대체: `prepare_quick_safe_clean(scan_id)`→`execute_quick_safe_clean(plan_id)` 쌍. FE 기여값은 scan_id 하나. 백엔드가 `rebuild_count==0 && manual_count==0` 사후 단언. AGENTS.md:39 자체 규약 위반 해소 | S·D·C·A |
| **삭제 저널** | 현재 삭제 내역 영속 기록 0(`executor.rs:81` 소멸). 신규 `journal/` JSONL append-only, sink 트레이트로 실행기에 주입(AuditStore 재사용 불가: 512KB 초과 시 저장 거부·파싱 실패 시 전량 폐기). 조건: 0600 + `~/` 축약 + 보존기간 상한 + 진단 내보내기 기본 제외 | D·A·S·F·C |
| **Trash 통일 + 복구 진입점** | HF/LM Studio 모델만 영구삭제(`models_inventory/deleter.rs:96`) → 지금 바로 `trash::delete`로. Trash 워크플로 3종에 "휴지통 열기" 버튼 0개 → 추가. 전면 병합은 금지(타깃 타입이 다름), seam은 `clean_target` 수준 per-target sink. `~/.Trash`를 blacklist에 추가, Trash 비우기 기능은 만들지 말 것 | D·A·S·C |
| **취소** | 스캔 취소: `large_files` 패턴 이식(쉬움). 클린 취소: **타깃 경계에서만**, 트리 중간 중단 금지. 취소 커맨드는 게이트를 잡지 않는다는 규약 명문화(현재 우연히 동기 fn이라 동작). 선결: Partial=`success:true` 수정 안 하면 취소가 성공으로 저널됨 | A·S·D·F |
| **회수량 표기 반전** | `actual_disk_free_delta`(실측)를 헤드라인, 논리 삭제 크기를 보조로. 차이 10%↑ 시 하드링크/클론 각주. `formatBytes` 1024진수+SI 라벨 불일치 수정. Docker "보고 기준" 출처 라벨 | D·F·C |
| **탭 재편 선결 = revision 정합** | FE 폴백 revision 3 vs 백엔드 5(`settings.svelte.ts:123`). 기본 탭 배열 3곳 불일치. 이걸 먼저 고치고 revision 6. `Dashboard.svelte:75 tabGroups`가 이미 있어 재편 자체는 S | D·A·F |
| **Windows 패리티 순서** | 안전 테스트 이식(`#![cfg(unix)]`) → uid→SID(`agent_activity/mod.rs:406`에 정답 존재) → graceful 분리 → 그 다음 기능. 순서 역전 금지. `ProcessLifecycleProvider` + `OwnerId` newtype(u32였기에 1000 상수가 컴파일됨) | S·A·C |

---

## 2. 신규 기능 판정 (5차원 병합)

| 기능 | D | C | A | S | 최종 | 조건 |
|---|---|---|---|---|---|---|
| 삭제 저널 + Activity 뷰 | 채택 최우선 | 넣기 #— | 선행 4 | 조건부 | **채택 P0** | 0600·`~`축약·보존기간·진단 제외 |
| 공통 확인 다이얼로그(티어별) | P0 | 조건부(피로) | ① 선행 2·6 | 채택(전제) | **채택 P0** | Rust stage 강제, 티어 재분류 동반 |
| Quick 권한 축소 + Safe 전용 커맨드 | P0 | 넣기 #1 | — | 채택 최우선 | **채택 P0** | 커맨드 쌍, 인라인 확정 바(모달 아님) |
| Trash 통일 + 휴지통 열기 | 채택 | 넣기 #2 | 모델만 즉시 | 조건부 | **채택 P0(모델)/P1(나머지)** | Trash 비우기 금지, 2값 표기 |
| 회수량 이중 표기 | 채택 | — | — | — | **채택 P1** | `ByteValue` 수렴 선행 |
| 스킵/불완전 사유 집계 | 채택 | — | — | — | **채택 P1(최저 비용)** | `DeveloperArtifactsView.statusLabel` 재사용 |
| 스캔 취소 / 클린 취소 | P0 | 부작용 경고 | ③ 선행 1·3 | 채택/조건부 | **채택 P1** | 게이트 규약, 타깃 경계 |
| 탭 11→8 재편 | IA안 제시 | — | ⑨ 최저 비용 | — | **채택 P1** | revision 정합 선행 |
| 프로젝트별 캐시 뷰 | 채택 | — | — | 채택(경로 커맨드 무증가) | **채택 P2** | opaque id만 |
| 명령 팔레트 + 전역 단축키 | 채택 | — | — | — | **채택 P2** | 팔레트에서 파괴 실행 금지, 확인 화면 이동만 |
| 알림 센터 | 보류 | 기각(2벌 통합 먼저) | ⑧ 2차 | 조건부 | **보류** | 발신자 2벌 통합·GLOBAL_STORE 이동 후 |
| 자동 업데이터 | 조건부 | 조건부 | 조기 착수 권장 | **기각(현시점)** | **의견 분열 → 조건부** | A: "회수 수단 0을 메우는 유일한 해법". S: "미서명 자동배포는 수동보다 나쁨". 합의점: **코드서명 완료 직후 1순위**, 그 전엔 provenance attestation만 |
| 예약/자동 정리 | 보류 | 기각 | ⑤ 3차 최후 | 조건부 | **보류(가장 늦게)** | 저널·Trash·취소·min_age 전용 티어·종료 훅 전부 선행 |
| 시그니처 커스터마이즈 | 기각 | 기각 | ⑦ 부활 금지 | 조건부(exclusions만) | **기각 → exclusions/min_age 오버레이만** | `load_from_dir` 부활 금지, 새 로더 아닌 설정 확장 |
| CLI/헤드리스 | 기각 | 조건부(읽기만) | — | 조건부 | **기각(삭제) / 읽기 전용만** | capability 우회 = quick에서 뺀 권한이 CLI로 회귀 |
| 위젯/상주 게이지 | 기각 | — | — | — | **기각** | Quick Panel 중복 + 전력 |
| 앱 내 버그리포트 | 보류 | — | — | 조건부 | **보류** | 리댁션 통합(Basic·PAT·AKIA·URL) 선행 |
| 회수 트렌드 차트 | 보류 | — | — | — | **보류** | 저널 부산물로 |

---

## 3. 비판 차원 단독 입장: "빼기"

다른 4차원은 "고치고 넣기"를 논했고, 비판 차원만 **제거**를 제안. 병합하지 않고 별도 기록.

| 영역 | 줄수 | 판정 | 근거 |
|---|---|---|---|
| AI Control Center | 3,586 | **제거** | `policy.rs:12` 자문만 반환, 산출물=알림 문구 5종. git RCE·.env 유출 위험 집중 |
| Project Cockpit 어댑터·훅·이벤트 | ≈1,231+ | **제거/분리** | 8개 어댑터 전부 Process-only, `install_integration` 항상 거부, Attention Counter 영구 0 |
| 메모리 프로세스 종료 | 1,110 | **제거** | Activity Monitor 열등 복제 + lease 없는 유일 파괴 경로 |
| Keep Awake 규칙 워처 | ≈2,229 | **축소** | 5초마다 전체 프로세스 스캔. `caffeinate` 존재. 수동 타이머 405줄만 |
| AI 사용량 | 1,940 | **분리** | OAuth 리스너가 삭제 도구에 있을 이유 없음 |
| Docker prune | 997 | **읽기 전용화** | `docker system prune`과 동일. `prune -a` 로컬 이미지 손실 |

합계: 50,297줄 중 **13,800줄(27%)** 제거/분리 가능. Critical 3건이 전부 이 바깥 영역에 몰림. 테스트 5,641줄 중 FE는 소스 텍스트 grep, Rust는 Windows 0건 → "회귀를 잡는 게 아니라 리팩터를 막음".

**내 판단:** 제거 여부는 제품 오너 결정. 단 "메모리 종료 제거 or lease화"는 양자택일 필수 — 현 상태 유지는 선택지 아님.

---

## 4. 착수 순서 (5차원 합의)

```
0. 선결 수정 (기능 아님, 1주)
   ├ operation_gate poison → into_inner()             [A1·S5]
   ├ Partial=success:true 수정 + failed_bytes 계상     [A·S·D·F]
   ├ FE 세대 토큰 3곳 + utils/tauri.ts 무동작층 제거    [F]
   ├ settings revision 폴백 3→5 정합                   [F·A8]
   └ 로그/설정/저널 0600 + 리댁션 통합                 [S3]

1. P0 (안전, 2~3주)
   ├ quick.json 권한 2개 제거 + prepare/execute_quick_safe_clean 쌍   [S·D·C]
   ├ 영구삭제 8경로에 ReviewDialog 강제 (Rust stage 게이트 동반)      [D·A·S]
   │   └ Category Detail:74 · Docker 4종 · Models · QuickPanel 인라인 확정바
   ├ system.intensive.* risk=safe → rebuild                          [S·C]
   ├ models_inventory/deleter.rs:96 → trash::delete                  [A·D]
   └ terminate_process_group lease화 or 제거 (양자택일)              [S1·A6·C]

2. P1 (신뢰, 3~4주)
   ├ 스킵 사유 집계 (최저 비용, 먼저)                  [D·F⑥]
   ├ 삭제 저널 JSONL + Activity 뷰                     [D·A4·S·F]
   ├ Trash 3종에 "휴지통 열기" + 회수량 2값 반전        [D·F⑦]
   ├ 스캔 취소 (large_files 패턴) → 클린 취소(타깃 경계) [A3·S]
   ├ 탭 11→8 재편 (Storage/Containers/Models/Runtime/Awake/AI/Activity/Settings) [D·A9·F]
   └ 라이트 모드 시맨틱 3색 대비 4.5:1 수정 (현 3.1~3.8) [D]

3. Windows 트랙 (병행)
   safety_tests 이식 → uid→SID (ProcessLifecycleProvider) → graceful 분리 → 그 전까지 Windows 삭제/종료 capability 비활성

4. P2 / 조건부
   ├ 프로젝트별 캐시 뷰 · 명령 팔레트 · 모달 6종 <dialog> 통일 · 사이드바 roving tabindex
   ├ 자동 업데이터 — SignPath 서명 완료 직후 1순위, 그 전 provenance attestation만
   └ 예약 정리 · exclusions 오버레이 — 위 전부 끝난 뒤

기각: 시그니처 새 경로 추가 · CLI 삭제 서브커맨드 · 플러그인 · 클라우드 동기화 · 위젯 · autopilot 실행화
```

---

## 5. 하지 말 것 (과설계 경고, A·C)

1. 파괴 경로 관통 단일 구체 타입 `DestructivePlan` — Option 수프. 공통화는 트레이트 계약·저널 엔트리·PlanVault 셋만.
2. 범용 이벤트 버스/제네릭 스케줄러 — Supervisor 80줄(태스크 Vec + 취소 플래그 + ExitRequested 훅)로 충분. `ExecutionBudgets` 확장 금지.
3. `load_from_dir` 부활·시그니처 플러그인 — 사용자 요구는 "이 캐시 건드리지 마" = `excluded_signatures` 세분화로 100% 충족.
4. `trash_manager`·`cleaner` 전면 병합 — 타깃 타입 구조적으로 다름.
5. i18n 착수 전 테스트 257건 `toContain(영어)` 이관 안 하면 21개 테스트 파일 동시 붕괴. UI 폴리시 전에 소스 grep 테스트 제거.

---

## 6. 차원별 요약

| 차원 | 핵심 산출 | 원본 |
|---|---|---|
| 디자인 | UX 결함 30건(P0 5) · 위험 경로 매트릭스 15행 · IA 11→8 · 기능 14개 판정 · 라이트 대비 실측 | improve-design.md |
| 비판 | 영역 15개 판정표 · 기능 반론 14 · UX 부작용 8 · README 문장 3 · 빼기3/넣기3 | improve-critic.md |
| 아키텍처 | 선행 구조 변경 10 · 기능 ①~⑩ 적합도 · DestructiveOperation 스케치 · 저널/취소/Trash/시그니처/Windows 보충 · 금지 3 | improve-architecture.md |
| FE 코드 | 먼저 고칠 것 8 · 컴포넌트 재사용 표(확인 UI 4계통) · 개선 ①~⑩ 비용 · createRequest/ConfirmDialog 코드 · a11y/i18n 실측 | improve-code.md |
| 보안 | 사전 정정 3 · 기능 ①~⑫ 보안 영향 · 티어 정책표 · quick 축소 커맨드 시그니처 · 선행 5 · 공급망 체크리스트 | improve-security.md |


---

# 부록 — 디자인/UX 차원 원본 (improve-design.md)

## Zenith UI/UX 평가 및 개선 제안 — v0.3.8

대상: `jaeyoung0509/zenith` (Tauri 2 + Svelte 5, macOS/Windows). 읽기 전용 클론 기준.
근거: 실제 컴포넌트 코드 + `DESIGN.md` + 1차 종합 리뷰(`report.md`). 미확인 추론은 "(추정)" 표기.

**전제 요약(실측):**
- 모달 표면 6개 중 네이티브 `<dialog showModal()>`(포커스 트랩·Esc 기본 제공)은 `CleanupReviewDialog.svelte` **1개뿐**. `role="dialog" aria-modal` div가 3개(Memory/DevServers/Docker), 나머지 3개(Models 삭제, AI Control 프리뷰, Awake 규칙 추가)는 role·Esc·포커스 관리 **전부 없음**.
- `ScanFreshnessNotice`는 전체 저장소에서 **`CategoryDetailView.svelte:82` 단 1곳**에서만 렌더된다. StorageView·QuickPanel에는 없다.
- `EmptyState`는 17개 뷰 중 **3곳**(Docker/Models/DevServers)만 사용. 나머지는 뷰마다 각자 다른 하드코딩 빈 상태.
- 온보딩 패키지는 **존재하지 않는다**(`find -iname "*onboard*"` → 0건). `DESIGN.md`는 "keeps the onboarding package on its own independent Vite pipeline"이라고 서술 중 → 문서 드리프트.
- UI i18n 없음. 한국어는 `tauri.conf.json:71` NSIS 설치 관리자 언어에만 존재.
- 전역 단축키·앱 메뉴바 없음(`src-tauri/Cargo.toml`에 `tauri-plugin-global-shortcut` 부재, `lib.rs:237` `Menu::with_items`는 트레이 메뉴 전용). 자동 업데이터도 없음.

---

## UX 결함/개선

> 형식: `{우선순위} | {파일:라인} | {현재 문제}. {개선안}. {비용}`

**P0 | src/routes/dashboard/CategoryDetailView.svelte:74-78, 116-130** | 1차 리뷰가 놓친 **두 번째 무확인 영구삭제 경로**. "Clean {bytes}" 버튼이 `scanStore.cleanItems()`를 직접 호출한다 — `CleanupReviewDialog`를 거치지 않는다(리뷰 다이얼로그는 `StorageView.svelte:362-369` 한 곳만 사용). 카테고리 상세에서 Rebuild 티어까지 선택한 상태로 1클릭 영구삭제가 가능하다. 개선: `StorageView.handleCleanSelected` 와 동일하게 `review` state → `CleanupReviewDialog` 경유로 통일하고, 리뷰 다이얼로그를 뷰가 아니라 `scanStore.cleanItems()` 진입점에 강제(호출 계약을 `requestClean(items)`로 바꿔 UI 우회 자체를 불가능하게). 비용: S

**P0 | src/routes/quick/QuickPanel.svelte:178-184, 309-323** | "Clean Safe" 1클릭이 `selectQuickCleanDefaults()` → `cleanSelected()`를 연속 호출해 확인·미리보기 없이 영구삭제. 360px 상시최상위 투명 창이 메인 창과 동일한 삭제 권한(`capabilities/quick.json:15-16`에 `allow-create-delete-plan`+`allow-execute-clean`)을 갖는다. 개선: 버튼을 2단계로 — 1클릭 시 패널 하단이 "N개 · X GB · Safe만 · [확인] [자세히]" 인라인 확정 바로 전환(모달 아님, 360px에 맞는 확정 스텝), [자세히]는 대시보드 리뷰 화면으로 딥링크. quick capability에서 `execute-clean` 제거하고 "백엔드가 생성한 Safe 전용 플랜만 실행"하는 전용 커맨드로 축소. 비용: M

**P0 | src/routes/dashboard/ModelsView.svelte:194-228 + src-tauri/src/models_inventory/deleter.rs:96** | 모델 삭제 확인 문구가 "You will need to re-download this model"뿐이고 **휴지통이 아니라 영구삭제**(`SafeTreeDeleter::delete_path`)라는 사실이 UI 어디에도 없다. 앱의 나머지 워크플로(Large Files/Applications/Developer Artifacts)는 전부 "Moves to Trash, never permanently deletes"라고 광고하므로 사용자 멘탈 모델과 정반대. 삭제 경로 문자열도 표시하지 않는다. 개선: 다이얼로그에 (a) 전체 경로, (b) `Permanently deleted — not moved to Trash` 붉은 배지, (c) Ollama처럼 CLI 위임이 가능한 provider와 파일시스템 직접 삭제 provider를 서로 다른 문구로 구분. 장기적으로는 이 경로도 Trash 경유로 통일. 비용: S

**P0 | src/routes/dashboard/DockerView.svelte:118-132, 152-166, 186-200, 220-234** | prune 버튼 5개 중 **4개가 확인 없이 즉시 실행**된다. 특히 `container.docker.unused_images`(:186-200)는 백엔드가 `image prune -a -f`를 돌려 **로컬에서만 빌드한 이미지도 복구 불가하게 삭제**하는데 UI 배지는 "Rebuild"이고 확인 단계가 없다. 유일한 확인은 volumes(:302-320). 개선: `unused_images`·`stopped_containers`를 volumes와 동일한 확인 다이얼로그로 승격하고, `unused_images` 확인문에 "레지스트리에 없는 로컬 전용 이미지는 다시 빌드해야 합니다"를 명시. Build Cache·Dangling만 무확인 유지. 비용: S

**P0 | src/lib/stores/scan.svelte.ts:373-422, 429-500 + src-tauri/src/cleaner/executor.rs** | 스캔·클린 모두 취소 커맨드가 없다(Large Files/Developer Artifacts는 `cancelLargeFileScan`/`cancelDeveloperArtifactScan`이 있음 — 같은 앱 안에서 규약이 갈린다). 수 분짜리 FS I/O가 전역 게이트를 쥔 채 돌아가고, 유일한 탈출구는 강제 종료 = 반쯤 지워진 캐시. 개선: 캐시 스캔/클린에도 `cancel_scan`/`cancel_clean`을 추가하고 `LargeFilesView.svelte:362-367`과 동일하게 진행 중일 때만 Cancel 버튼 노출. 클린 취소는 "현재 항목까지 완료 후 중단(항목 경계 취소)"으로 정의하고 그 의미를 버튼 툴팁에 적어라. 비용: L

**P1 | src/lib/stores/scan.svelte.ts:151-154, 110-117** | TTL 만료 시 `invalidate()`가 `selectedMap`을 **통째로 비운다**. `maybeAutoRescan()`이 창이 보이는 동안 1초 틱으로 자동 재스캔하므로, 사용자가 수십 개 항목을 손으로 골라둔 선택이 예고 없이 초기화된다(자동 재스캔 후에는 `syncSelectionFromScan`이 safe 기본값으로 덮어씀). 개선: 자동 재스캔 시 선택을 항목 id 기준으로 이월하고, 이월 실패분만 "N개 항목이 사라져 선택에서 제외됨"으로 고지. 사용자가 수동 선택을 건드린 세션에서는 자동 재스캔을 보류하고 배너로만 알림. 비용: M

**P1 | src/lib/components/CleanResultModal.svelte:87-158** | 결과 모달이 "무엇이 얼마나 지워졌나"만 말하고 **되돌릴 수단·사후 추적 수단이 없다**. Done 버튼 하나로 닫히면 그 실행 기록은 어디에도 남지 않는다(`executor.rs`에 삭제 저널 없음). 개선: 푸터에 `[Done] [로그 저장…] [Trash 열기(해당하는 경우)]`, 그리고 항목마다 삭제된 절대 경로를 접힌 상태로 제공(현재는 `item.name`만). 삭제 저널(신규 기능 제안 #1)의 진입점이 여기여야 한다. 비용: M

**P1 | src/lib/components/CleanResultModal.svelte:123-143 + src/lib/utils/cleanResult.ts:9-23** | 프론트가 `cleanOutcome()`으로 백엔드의 `success:true`(Partial)를 **사후 보정**하고 있다. 즉 UI 진실성이 FE 휴리스틱(`error_message`가 비어있지 않으면 partial)에 의존한다 — 백엔드가 실패 사유를 영어 문자열 매칭으로 분류하므로(`executor.rs:336`) 비영어 로케일에서는 사유가 Unknown이 되고 partial 판정도 흔들린다. 개선: 보정 로직을 백엔드로 올리고 `status: success|partial|failed`를 신뢰 가능한 열거형으로 계약화. FE는 표시만. 그 전까지는 partial 카드에 "일부 파일이 잠겨 있었습니다" 대신 **남은 바이트 수**를 함께 노출(현재 `+{bytes_reclaimed}`만 있고 `total_failed_bytes`는 계상되지 않음). 비용: M

**P1 | src/lib/components/CleanResultModal.svelte:88-99** | 회수량 표기가 신뢰를 깎는다. 큰 숫자 `total_reclaimed_bytes`가 주값이고, 실제 디스크 여유 증가분 `actual_disk_free_delta`는 괄호 안 보조값이다. 하드링크·APFS 클론·Docker SI 단위 문제(리뷰 Major)로 주값이 항상 과대평가되므로 **틀린 숫자가 크게, 맞는 숫자가 작게** 표시된다. 개선: 두 값의 위계를 뒤집어 "확보된 여유 공간 X"를 헤드라인, "삭제된 논리 크기 Y"를 보조로. 둘의 차이가 10% 이상이면 "하드링크·복제 블록으로 인해 삭제 크기와 여유 공간 증가분이 다를 수 있습니다" 각주. 비용: S

**P1 | src/routes/dashboard/DockerView.svelte:122, 156, 190, 224 + src/lib/stores/docker.svelte.ts:22-35** | `pruneTarget()`은 회수 바이트를 **반환하는데 뷰가 버린다**(`onclick={() => dockerStore.pruneTarget(...)}`). 사용자는 prune 후 어떤 확인 메시지도 받지 못하고, 카드 숫자가 바뀌는 것으로만 추측해야 한다. 개선: prune 결과를 `InlineNotice variant="success"`로 노출("빌드 캐시 3.2 GB 정리 — Docker 보고 기준"). Docker는 SI 단위로 보고하므로 "Docker 보고 기준"이라는 출처 라벨을 반드시 붙일 것(현재 어댑터가 GB를 1024³로 환산해 7.4% 과대). 비용: S

**P1 | src/routes/dashboard/MemoryView.svelte:337-341** | Force Quit이 Cancel·Quit Normally와 **같은 줄, 같은 크기, 1클릭 거리**에 있다. DevServers는 정확히 반대로 설계돼 있다(`DevelopmentServersView.svelte:268-306`: graceful 다이얼로그 → 실패 시에만 별도 force 다이얼로그). 개선: Memory도 DevServers 2단계 모델로 통일 — 1단계는 [Cancel] [Quit] 만, `terminateProcessGroup(force:false)` 후 프로세스가 남아 있을 때만 별도의 destructive 다이얼로그에서 Force Quit 제시. 비용: S

**P1 | src/routes/dashboard/MemoryView.svelte:327-329 + src/lib/stores/memory.svelte.ts:84** | 확인 다이얼로그는 "이 그룹은 N개 프로세스"라고만 하고 **어떤 PID인지 보여주지 않는다**. 백엔드는 이름 문자열로 프로세스 테이블 전체를 훑어 동명 프로세스에 신호하므로(리뷰 Critical #1) 사용자가 본 것보다 많은 프로세스가 죽을 수 있다. 결과 토스트도 `(N processes)` 건수뿐이고 실제 종료 확인이 없다. 개선: 다이얼로그에 PID·명령줄 목록을 스크롤 영역으로 노출(`proc.pids`는 이미 `MemoryView.svelte:262` 툴팁에 있음 → 본문으로 승격). 결과는 "요청 N건 · 종료 확인 M건 · 잔존 K건"으로 실측 보고. 비용: M

**P1 | src/lib/stores/developmentPorts.svelte.ts:59-60** | 백엔드가 리스너를 못 찾았을 때도 `outcome:'released'`를 돌려주는데(리뷰 Major) UI는 그대로 `"Port {port} released — retry just dev."` 성공 토스트를 띄운다. 거짓 성공은 신뢰 손상 중 가장 비싼 종류다. 개선: outcome을 `released`(시그널 후 포트 해제 확인) / `already_free`(시그널 전부터 리스너 없음) / `still_listening` / `ownership_changed` 4값으로 분리하고 `already_free`는 중립 톤 메시지로. 비용: S

**P1 | src/routes/dashboard/LargeFilesView.svelte:396-408, src/routes/dashboard/ApplicationsView.svelte:270-286, src/routes/dashboard/DeveloperArtifactsView.svelte:455-464** | Trash 워크플로 3종의 결과 카드가 `... · {failed+skipped} not moved` 로 **건수만** 말하고 사유를 말하지 않는다. 그리고 앱 전체에서 가장 값싼 안전장치인 **"휴지통 열기 / 되돌리기"가 하나도 없다** — 이미 Trash에 들어가 있어서 복구가 원클릭인데도. 개선: (a) 세 카드 모두에 `[휴지통 열기]` 버튼 추가(macOS는 Finder Trash, Windows는 Recycle Bin — `show_in_file_manager` 재활용), (b) not-moved 항목은 확장 가능한 목록으로 사유 병기(권한 없음 / 사용 중 / identity 변경 / 깊이 초과). 비용: S

**P1 | src/routes/dashboard/DeveloperArtifactsView.svelte:440-445, 521** | 스킵 사유 표기가 **스캔 중에만** 보인다(`{skippedEntries} skipped`). 스캔이 끝나면 사라지고, 결과 헤더는 `· scan cancelled` / `· result cap reached`만 남는다. 깊이 초과·권한 거부·심링크 회피로 몇 GB가 측정에서 빠졌는지 사후에 알 방법이 없다. 개선: 스캔 완료 후 `InlineNotice variant="warning"`로 "N개 항목을 측정하지 못했습니다 — 사유별 내역 보기"를 상시 노출. `statusLabel()`(:176-187)이 이미 항목 단위 사유 라벨을 갖고 있으니 집계 뷰만 추가하면 된다. 비용: S

**P1 | src/app.css:68-72 vs DESIGN.md "Accessibility and visual QA"** | `DESIGN.md`는 "WCAG AA: Normal text ≥ 4.5:1 in both light and dark themes"를 선언하지만 **라이트 모드 시맨틱 3색 전부 미달**이다(카드 배경 `hsl(0 0% 100%)` 기준 계산): `--warning: 32 95% 44%` → **3.15:1**, `--destructive: 0 84% 60%` → **3.78:1**, `--success: 161 94% 30%` → **3.84:1**. 이 색들은 `InlineNotice.svelte:26-29`, `ScanFreshnessNotice.svelte:7`, `CleanResultModal.svelte:125-137`, `QuickPanel.svelte:299-303`(10px `text-caption`!)에서 **본문 텍스트 색**으로 쓰인다. DESIGN.md는 `--ring` 대비만 수치로 문서화하고 시맨틱 팔레트는 검증하지 않았다. 개선: 라이트 모드 텍스트용 토큰을 분리(`--warning-text: 32 95% 32%` 수준으로 명도 하향, 배경/보더용 `--warning`은 유지)하고 DESIGN.md에 6개 조합의 실측 수치 표를 추가. 비용: M

**P1 | src/routes/dashboard/ModelsView.svelte:194-228, src/routes/dashboard/AiControlCenterView.svelte:157, src/routes/dashboard/AwakeView.svelte:453-588** | 모달 3개가 `role="dialog"`·`aria-modal`·Esc 핸들러·포커스 트랩·포커스 복귀를 **전부 갖고 있지 않다**. 그중 Models는 파괴적 삭제 확인창이다. 저장소에 이미 정답이 있다 — `CleanupReviewDialog.svelte:14-33`이 네이티브 `<dialog>` + `showModal()` + `oncancel` + `previousFocus` 복귀로 전부 처리한다. 개선: 그 파일을 `Modal.svelte` 기본 컴포넌트로 승격하고 6개 모달을 전부 이관. `DockerView.svelte:302`(role은 있으나 Esc 없음), `MemoryView.svelte:97-101`·`DevelopmentServersView.svelte:123-127`(수동 Esc 3벌 중복)도 함께 정리. 비용: M

**P1 | src/routes/dashboard/Dashboard.svelte:212-302 + 전역** | 키보드 내비게이션이 사실상 없다. 사이드바 11개 탭이 각각 독립 tabstop이라 Storage에서 Settings까지 Tab 11번(현재 `role`도 `tablist`가 아니라 `nav` + 개별 `button`), 뷰 전환 단축키·명령 팔레트·전역 핫키 전무(`Cargo.toml`에 `tauri-plugin-global-shortcut` 없음). 반면 `SegmentedTabs.svelte:40-53`와 `ProjectCockpitView.svelte:169-198`은 roving tabindex + 화살표 키를 제대로 구현했다 — 사이드바만 뒤처져 있다. 개선: (a) 사이드바를 `role="tablist" aria-orientation="vertical"` + roving tabindex로 전환(`segmentedTabs.ts:5-33` 로직 재사용), (b) `Cmd/Ctrl+1..9` 탭 전환, `Cmd/Ctrl+R` 재스캔, `Cmd/Ctrl+K` 명령 팔레트, (c) `tauri-plugin-global-shortcut`으로 Quick Panel 토글 핫키. 비용: M

**P2 | src/app.css:132-135 + 전역 grep** | `.focus-ring` 유틸리티가 정의돼 있으나 **사용처 0곳**. 대신 `focus-visible:ring-2`가 6곳, 더 약한 `focus:ring-1`/`focus-visible:ring-1`이 18곳(모든 검색/셀렉트 입력이 여기 해당: `MemoryView.svelte:240`, `CategoryDetailView.svelte:209`, `ModelsView.svelte:119`, `DevelopmentServersView.svelte:163`, `LargeFilesView.svelte:330,340`, `DeveloperArtifactsView.svelte:529` 등). DESIGN.md는 `focus-visible:ring-2 focus-visible:ring-ring`을 규약으로 명시한다. 개선: `focus:ring-1`을 전부 `focus-visible:ring-2 focus-visible:ring-ring`으로 치환하고 `.focus-ring` 삭제하거나 실제로 채택. 비용: S

**P2 | src/routes/dashboard/CategoryDetailView.svelte:226-266** | 리스크 필터가 `SegmentedTabs`를 쓰지 않고 손으로 만든 4개 버튼이다 — `role="tablist"`·`aria-selected`·화살표 키 전부 없고, 활성 상태를 색(`text-success`/`text-warning`/`text-destructive`)으로만 전달한다. DESIGN.md "No information conveyed by color alone" 위반. 개선: `SegmentedTabs.svelte`로 교체하고 badge slot에 각 티어 건수를 넣어라(컴포넌트가 이미 `badge` prop 지원). 비용: S

**P2 | src/routes/dashboard/CategoryDetailView.svelte:280-285, src/routes/dashboard/DeveloperArtifactsView.svelte:549-551** | 가상 리스트(`utils/virtualList.ts`, 테스트까지 있음)가 `LargeFilesView.svelte:78`·`ApplicationsView.svelte:97` **2곳에만** 적용됐다. 캐시 항목 목록과 개발 아티팩트 목록은 전량 렌더 — 대형 모노레포에서 수천 개 `ItemRow`(각각 `RiskBadge`+`Checkbox`+메타 3줄)를 그린다. 개선: 두 목록에 `getVirtualWindow` 적용. `ItemRow`는 높이가 가변이므로 메타 줄을 고정 2줄로 정규화하거나 고정 높이 컴팩트 변형을 추가해야 한다. 비용: M

**P2 | src/lib/utils/format.ts:5-7** | `formatBytes`가 **1024 진수로 나누고 SI 라벨(KB/MB/GB)을 붙인다**. 앱 전체 단일 포맷터라 모든 화면의 숫자가 macOS Finder(1000 진수)와 어긋난다 — "1.5 GB 회수" 후 Finder 여유 공간이 1.6 GB 늘어나면 사용자는 앱을 의심한다. Docker 어댑터는 반대 방향으로 틀렸다(SI 값을 1024로 환산). 개선: macOS는 1000 진수 + `GB`, Windows는 1024 진수 + `GB`(OS 관용)로 플랫폼 분기하거나, 최소한 `KiB/MiB/GiB` 라벨로 정직하게. 비용: S

**P2 | src/routes/dashboard/StorageView.svelte(전체) vs src/lib/components/ScanFreshnessNotice.svelte** | 스캔 신선도 배너가 정작 **주 클린업 화면(StorageView)에는 없다**. StorageView는 "Last scan {timeAgo}"라는 회색 소문자만 보여주고(`:213-219`), 만료 상태는 SelectionToolbar의 버튼 비활성(`isActionDisabled={!scanStore.canClean}`)으로만 암시된다 — 왜 비활성인지 설명이 없다. 개선: `<ScanFreshnessNotice />`를 StorageView 최상단과 QuickPanel(cleanup 섹션 위)에도 배치. 비활성 버튼에는 `title`로 사유를 붙여라(DESIGN.md "disabled explanation" 규약이 Button에 있는데 여기선 안 지켜짐). 비용: S

**P2 | src/lib/components/SelectionToolbar.svelte:127-138** | Manual 항목이 하나라도 선택되면 액션 버튼 전체가 비활성되고 사유는 `title` 툴팁에만 있다(키보드·터치 사용자는 볼 수 없음). Manual은 애초에 `ItemRow.svelte:28-31`에서 선택 자체가 차단되므로 이 상태는 상위 카테고리 토글 경로로만 도달 가능한 혼란 상태다. 개선: 툴팁 대신 툴바 안에 인라인 경고 줄 + `[Manual 항목 선택 해제]` 원클릭 버튼. 비용: S

**P2 | src/routes/quick/QuickPanel.svelte:339-356** | Quick Panel 메모리 섹션이 `memory.pressure` 원문(`normal`/`warning`/`critical`)을 그대로 노출한다. 백엔드 압력 판정이 used/total 휴리스틱이라 macOS에서 상시 `warning` 오탐(리뷰 Major)이며, 360px 패널에 늘 주황 게이지가 떠 있으면 **경고 피로**로 진짜 경고까지 무시된다. 개선: (a) 백엔드를 `kern.memorystatus_vm_pressure_level`로 교체하기 전까지 Quick Panel에서는 압력 배지를 숨기고 사용량 게이지만, (b) MemoryView(`:121-124`)에서는 배지에 `(추정)` 또는 판정 근거 툴팁 병기. 비용: S

**P2 | src/routes/dashboard/StorageTools.svelte (파일 전체, 115줄)** | 어디서도 import되지 않는 **죽은 뷰**(`grep -rn "StorageTools"` → 정의 파일 외 0건). Storage 하위 도구 진입 카드 3장이 여기 있는데 실제 IA는 `StorageView`의 `SegmentedTabs`로 대체됐다. 개선: 삭제. 남겨두면 다음 IA 개편 때 두 개의 진실이 충돌한다. 비용: S

**P2 | src/lib/components/ProgressBar.svelte:22-24 + src/lib/stores/scan.svelte.ts:467-473** | 클린 진행률이 **항목 인덱스 비율**이다(`Math.round(event.index / event.total * 100)`). 항목 크기 편차가 수백 배(수 KB temp vs 8 GB DerivedData)라 "90%"에서 몇 분이 더 걸린다. 개선: 플랜에 이미 있는 항목별 바이트로 가중 진행률을 계산하고, 바 아래에 `{현재 항목명} · {처리 바이트}/{전체 바이트}`를 병기. 남은 시간 추정은 하지 마라(변동성이 커서 또 다른 거짓말이 된다). 비용: S

**P2 | src/lib/stores/scan.svelte.ts:486-492** | 클린 완료 후 `await this.runScan()`이 **결과 모달 표시를 막는다**. `cleanItems()`가 전체 재스캔이 끝난 뒤에야 반환하므로, 사용자는 삭제가 끝났는데도 "Cleaning…" 스피너를 수십 초 더 본다(`QuickPanel.svelte:181-183`, `StorageView.svelte:100-102` 모두 반환값을 기다림). 개선: 결과를 먼저 반환해 모달을 띄우고 재스캔은 백그라운드로. 재스캔 완료 시 모달 상단에 "최신 스캔 반영됨" 표시. 비용: S

**P2 | src/routes/dashboard/AwakeView.svelte:426-434** | Keep Awake 규칙 삭제가 확인 없이 즉시 실행된다(아이콘 버튼 1클릭). 파괴적이지 않지만 되돌릴 수 없는 사용자 설정 손실이며, 같은 화면의 Switch 토글과 3px 옆에 붙어 있어 오조작 위험이 높다. 개선: 삭제 후 5초짜리 `[실행 취소]` 스낵바(확인 다이얼로그보다 이 종류의 작업에 적합). 비용: S

**P2 | 전역 / src-tauri/tauri.conf.json:71** | UI에 i18n 레이어가 없는데 설치 관리자만 한국어를 지원한다. 한국어 UI가 붙으면 `Button.svelte:41-47`의 고정 높이(`h-6`/`h-7`), `ItemRow.svelte:107` `w-[12ch]`, `MemoryView.svelte:278` `w-16`, `DevelopmentServersView.svelte:232` `w-20` 같은 하드코딩 폭이 전부 깨진다. 개선: (a) 지금 i18n을 넣지 않을 거라면 NSIS 한국어를 빼서 기대치를 맞추고, (b) 넣을 거라면 지금 고정 폭을 `min-w` + `truncate`로 전환해두는 것이 나중 비용의 1/10. 비용: 현재는 S, 나중에는 L

**P2 | src/lib/components/EmptyState.svelte + 14개 뷰** | 공용 빈 상태 컴포넌트가 3곳에서만 쓰이고, 나머지는 뷰마다 다른 마크업·다른 톤이다(`StorageView.svelte:352-356` 스피너+문장, `CategoryDetailView.svelte:287-289` 회색 한 줄, `LargeFilesView.svelte:554-559` Card+아이콘, `DeveloperArtifactsView.svelte:547` Card 한 줄). DESIGN.md는 "Distinct states for empty search results, no inventory, missing platform capability, and failed loading" 4종을 규정하는데 실제로는 어느 뷰도 4종을 구분하지 않는다. 개선: `EmptyState`에 `variant: 'no-search-result' | 'no-inventory' | 'unavailable' | 'error'` prop을 추가하고 전 뷰 이관. `unavailable`은 `platformCapabilitiesStore.feature(x).reason`을 자동 렌더. 비용: M

---

## 위험 작업 진입 경로 매트릭스

| # | 진입 경로 | 확인 | 미리보기 | Trash / 영구 | 결과 상세 | 되돌리기 |
|---|---|---|---|---|---|---|
| 1 | Quick Panel → Clean Safe (`QuickPanel.svelte:178`) | **없음** | 없음 | **영구** | CleanResultModal (항목명+바이트) | 없음 |
| 2 | Storage → Review cleanup (`StorageView.svelte:85-103`) | `CleanupReviewDialog` (항목 나열 + "cannot be undone") | 항목명·티어·바이트 | **영구** | CleanResultModal | 없음 |
| 3 | Category Detail → Clean {bytes} (`CategoryDetailView.svelte:74`) | **없음** | 없음 | **영구** | CleanResultModal | 없음 |
| 4 | Docker → Prune Build Cache (`DockerView.svelte:122`) | **없음** | 없음 | **영구** | **없음** (반환값 폐기) | 없음 |
| 5 | Docker → Prune Dangling (`:156`) | **없음** | 없음 | **영구** | **없음** | 없음 |
| 6 | Docker → Remove Unused Images (`:190`) | **없음** | 없음 | **영구** (`prune -a -f`, 로컬 빌드 이미지 복구 불가) | **없음** | 없음 |
| 7 | Docker → Prune Containers (`:224`) | **없음** | 없음 | **영구** | **없음** | 없음 |
| 8 | Docker → Prune Volumes (`:258` → `:302`) | 전용 다이얼로그 (`aria-modal`, Esc 없음) | 회수 예상 바이트 | **영구** | **없음** | 없음 |
| 9 | Local Models → Delete (`ModelsView.svelte:176` → `:194`) | 다이얼로그 (role/Esc/포커스 **전부 없음**) | 이름·크기 (경로 없음) | **영구** — UI에 미표기 | **없음** (store가 결과 폐기) | 없음 |
| 10 | Large Files → Move to Trash (`LargeFilesView.svelte:481` → `:433`) | 2단계: prepare plan → 실행 (TTL 15분 카운트다운) | 건수·allocated 바이트 | **Trash** (명시됨) | 이동/미이동 **건수만** | Finder로 수동 (앱 내 버튼 없음) |
| 11 | Applications → Move App to Trash (`ApplicationsView.svelte:453` → `:425`) | 2단계 + TTL 5분 + Library 항목 개별 선택 | 항목 수·바이트·경고 목록 | **Trash** (명시됨) | 이동/미이동 건수만 | 수동 |
| 12 | Developer Artifacts → Move to Trash (`DeveloperArtifactsView.svelte:538` → `:478`) | 2단계 + TTL + **부분측정 체크박스 동의**(`:502-511`) | **경로 전문 목록**(`:487-493`) | **Trash** (명시됨) | 건수만 | 수동 |
| 13 | Memory → Force Quit (`MemoryView.svelte:340`) | 다이얼로그 (Quit과 **동일 계층**) | 프로세스 수·메모리 (PID 미표시) | N/A (SIGKILL) | 토스트 "requested (N processes)" — 실측 아님 | 불가 |
| 14 | Dev Servers → Release (`DevelopmentServersView.svelte:240` → `:268`) | **2단계 에스컬레이션**: graceful 확인 → 실패 시 별도 force 확인 | 포트·PID·프로젝트·bind·영향 설명 | N/A (SIGTERM→SIGKILL) | 토스트 (`already_free`도 "released"로 표기) | 불가 |
| 15 | Keep Awake → 규칙 삭제 (`AwakeView.svelte:428`) | **없음** | 없음 | 설정 손실 | 없음 | 없음 |

**패턴 판정:**
- **모범(12 → 11 → 10 → 14)**: TTL 있는 일회용 플랜 + 항목 나열 + Trash + 실행 직전 재검증. 경로 12는 부분측정 동의까지 갖춰 앱 내 최고 수준. 경로 14는 위험 승급을 강제 2단계로 나눈 유일한 프로세스 종료 UX.
- **최악(1 · 3 · 4~7 · 9)**: 확인·미리보기·결과·되돌리기 4칸 모두 비어 있는 **영구삭제 8개 경로**. 이 8개가 사용자 데이터 손실의 실질 표면 전부다.
- **핵심 비대칭**: 앱이 "Moves to Trash, never permanently deletes"라고 광고하는 경로(10~12)는 확인 3단계인데, **실제로 영구삭제하는 경로(1·3·4~9)는 확인이 0~1단계**다. 안전 예산이 정확히 거꾸로 배분돼 있다.

---

## 정보 구조 재편안

### 현행 (실측 17개 뷰 / 사이드바 11 entry)

```
사이드바 (settings.dashboard_tabs, 기본 7개 · tabDefs 정의는 10개)
├─ [Storage 그룹]
│  ├─ storage          → StorageView ─┬─ SegmentedTabs: Cleanup
│  ├─ disk             → StorageView  ├─ Developer Artifacts   ← 사이드바 disk 탭과 중복 진입
│  ├─ docker           → DockerView   ├─ Large Files
│  └─ models           → ModelsView   ├─ Applications
│                                     └─ Disks                 ← disk 탭과 동일 대상
├─ [Runtime 그룹]
│  ├─ memory           → MemoryView
│  ├─ development_servers → DevelopmentServersView
│  └─ awake            → AwakeView
├─ [AI 그룹]
│  ├─ projects         → ProjectCockpitView ─┬─ Usage      ← usage 탭과 중복
│  ├─ usage            → ProjectCockpitView  ├─ Projects
│  └─ ai_control       → AiControlCenterView ├─ Tool Adapters
│                                            └─ AI Control ← ai_control 탭과 중복
└─ [Preferences]
   └─ settings         → SettingsView (798줄, 8개 섹션 h3, 랜드마크 없음)

기타: CategoryDetailView(모달성 전체화면), ProjectDetailPanel, StorageTools(죽은 코드)
```

**진단:** 17개라는 숫자 자체보다 **동일 목적지로 가는 중복 진입점 3쌍**(`disk`↔Storage/Disks, `usage`↔AI/Usage, `ai_control`↔AI/AI Control)이 문제다. `Dashboard.svelte:222`의 활성 판정 조건식이 이미 그 부채를 드러낸다 — `isTabActive`가 5개 값을 OR로 묶고 있다. 또 그룹 헤더(`:224-230`)는 사이드바가 펼쳐졌을 때만 보이고 접히면 얇은 구분선이 되는데, 접힌 상태(64px)에서는 아이콘 3개가 같은 `HardDrive`/`Sparkles`를 공유해 구분이 불가능하다.

### 제안 (사이드바 7 + Settings, 중복 진입 0)

```
Zenith
│
├─ 🗄  Storage                                  [사이드바 1] — 회수 가능 바이트 배지 유지
│   ├─ Cleanup            (기본, 카테고리 + 리뷰 클린)
│   │   └─ Category Detail  (뒤로가기 가능한 하위 화면, 현행 유지)
│   ├─ Developer Artifacts
│   ├─ Large Files
│   ├─ Applications
│   └─ Disks               ← 사이드바 disk 항목 제거, 여기로 단일화
│
├─ 📦 Containers                                [사이드바 2]
├─ 🧊 Local Models                              [사이드바 3]
│
├─ 📊 Runtime                                   [사이드바 4]  ← 신규 묶음 (Memory/Dev Servers 통합)
│   ├─ Memory & Processes
│   └─ Dev Server Ports
│      ※ 두 화면 모두 "프로세스를 멈춘다"는 동일 위험 클래스이고
│        동일한 2단계 graceful→force 규약을 공유해야 하므로 한 지붕이 맞다.
│
├─ ☾  Keep Awake                                [사이드바 5]
│
├─ ✨ AI                                        [사이드바 6]  ← projects/usage/ai_control 3탭 → 1개
│   ├─ Usage
│   ├─ Projects
│   │   └─ Project Detail  (패널, 현행 유지)
│   ├─ Tool Adapters
│   └─ Control Center      ← 별도 사이드바 항목이 아니라 서브탭으로 강등
│
├─ 🕘 Activity                                  [사이드바 7]  ← 신규 (아래 기능 제안 #1)
│   ├─ Cleanup History     (삭제 저널 · 회수 실측 · Trash 링크)
│   └─ Skipped & Failed    (스킵 사유 집계)
│
└─ ⚙  Settings                                  (하단 고정)
    ├─ Appearance
    ├─ Navigation & Quick Panel
    ├─ Cleanup
    ├─ Providers
    ├─ Notifications & Automation
    └─ Diagnostics
```

**변경 요지:** 사이드바 11 → 8 entry, 중복 진입점 3 → 0, 최대 깊이 3단(사이드바 → 서브탭 → 상세) 유지. `settings.dashboard_tabs` 사용자 커스터마이즈는 그대로 존중하되(DESIGN.md preservation rule), `disk`/`usage`/`ai_control` 3개 레거시 값은 마이그레이션에서 부모 탭으로 접어라 — 지금은 `settings.svelte.ts:123`의 폴백 revision(3)이 백엔드(5)와 어긋나 마이그레이션이 되감기는 상태라 이 정리 전에 revision부터 맞춰야 한다.

**접힌 사이드바(64px) 보완:** 아이콘 중복(`HardDrive`가 storage·disk, `Sparkles`가 projects·ai_control) 문제는 위 통합으로 자연 해소된다. 남은 8개는 서로 다른 아이콘을 갖는다.

---

## 신규 기능 제안

| 기능 | 사용자 가치 | 구현 비용 | 안전 리스크 | 판정 | 근거 |
|---|---|---|---|---|---|
| **1. 삭제 저널 + Activity 뷰** (모든 파괴적 작업의 시각·경로·바이트·실측 여유증분·결과를 append-only 로그로 남기고 앱에서 조회) | 최상. 현재 `executor.rs`에 삭제 내역 영속 기록이 **전무**해서 "어제 뭘 지웠지?"에 답할 방법이 0이다. 사고 시 회수 수단이 없는 지금 상태(미서명 배포 + updater 부재 + 스쿼시 히스토리)에서 가장 값싼 신뢰 회복 장치 | M (백엔드 JSONL append + 회전, FE 뷰 1개) | **낮음 — 오히려 리스크 감소**. 단 경로 문자열을 남기므로 파일 권한 0600 강제 필요(현재 로그·설정 파일에 0600 지정 0건) | **채택 · 최우선** | `report.md` 데이터 차원 "삭제 내역 영속 기록 없음". 아래 3·4·6의 전제 조건이기도 함 |
| **2. Trash 경유 통일 + 앱 내 "휴지통 열기 / 되돌리기"** (캐시 클린·Docker prune 외 모든 삭제를 Trash로, 결과 카드에 복구 진입점) | 높음. Trash 워크플로 3종이 이미 존재하는데 **복구 버튼이 하나도 없다**. Models 삭제는 Trash 광고와 달리 영구삭제 | S (`show_in_file_manager` 재활용 + Models 경로를 기존 trash 커맨드로 이관) | **낮음**. Trash는 사용자가 최종 결정권을 갖는 표준 관용 | **채택** | 경로 9·10·11·12 매트릭스. 되돌리기 칸이 전부 비어 있음 |
| **3. 회수량 이중 표기 + 하드링크/SI 각주** (논리 삭제 크기 ↔ 실측 여유 증가분을 항상 쌍으로, 차이 10%↑ 시 설명) | 높음. 현재 큰 숫자가 항상 과대(hardlink·APFS clone·Docker SI 7.4%)라 사용자가 Finder와 대조하면 앱을 못 믿는다 | S (`actual_disk_free_delta` 이미 존재, 위계만 반전) | 없음 | **채택** | `CleanResultModal.svelte:88-99`, `docker/adapter.rs:283`, `scanner/size.rs:48` |
| **4. 스킵/불완전 사유 집계 뷰** (깊이 초과·권한 거부·심링크 회피·측정 실패를 사유별로 집계해 스캔 후에도 상시 노출) | 중상. 지금은 스캔 **중에만** 카운터가 보이고 사라진다. "왜 내 5GB `node_modules`가 안 나오지?"에 답이 없다 | S (`DeveloperArtifactsView.statusLabel()` 로직 재사용 + 집계 컴포넌트) | 없음 | **채택** | `DeveloperArtifactsView.svelte:440-445, 176-187` |
| **5. 프로젝트별 캐시 크기 뷰** (workspace 단위로 `node_modules`/`target`/`.venv`/DerivedData/Docker 레이어를 합산) | 중. `DeveloperArtifactsView`에 이미 workspace 개념·마커 탐지가 있어 절반은 구현됨. "이 레포가 12GB"는 개발자에게 가장 행동 가능한 단위 | M (기존 스캐너 결과를 project 키로 재집계 + 뷰) | 낮음 (읽기 전용 집계) | **채택** | `DeveloperArtifactsView.svelte:159-163, 528-534`에 ecosystem/project 축이 이미 있음 |
| **6. 명령 팔레트 + 전역 단축키** (`Cmd/Ctrl+K` 팔레트, `Cmd/Ctrl+1..9` 탭, Quick Panel 전역 핫키) | 중상. 개발자 유틸에서 키보드 부재는 기능 결손에 가깝다. 현재 앱 전체 키보드 핸들러가 Esc/Cmd+W 2개뿐 | M (`tauri-plugin-global-shortcut` 추가 + 팔레트 컴포넌트) | 낮음. 단 팔레트에서 파괴적 커맨드는 **실행이 아니라 해당 확인 화면으로 이동**만 하도록 제한할 것 | **채택** | `Cargo.toml:36`에 notification 플러그인만, `quickPanel.ts:28-30` |
| **7. 자동 업데이터** (`tauri-plugin-updater` + 서명 검증) | 중상. 지금은 사용자가 릴리스를 직접 받아 `xattr -cr`로 격리 속성을 벗겨야 한다. 위 P0 결함들을 고쳐도 **사용자에게 전달할 경로가 없다** | M (플러그인 + 서명 키 운영) | **중간**. 자체 생성 체크섬으로 자체 검증하는 현행 릴리스 파이프라인(`release.yml:156,285`) 위에 올리면 업데이트 채널이 새 공격면이 된다. 코드서명·공증 선행 필수 | **채택(조건부)** | `report.md` 공급망 항목 + updater 부재 |
| **8. 예약 정리 (Scheduled cleanup)** | 중. 반복 작업 자동화 | M | **높음**. 확인 다이얼로그 없는 자동 영구삭제. 현재 Safe 판정 자체가 `$TMPDIR` 직계 자식 **이름 접두사 + mtime**만 보고(`signatures/system.toml:3-38`), 사람이 만든 동명 디렉터리를 3일 방치하면 `delete_directory` 대상이 된다 | **보류** | 저널(#1) + Trash 경유(#2) + Safe 판정 강화가 모두 끝난 뒤 재검토 |
| **9. 회수 트렌드 차트** | 하~중. 보기 좋지만 행동을 바꾸지 않는다 | S (저널이 있다면) | 없음 | **보류** | #1의 부산물로 나중에. 단독 가치로는 정당화 안 됨 |
| **10. 앱 내 알림 센터(인박스)** | 하. OS 알림(`tauri-plugin-notification`)과 설정 섹션이 이미 있고, 데스크톱 유틸에 두 번째 알림 저장소는 중복 | M | 낮음 | **보류** | `SettingsView.svelte:593-686` Notifications 섹션이 이미 존재. Activity 뷰(#1)가 사실상 이 역할을 흡수 |
| **11. 시그니처 커스터마이즈 UI** (사용자가 삭제 대상 경로 규칙 추가) | 중 (파워유저 한정) | M | **매우 높음**. `registry.rs:55`의 `load_from_dir`는 현재 미사용 dead 확장점인데, UI로 살려내면 **임의 TOML → 삭제 루트**가 된다. 게다가 `models/signature.rs:8`에 `deny_unknown_fields`가 없어 `platforms` 오타 시 전 플랫폼 활성 | **기각** | 위험 대비 사용자 수 불균형. 필요하면 "제외(exclusion) 추가"만 허용하고 "포함 추가"는 금지 |
| **12. CLI / 헤드리스 모드** | 중 (CI 활용) | L | **높음**. 확인 UI가 없는 삭제 진입점을 새로 만드는 일. 지금 GUI에도 무확인 영구삭제 경로가 8개 있다 | **기각** | 먼저 GUI 8개 경로를 고쳐라 |
| **13. 데스크톱 위젯 / 메뉴바 상주 게이지** | 하. Quick Panel이 이미 그 역할이고, 상시 표시는 `metrics/memory.rs:354`의 `System::new_all()` 전체 스캔을 주기 실행하게 만든다 | M | 중 (배터리) | **기각** | Quick Panel과 기능 중복 + 전력 비용 |
| **14. 앱 내 버그리포트(진단 첨부)** | 중 | S | **중간**. `diagnostics/mod.rs:43-59` 리댁션이 Basic auth·`github_pat_`·`gho_`·`AKIA`·`AIza`·URL 자격증명을 놓치고, 실패 로그에 절대경로가 그대로 들어간다(README "absolute paths never cross" 반례) | **보류** | 리댁션 패턴 보강 후 채택 가능. 지금 켜면 사용자가 자기 시크릿을 이슈에 붙이게 된다 |

**착수 순서 권고:** #1(저널) → #2(Trash 통일 + 복구 진입점) → #3(회수량 표기) → #4(스킵 사유) → #5·#6 → #7(코드서명 완료 후)

---

## 총평

위험 작업 UX가 **정확히 거꾸로 배분**돼 있다. 이미 Trash로 복구 가능한 경로(Large Files·Applications·Developer Artifacts)에는 일회용 플랜·TTL 카운트다운·경로 전문 나열·부분측정 동의까지 3단계 안전장치가 있고, 실제로 되돌릴 수 없는 영구삭제 8개 경로(Quick Clean·Category Detail Clean·Docker prune 4종·Models 삭제·Volumes)는 확인이 0~1단계다.

앱 안에 이미 정답이 있다는 점이 이 프로젝트의 강점이자 문제다. `DevelopmentServersView`의 graceful→force 2단계 에스컬레이션, `DeveloperArtifactsView`의 부분측정 동의, `CleanupReviewDialog`의 네이티브 `<dialog>` 포커스 관리, `SegmentedTabs`의 roving tabindex — 네 개의 훌륭한 패턴이 각각 **한 곳에서만** 쓰이고 나머지 화면은 각자 다시 만들었다. 신규 설계보다 **기존 최고 패턴의 강제 적용**이 압도적으로 비용 대비 효과가 크다.

신뢰 신호는 숫자에서 무너진다. 회수량은 하드링크·APFS clone·Docker SI 환산으로 항상 과대이고, 정확한 값(`actual_disk_free_delta`)은 괄호 안 작은 글씨다. `formatBytes`는 1024로 나누고 SI 라벨을 붙여 Finder와 어긋난다. Partial 결과는 프론트가 `error_message` 유무로 사후 보정하고 있다. 삭제 내역은 어디에도 남지 않는다 — 사용자가 검증할 방법 자체가 없다.

접근성은 선언과 구현의 격차가 크다. `DESIGN.md`가 "WCAG AA 4.5:1 both themes"를 명시하지만 라이트 모드 시맨틱 3색이 전부 3.1~3.8:1이고, 그 색들이 10~12px 본문 텍스트에 쓰인다. 모달 6개 중 포커스 트랩이 있는 건 1개, 파괴적 삭제 확인창(Models)은 role·Esc·포커스 전부 없다. 문서에만 존재하는 온보딩 패키지, 사용처 0인 `.focus-ring`, import되지 않는 `StorageTools.svelte`처럼 **문서·토큰·코드가 서로 다른 시점의 진실**을 담고 있다.

1인 베타로서는 삭제 코어의 견고함이 이례적이므로, 다음 릴리스는 새 기능이 아니라 **영구삭제 8경로에 확인 붙이기 + 삭제 저널 + Trash 복구 진입점 + 라이트 모드 대비 수정** 네 가지에만 집중하는 편이 제품 신뢰도를 가장 크게 올린다.


---

# 부록 — 비판적사고 차원 원본 (improve-critic.md)

## Zenith 제품 전략 비평 — "무엇을 빼야 하는가"

대상: `jaeyoung0509/zenith` v0.3.8 (읽기전용 클론, shallow, 커밋 1개·저자 1명)
전제: 1차 종합 리뷰(`report.md`) 결론 — "삭제 코어는 견고, 그 바깥이 문서 약속 미달, 같은 일 하는 코드 4벌"

**실측 기준선**
- 코드 총 50,297줄 (Rust 30,425 / TS·Svelte 19,872), 그중 테스트 5,641줄(Rust 1,435 + FE 4,206)
- 문서 2,088줄 (README 278, ARCHITECTURE 496, SAFETY 370, DESIGN 186, AGENTS 143, 그 외)
- IPC 커맨드 59개 (`lib.rs:325-385`), 시그니처 TOML 49개(5파일), 대시보드 뷰 17개
- 저자 1명 / 커밋 1개 → **bus factor 1 확정**

---

## 영역별 판정표

줄수 = Rust 모듈 + 대응 Svelte 뷰 + store + models 타입을 합산한 실측 버킷값(`wc -l`). 공유 인프라(tooling·bindings·mock·trash_manager 등 ≈11,000줄)는 별도.

| # | 영역 | 줄수 | 핵심 가치 기여 | 대체재 | 안전 리스크 | 판정 | 근거 |
|---|---|---|---|---|---|---|---|
| 1 | **캐시 스캔/삭제 코어** (scanner+cleaner+safety+signatures+collection+cache_providers+StorageView 계열) | **6,745** | ★★★ 제품의 존재 이유. 49개 시그니처를 한 화면에 모은 건 대체재 없음 | CleanMyMac(유료·비개발자향), 각 툴 개별 명령 | 중 — 코어는 O_NOFOLLOW dirfd·dev/ino 재검증으로 견고하나 `blacklist.rs:119` 홈 하위 early-return 허용 | **유지 (전부 여기 집중)** | 6개 리뷰 차원이 유일하게 일치해 칭찬한 부분 |
| 2 | **Large Files** (`large_files/mod.rs` 664 + `LargeFilesView` 561) | **1,338** | ★★☆ 승인 폴더 한정 + Trash 이동. 코어와 같은 여정 | macOS 저장공간 관리, `du`, DaisyDisk | 저 — Trash 경유 복구 가능 | **유지** | trash_manager(1,037) 공유로 한계비용 낮음 |
| 3 | **App 언인스톨** (`applications` 544 + `ApplicationsView` 527) | **1,185** | ★★☆ 잔여 파일 회수, Trash 경유 | AppCleaner(무료·성숙) | 저 — Trash 경유 | **유지 (단, 차별화 없음 인정)** | 코어와 동일 삭제 엔진 재사용 |
| 4 | **Developer Artifacts** (`developer_artifacts/mod.rs` 1,979 + 뷰 590 + models 179) | **2,748** | ★★★ node_modules·target·DerivedData 프로젝트 단위 집계는 진짜 가치 | `cargo clean`/`go clean -cache` 개별 실행 | 중 — 빌드 중 삭제(`toctou.rs:132` 디렉터리 mtime 미검증) | **유지 + 축소** | `recognize_*` 함수 14벌(`:766~:1147`) — 테이블 주도로 접으면 ~600줄 절감 |
| 5 | **Docker prune** (`docker/adapter.rs` 554 + `DockerView` 321) | **997** | ★☆☆ 용량 표시는 유용, prune은 얇은 CLI 래퍼 | `docker system df` / `docker system prune` (완전 동일) | **고** — `:459` `image prune -a -f` 로컬 전용 이미지 영구 손실, `:483` `volume prune` 분기(TOML 한 줄 지뢰) | **축소 (읽기 전용화)** | GB를 1024³으로 계산해 Docker(SI) 대비 7.4% 과대 표시까지 함 |
| 6 | **로컬 모델 삭제** (`models_inventory` 446 + `ModelsView` 229) | **755** | ★★☆ Ollama/HF/LM Studio 통합 인벤토리는 가치 | `ollama rm`, `huggingface-cli delete-cache` | **고** — `deleter.rs:96-101` HF/LM Studio는 Trash 아닌 **영구 삭제**. 수십 GB 재다운로드 | **축소 (Trash 강제)** | 같은 앱 안에서 Large Files는 Trash, 모델은 영구 — 일관성 붕괴 |
| 7 | **메모리/프로세스 종료** (`metrics/memory` 524 + `MemoryView` 352 + store 115) | **1,110** | ★☆☆ 지표는 Activity Monitor 열등 복제, 종료는 위험 | Activity Monitor / 작업 관리자 (무료·서명됨·OS 제공) | **최고** — `commands/system.rs:29` 이름 문자열로 동명 프로세스 전부 SIGKILL. lease·uid·start_time·self-PID 검증 전무. 보호 목록에 iTerm2/Ghostty/kitty/WezTerm 없음 | **제거** | 제품 전체에서 lease 모델을 깨는 유일한 지점. 1차 리뷰 Critical 1번 |
| 8 | **개발서버 포트 해제** (`dev_ports` 2,693 + 뷰 307 + store 141) | **3,354** | ★★☆ lease + 2단계 force는 설계적으로 우수 | `lsof -ti:3000 \| xargs kill` (한 줄) | 중(mac) / **고**(Win) — `termination.rs:74` Windows uid 하드코딩 1000 → 소유자 필터 무조건 통과, `:189-215` graceful 부재 | **축소 (macOS 한정 유지, Windows 비활성)** | 3,354줄이 한 줄 셸의 안전판. 가치는 있으나 코어 대비 투자 과다 |
| 9 | **Keep Awake** (`power` 1,822 + `AwakeView` 593 + models 130) | **2,634** | ★☆☆ 규칙 기반 자동화는 니치 | `caffeinate -i` / `-d` (macOS 기본 탑재, 0줄), Amphetamine(무료·서명) | 중 — `watcher.rs:450` 규칙 활성 시 **5초마다 전체 프로세스 스캔**. 배터리 절약 도구가 배터리를 먹음. `:452` 무한 스레드 안 `.unwrap()` | **축소 (수동 타이머만 남기고 watcher 제거)** | `assertion.rs`(229)+`source.rs`(176)만 남기면 405줄. **2,229줄 삭제 가능** |
| 10 | **AI 사용량** (`ai_usage` 960 + `ai_snapshots` 511 + store/뷰 385) | **1,940** | ★★☆ 개발자 관심사 적중, 다만 코어와 여정 무관 | ccusage, `codex /status`, OpenRouter 대시보드 | **고** — `ai_usage/mod.rs:434-518` OAuth `state` 없음 + 콜백 무검증 → 180초 동안 임의 로컬 프로세스가 공격자 키 주입 | **분리 (별도 메뉴바 앱)** | "삭제 도구"에 OAuth 리스너를 붙일 이유가 없다. 취약점의 진입면이 삭제 코어와 무관하게 늘어남 |
| 11 | **Project Cockpit** (`agent_activity` 3,008 + 뷰 549 + store 157 + 컴포넌트 496 + models 230) | **4,440** | ★☆☆ **문서 자체가 전 어댑터를 "Process-only"로 표기** | `ps aux \| grep claude`, `git worktree list` | 중 — `events.rs:138` FE가 준 `cwd`로 `git -C` 실행 | **제거 또는 분리** | `PROJECT_COCKPIT.md`: 8개 어댑터 전부 Process-only, "No bundled adapter currently emits this evidence". `hooks.rs:install_integration`은 **항상 거부**("verified protocol-specific") — 223줄이 legacy 마커 삭제 전용. "Attention Counters(승인 대기·턴 완료)"는 UI에 있으나 이벤트 소스가 없어 **영구히 0** |
| 12 | **AI Control Center** (`ai_control_center` 2,907 + 뷰 158 + store 106 + models 415) | **3,586** | ★☆☆ 4개 하위 기능 전부 자문(advisory) | gitleaks/trufflehog(안전스캔), 각 벤더 대시보드(예산), `git diff`(diff) | **최고** — `git.rs:341-373` `--no-ext-diff`/`GIT_CONFIG_NOSYSTEM` 없음 → 공격자 `.git/config` 저장소에서 RCE. `safety.rs:156` `.env` 스캔 제외인데 `:407-432` untracked 전문 diff는 WebView로 반환 → **.env 평문 유출** | **제거** | `policy.rs:12` `evaluate()`는 `Vec<Recommendation>`만 반환. 문서 명시: "recommendations only", "consuming it does not perform the downstream mutation". **3,586줄로 알림 문구 5종을 만든다** |
| 13 | **설정** (`models/settings` 541 + `SettingsView` 798 + store 301 + store파일 147) | **1,799** | ★★☆ 필수 | — | 중 — `settings_store.rs:98` fsync 없음, `models/settings.rs:100` unknown 필드 미보존 → 다운그레이드 시 영구 소실 | **유지 + 축소** | 위 영역들을 잘라내면 설정 화면 798줄도 절반으로 |
| 14 | **디스크 뷰** (`metrics/disk` 109 + `DiskView` 157) | **266** | ★★☆ 저렴하고 맥락상 맞음 | Finder 정보 | 저 | **유지** | 비용 대비 최고 |
| 15 | **Quick Panel** (`QuickPanel.svelte` 543 + utils 79) | **622** | ★★☆ 메뉴바 접근성 | — | **최고** — `:178-184` `handleCleanSafe()`가 확인 없이 플랜 생성→실행 연속. `CleanupReviewDialog`("cannot be undone")는 **StorageView 한 곳에서만** 사용 | **축소 (삭제 권한 박탈)** | `quick.json`은 "read-mostly"라 선언하고 `allow-create-delete-plan`+`allow-execute-clean` 부여 — 선언과 권한이 모순 |

### 합계 판정
| 판정 | 영역 | 줄수 |
|---|---|---|
| 제거 | 메모리 종료(7·일부), Project Cockpit(11), AI Control Center(12) | **≈ 8,400** |
| 분리 | AI 사용량(10) | **≈ 1,940** |
| 축소 | Docker(5), 로컬모델(6), 포트(8), Keep Awake(9), Quick Panel(15), Dev Artifacts(4) | **≈ 3,500 절감 가능** |
| 유지 | 코어(1), Large Files(2), 언인스톨(3), 디스크(14), 설정(13) | ≈ 11,300 |

**결론: 50,297줄 중 13,800줄(27%)이 "삭제 도구"라는 정체성과 무관하고, 그 안에 Critical 취약점 3건(터미널 SIGKILL·OAuth 하이재킹·git RCE/.env 유출)이 몰려 있다.**

### 사용자 여정 검증
"디스크가 꽉 찼다 → 개발 캐시를 지운다"가 코어 여정이다.
- 여기 있는 것: 캐시 스캔, Large Files, 언인스톨, Dev Artifacts, Docker 용량, 로컬 모델, 디스크 뷰 — **일관됨**
- 여기 없는 것: Keep Awake(전원), AI 사용량(과금), Project Cockpit(관측), AI Control Center(거버넌스), 프로세스 종료(성능) — **다섯 개의 다른 제품**

---

## 기능 제안 반론

| 기능 | 넣으면 터지는 것 | 채택 전제조건 / 기각 |
|---|---|---|
| **자동 정리 스케줄러** | 확인 없는 삭제가 백그라운드로 옮겨간다. `toctou.rs:132`가 디렉터리 mtime을 안 보므로 야간 빌드 중 `target/`·`DerivedData` 삭제. Quick Clean(확인 없음)이 사람 없는 시간에 반복 실행되는 것과 동일 | **기각.** 최소 전제: 디렉터리 신선도 재검증 + 전 삭제 Trash 경유 + 삭제 이력 UI. 셋 다 없는 지금은 사고를 자동화 |
| **시그니처 커스터마이즈(사용자 TOML)** | `registry.rs:55` `load_from_dir`가 이미 dead 확장점으로 존재. 살아나는 순간 임의 TOML이 삭제 루트가 되고, `models/signature.rs:8`에 `deny_unknown_fields`가 없어 `platforms` 오타 하나로 전 플랫폼 활성. `registry.rs:81` id 중복은 조용히 덮어씀 | **기각.** 이건 "커스터마이즈"가 아니라 "임의 경로 삭제 커맨드"의 우회로. 제품이 "arbitrary path deletion command를 노출하지 않는다"고 약속한 것을 정면으로 뒤집는다 |
| **자동 업데이트(updater)** | **미서명 배포에서 업데이터 = 서명 없는 코드 실행 채널.** `release.yml:288-291`이 자기가 만든 체크섬을 자기가 검증하고, 액션은 전부 가변 태그(`@v2`), `pnpm install` lifecycle script 허용. 릴리스 파이프라인이 뚫리면 전 사용자에게 배포 | **조건부.** ① SignPath 서명 완료 ② Tauri updater 서명키를 릴리스 워크플로 밖(HSM/OIDC)에 보관 ③ 액션 SHA 핀 고정 ④ `pnpm install --ignore-scripts`. 넷 다 끝나기 전엔 수동 다운로드가 더 안전 |
| **클라우드 동기화** | README:100 "100% Local: Zero telemetry, zero cloud tracking"이 즉시 거짓이 된다. 동기화 대상이 설정이라도 시그니처 선택·승인 폴더 경로가 나가면 로컬 디렉터리 구조 유출 | **기각.** 이 제품의 유일한 마케팅 차별점이 "로컬"이다. 1인 개발자가 동기화 백엔드를 운영·보안 유지할 수 없다 |
| **텔레메트리(익명 사용 통계 포함)** | 위와 동일 + 삭제 도구가 파일 경로 통계를 보낸다는 인상. 크래시 리포트조차 `executor.rs:52-56`이 실패 시 **절대경로를 로그에 남기므로** 자동 전송하면 경로가 그대로 나감 | **조건부.** 로컬 로그의 절대경로 제거 + opt-in 명시 동의 + 전송 payload 전문 공개. 그래도 ROI가 없다 — 1인이 대시보드를 볼 시간이 없다 |
| **플러그인 시스템** | 삭제 권한을 3자 코드에 위임. `tooling.rs:37-41`이 상속 PATH를 우선하고 실행파일 검증이 없어 플러그인이 PATH 하이재킹으로 임의 바이너리 실행. bus factor 1이 플러그인 API 하위호환까지 짊어짐 | **기각.** 영구히 |
| **CLI 인터페이스** | GUI의 확인 다이얼로그·리뷰 화면을 전부 우회하는 경로가 생긴다. `--yes` 플래그가 나오는 순간 코어의 "리뷰 후 삭제" 보증이 무의미. 스크립트/CI에서 호출되면 사고가 배수로 | **조건부.** 읽기 전용(`zenith scan --json`)만. 삭제 서브커맨드는 금지 |
| **알림(네이티브 노티) 확장** | 이미 `agent_activity/notifications.rs`·`ai_control_center/notifications.rs` 2벌이 존재하고 둘 다 이벤트 소스가 없어 사실상 휴면. 확장하면 3벌째 | **기각.** 먼저 기존 2벌을 1벌로 합치고, 소스 없는 알림 종류(turn_completed·approval)를 UI에서 제거 |
| **추천 엔진 / autopilot 실행화** | 현재 `policy.rs:12`는 자문만 한다. 실행 권한을 주면 3,586줄의 미검증 판정 로직이 **삭제·프로세스 종료 트리거**가 된다. 그 판정 근거는 `confidence: "process_observed"` — exe basename 하나 | **기각.** autopilot을 실행형으로 만드는 것은 이 제품이 할 수 있는 가장 위험한 변경 |
| **다른 사용자 프로세스 관리** | `dev_ports/termination.rs:74`가 Windows uid를 1000으로 하드코딩해 이미 타사용자·SYSTEM 리스너가 후보에 들어온다. 이를 공식 기능화하면 관리자 권한 요구 → NSIS 사용자 설치(`%LOCALAPPDATA%`, 관리자 불필요) 전제 붕괴 | **기각.** 오히려 현재의 우발적 노출을 SID 대조로 막아야 함 |
| **다국어(i18n)** | `cleaner/executor.rs:336` 실패 분류가 **영어 문자열 매칭**이라 비영어 로케일에서 이미 전부 Unknown. UI만 번역하면 오류 분류가 조용히 깨진 채 확산 | **조건부.** 먼저 실패 분류를 `io::ErrorKind` 기반 타입으로 교체 |
| **삭제 되돌리기(Undo)** | `executor.rs:81`에 삭제 내역 영속 기록 자체가 없어 되돌릴 대상을 모른다. 구현하려면 삭제 전 백업 = 회수하려던 용량을 다시 점유 | **조건부.** "Undo"가 아니라 "전부 Trash 경유"로 대체. Trash가 곧 OS가 제공하는 undo |
| **Homebrew Cask / winget 확대** | 미서명 상태로 패키지 매니저에 올리면 `xattr -cr` 안내(README:79)를 사용자 수천 명이 습관화한다. Gatekeeper 무력화 습관을 퍼뜨리는 것 | **조건부.** 서명·공증 완료 후에만 |
| **멀티 프로파일 / 작업공간** | `settings_store.rs:98` fsync 없음 + 두 WebView last-write-wins 상태에서 프로필 수를 늘리면 설정 소실 확률이 곱해진다 | **기각** |

---

## UX 개선 부작용

| 개선안 | 부작용 | 붙여야 할 조건 |
|---|---|---|
| **모든 삭제에 확인 다이얼로그 추가** | 확인 피로 → 습관적 OK. 지금 Quick Clean은 확인이 0회라 오히려 "실수했다"는 자각이 남지만, 확인 5회가 되면 5회 모두 무의식적으로 통과. 실제 위험(로컬 모델 영구삭제·`prune -a`)이 저위험(temp 캐시) 확인에 묻힘 | **확인은 위험 등급별로 차등.** Safe=확인 없음 유지, Rebuild=요약 1회, **영구삭제/복구불가만 문구 입력형 확인**. 확인 횟수 총량을 늘리지 말고 재배치 |
| **전 삭제를 Trash 경유로** | "지웠는데 용량이 안 늘었다" 클레임. 50GB 캐시가 Trash로 가면 회수량 0. 게다가 macOS Trash는 볼륨별이라 외장 디스크는 이동 자체가 복사가 됨 | **Trash 경유 + 결과 화면에 "Trash 비우기" 1클릭 + 회수 예정/실회수 2값 표시.** 볼륨 다르면 Trash 불가로 표기하고 영구삭제 확인 승격 |
| **삭제 히스토리 UI** | 경로를 저장하는 순간 README:100 "100% Local"은 유지되지만 `~/Library/Logs/Zenith/`에 사용자 디렉터리 지도가 평문으로 쌓인다. 현재 로그·설정·감사 파일에 **0600 지정이 한 곳도 없다** — 같은 머신의 다른 프로세스가 읽음 | **경로는 해시 또는 시그니처 ID+상대경로만 저장. 파일 0600 강제. 보존기간 기본 7일.** 절대경로 저장은 금지 |
| **진행률 + 취소 버튼** | 현재 `execute_clean`에 취소 커맨드가 없고(`tree_deleter.rs:407-448`) 전역 `operation_gate`를 쥔 채 단일 스레드로 돈다. 취소 UI만 붙이면 "반쯤 지운 캐시"를 사용자가 **의도적으로** 만들게 됨 | **취소는 "다음 항목 경계에서 중단"으로 한정하고, 중단 시 "N개 완료 / M개 미처리"를 결과 화면에 명시.** 항목 중간 취소는 제공 금지 |
| **드라이런 / 상세 미리보기 화면** | 플랜 생성 → 검토 → 실행 사이 시간이 늘어난다. TOCTOU 창이 그만큼 벌어지고, `toctou.rs:132`가 디렉터리 mtime을 안 보므로 미리보기를 오래 볼수록 위험. 플랜 TTL(300초) 만료로 "실행 버튼이 안 먹는다" 문의 발생 | **미리보기 진입 시 TTL 리셋 대신 카운트다운 표시(`ScanFreshnessNotice` 재사용). 실행 직전 전 타깃 mtime 재확인 필수** |
| **자동 선택 범위 확대(Rebuild 기본 선택)** | `Rebuild`는 재다운로드·재빌드 비용을 뜻한다. 자동 선택하면 종량제 회선·오프라인 환경에서 수 GB 재다운로드. README가 "Only safe items are selected automatically"라고 광고한 것과 충돌 | **기각.** 대신 항목별 "재빌드 예상 비용(다운로드 N GB / 빌드 시간)" 라벨 추가가 정답 |
| **다크모드·애니메이션·디자인 폴리시** | 이미 `designSystem.test.ts`·`uiPolish.test.ts`·`redesign-controls.test.ts`가 **소스 텍스트를 문자열 매칭**한다(`expect(source).toContain('RotateCw size={11}')`). 스타일을 손댈 때마다 회귀와 무관한 테스트가 깨져 1인 개발자의 시간을 먹음 | **소스 텍스트 grep 테스트를 먼저 제거/전환.** 그 전엔 UI 폴리시 금지 |
| **Windows 기능 동등화** | 동등화는 "macOS에서 검증된 위험한 기능을 미검증 플랫폼으로 복제"다. Windows 안전 테스트가 0건(`tests/safety_tests.rs:1` `#![cfg(unix)]`)인 상태에서 기능만 맞추면 검증 격차가 벌어짐 | **순서 역전: 테스트 이식 → uid를 SID로 → graceful 종료 → 그 다음 기능.** 그때까지 Windows는 읽기 전용 모드로 출시 |

---

## README 문장 수정 제안

### 1) 프로세스 종료 (README:137)
**현재**
> Zenith never exposes an arbitrary PID-kill command. Application Quit actions resolve a fresh allowlisted app group in Rust, while Development Servers uses a separate endpoint-level workflow:

**문제** — `commands/system.rs:29` `terminate_process_group(name, force)`는 PID를 받지 않는 대신 **이름 문자열**을 받아 프로세스 테이블 전체에서 일치하는 것 **전부**에 신호를 보낸다. lease·uid·start_time·self-PID 검증이 없고 `force=true`면 즉시 SIGKILL이다. "arbitrary PID kill이 없다"는 문장은 기술적으로 참이지만 실제로는 **PID kill보다 넓은** 권한을 감춘다. 보호 목록(`memory.rs:275-304`)에 iTerm2·Ghostty·Alacritty·kitty·WezTerm·Warp가 없어 터미널을 SIGKILL하면 하위 셸과 에이전트가 연쇄로 죽는다.

**제안**
> Development Servers releases exactly one verified TCP listener through a short-lived one-shot lease. Application Quit is a different, weaker mechanism: it matches installed applications by executable name and may signal more than one process with that name. Terminal emulators other than Terminal.app are not currently in the protected list — quit them from the Dock or Activity Monitor instead.

### 2) 시크릿 리댁션 (README:101)
**현재**
> **Secret Redaction**: Subprocess errors and diagnostic messages automatically redact sensitive API keys (`sk-...`, tokens, passwords) before writing to disk.

**문제** — `diagnostics/mod.rs:43-59`가 탐지하지 못하는 것: GitHub PAT(`github_pat_`, `gho_`), AWS 키(`AKIA`), Google 키(`AIza`), HTTP Basic 자격증명, URL 내장 자격증명(`https://user:pass@host`). 특수문자 포함 비밀번호는 부분만 가려진다. "automatically redact"라는 단정은 미탐지 항목이 있을 때 사용자를 무방비로 만든다 — 진단 파일을 GitHub 이슈에 붙이도록 유도하는 문구이기 때문에 더 위험하다.

**제안**
> **Best-effort redaction**: Diagnostic output masks common OpenAI-style keys (`sk-...`) and password-like fields before writing to disk. Coverage is not exhaustive — GitHub, AWS, and Google credential formats are not yet detected. Review the exported file before sharing it.

### 3) Windows 검증 (README:66-75 + `docs/WINDOWS.md:106-108`)
**현재 (README)**
> Confirm that the installer came from this repository's GitHub Release and verify its SHA256 value against `SHA256SUMS.txt` before choosing **More info → Run anyway**.

**현재 (WINDOWS.md:106)**
> `.github/workflows/ci.yml` runs the same checks on `windows-latest`

**문제** — 두 가지가 동시에 무너진다. ① `SHA256SUMS.txt`는 릴리스 워크플로가 **직접 생성하고 직접 검증**한다(`release.yml:288-291`). 배포 채널이 곧 체크섬 채널이므로 이 대조는 변조를 막지 못하고 전송 오류만 잡는다 — 사용자에게는 변조 방지처럼 읽힌다. ② `tests/safety_tests.rs:1`·`tests/dev_ports_tests.rs:1`이 `#![cfg(unix)]`라 **Windows에서 25개 삭제-안전 통합 테스트가 한 건도 컴파일되지 않는다.** `cargo test`는 초록으로 통과하고 NSIS가 배포된다. "same checks"는 거짓이다.

**제안 (README)**
> The published SHA256 values detect download corruption. They are generated by the same release pipeline that builds the installer, so they are **not** evidence that the build was not tampered with; that guarantee arrives with code signing.

**제안 (WINDOWS.md)**
> `windows-latest` runs the compile and packaging gate. The deletion-safety and development-port integration suites are Unix-only today, so Windows-specific path handling (verbatim, UNC, alternate data streams) has no automated coverage. Treat the Windows build as an early preview.

---

## 빼기 3 / 넣기 3

### 빼기

**1. AI Control Center 전체 (3,586줄)**
`policy.rs:12`의 `evaluate()`는 `Vec<Recommendation>`만 반환하고, 문서가 스스로 "recommendations only", "consuming it does not perform the downstream mutation"이라 못박는다. 즉 3,586줄이 만들어내는 최종 산출물은 알림 문구 5종이다. 반대편에 붙은 비용은 이 제품 최악의 취약점 조합 — `git.rs:341-373`이 `--no-ext-diff`/`GIT_CONFIG_NOSYSTEM` 없이 `git -C <FE가 준 cwd>`를 실행하고(악성 `.git/config` 저장소에서 RCE), `:407-432`가 untracked 파일 전문을 WebView로 반환하는데 `safety.rs:156`은 `.env`를 스캔에서 제외한다(= 보호 대상이라 인정한 파일을 diff로는 그대로 노출).

**2. Project Cockpit의 어댑터·훅·이벤트 계층 (agent_activity 3,008줄 중 hooks 223 + events 241 + notifications 206 + adapters 561 ≈ 1,231줄, 잔여도 재검토)**
`hooks.rs:install_integration`은 **모든 도구에 대해 거부**를 반환한다("verified protocol-specific"). `PROJECT_COCKPIT.md` 어댑터 표는 8행 전부 "Process-only / No verified bridge"이고, "Vendor confirmed"에 대해 "No bundled adapter currently emits this evidence"라 명시한다. UI가 광고하는 Attention Counter(승인 대기·턴 완료)는 이벤트 소스가 없어 **영구히 0을 표시**한다. 남은 실질 기능은 `ps`로 CLI 프로세스를 찾아 git root로 묶는 것이며, 그 대가로 `post_agent_event(cwd)` → `git -C` 라는 IPC 진입면을 열어 둔다.

**3. Keep Awake 규칙 워처 (`power/watcher.rs` 1,107줄 + 관련 ≈ 2,229줄)**
`watcher.rs:450`이 규칙 활성 시 5초마다 전체 프로세스 테이블을 스캔한다 — 전력 관리 기능이 전력을 쓴다. macOS는 `caffeinate`를 기본 탑재하고 Amphetamine이 무료·서명 상태로 존재한다. `assertion.rs`(229) + `source.rs`(176) = 405줄만 남겨 수동 타이머로 축소하면 기능의 90%를 5% 코드로 유지한다. `:452` 무한 스레드 안의 `.unwrap()`도 함께 사라진다.

### 넣기 (안전·신뢰만)

**1. 영구 삭제 경로 단일화 — 전 경로 `CleanupReviewDialog` 강제 + Quick Panel 삭제 권한 박탈**
현재 `CleanupReviewDialog`("cannot be undone")는 `StorageView.svelte:363` 한 곳에서만 쓰이고, `QuickPanel.svelte:178-184`의 `handleCleanSafe()`는 플랜 생성과 실행을 **확인 없이 연속 호출**한다. `quick.json`은 스스로를 "read-mostly"라 선언해 놓고 `allow-create-delete-plan`·`allow-execute-clean`을 부여한다. always-on-top 투명 창이 메인 창과 동일한 삭제 권한을 갖는 구성은, 사용자가 메뉴바를 잘못 클릭하면 되돌릴 수 없다. 두 permission을 제거하고 Quick Panel의 Clean은 메인 창을 여는 딥링크로 바꾸면 코드가 **줄어들면서** 최악의 사고 경로가 닫힌다.

**2. 삭제 = Trash 원칙의 예외 제거 (로컬 모델 + 실회수량 2값 표시)**
`models_inventory/deleter.rs:96-101`만 HF/LM Studio 모델을 Trash가 아닌 영구 삭제로 처리한다. Large Files와 App Uninstaller는 Trash를 쓰는데 **가장 비싼 자산(수십 GB, 재다운로드 필요)**만 영구 삭제인 것은 정확히 거꾸로다. 같이 넣어야 할 것은 결과 화면의 "Trash로 이동 N GB / 실제 회수 M GB" 2값 — 위 UX 표에서 지적한 "안 지워졌다" 클레임을 미리 막는다.

**3. Windows 안전 테스트 이식 + 그전까지 Windows 읽기 전용 모드**
`tests/safety_tests.rs:1`의 `#![cfg(unix)]` 하나 때문에 Windows에서 삭제-안전 검증이 0건이고, `blacklist.rs:225-311`의 Windows 전용 verbatim/UNC/ADS 로직은 자동 테스트를 한 번도 통과한 적이 없다. 여기에 `dev_ports/termination.rs:74`의 uid 하드코딩 1000이 겹쳐 소유자 필터가 무조건 통과한다. 픽스처 이식이 끝날 때까지 Windows 빌드에서 삭제·종료 커맨드를 capability 수준에서 비활성화하고 스캔/표시만 제공하면, 문서의 거짓 보증을 코드로 해소할 수 있다.

---

## 총평

Zenith의 삭제 코어(6,745줄)는 1인 프로젝트로는 이례적으로 견고하지만, 그 주위에 **다섯 개의 다른 제품**(전원 관리·과금 추적·에이전트 관측·AI 거버넌스·프로세스 관리)이 붙어 총 50,297줄이 됐고, Critical 취약점 3건이 전부 그 바깥에 있다.
가장 큰 문제는 코드량이 아니라 **약속의 인플레이션**이다 — Project Cockpit은 문서 스스로 전 어댑터가 Process-only라 적고, AI Control Center는 자문만 하며, `setup_agent_integration`은 항상 거부를 반환한다. 미구현을 "advisory"·"truthful evidence model"로 포장한 3,586+3,008줄이 유지 부담과 공격면을 동시에 만든다.
테스트 5,641줄은 숫자만 크다. FE 690개 `expect()` 중 상당수가 `svelte/server` SSR 문자열이나 **소스 텍스트 grep**(`toContain('RotateCw size={11}')`)이고, Rust 298개 테스트는 `#![cfg(unix)]`로 Windows에서 통째로 빠진다 — 회귀를 잡는 게 아니라 리팩터를 막는다.
신뢰 포지셔닝에서는 "never / always" 단정을 줄이고 능력 경계를 명시하는 쪽이 오히려 신뢰를 올린다. 미서명 + updater 부재 + 단일 저자 + 1커밋 스쿼시 히스토리 조합에서, 사고가 나면 회수 수단이 존재하지 않기 때문이다.
**권고: 13,800줄(27%)을 잘라내고 남은 예산 전부를 코어의 fail-open 3곳·Windows 테스트 이식·Quick Panel 확인 경로에 투입하라. 새 기능은 이 세 가지가 끝나기 전까지 전부 기각한다.**


---

# 부록 — 아키텍처 차원 원본 (improve-architecture.md)

## Zenith 개선안 설계 리뷰 — UI/UX 개선 + 신규 기능 수용력

대상: `zenith` (Tauri 2 + Rust ~30k / Svelte 5 ~16k), 읽기 전용
전제: 1차 리뷰(`architecture.md`, `report.md`) 결과를 기준선으로 사용. 여기서는 "지금 구조가 후보 기능 ①~⑩을 버티는가"만 본다.

---

## 0. 현 파괴 작업 경로 열거 (분석 항목 1의 전제)

| # | 경로 | plan | preview | confirm | execute | journal |
|---|---|---|---|---|---|---|
| P1 | generic clean | `DeletePlan` (models/plan.rs:31, TTL 300s, one-shot) — `create_delete_plan` commands/cleanup.rs:60 | `PlanPreview` (plan.rs:51) | FE `CleanupReviewDialog` — **StorageView.svelte:363 단 1곳** | `CleanExecutor::execute` cleaner/executor.rs:16 ← `execute_clean` cleanup.rs:114 | 없음 |
| P2 | trash plan (large files) | `TrashPlan` (trash_manager/mod.rs:47) — `prepare_large_file_trash` storage_commands.rs:351 | `TrashPlanPreview` (trash_manager/mod.rs:55) | LargeFilesView 인라인 | `TrashExecutor::execute` trash_manager/mod.rs:236 ← storage_commands.rs:466 | 없음 |
| P3 | trash plan (app uninstall) | 동상 — storage_commands.rs:434 | 동상 | ApplicationsView 인라인 | 동상 | 없음 |
| P4 | trash plan (dev artifacts) | 동상 — storage_commands.rs:318 | 동상 | DeveloperArtifactsView.svelte:65 `partialCleanupConfirmed` + :477 | 동상 | 없음 |
| P5 | docker prune | **없음** — `signature_id` 직행 (commands/system.rs:90) | **없음** | DockerView.svelte:302 (volume prune만) | `DockerAdapter::prune_category` docker/adapter.rs:436 | 없음 |
| P6 | model delete | **없음** — `model_id` 직행 (commands/system.rs:110) | **없음** | ModelsView.svelte:193 모달 | `LocalModelManager::delete_by_id` models_inventory/deleter.rs:11 | 없음 |
| P7 | terminate process group | **없음** — `(name, force)` 직행 (commands/system.rs:29) | **없음** | **없음** (memory.svelte.ts:83) | `MemoryInspector::terminate_group` metrics/memory.rs:353 | 없음 |
| P8 | release listener | lease (dev_ports/termination.rs:439, one-shot + `force_authorized` 2단계) | 리스트 항목 자체 | `mode` 파라미터 | `release_listener` dev_ports/termination.rs:483 | 없음 |
| P9 | stop agent session | lease (agent_activity/termination.rs:25) | 없음 | 없음 | `execute_graceful_stop` agent_activity/termination.rs:186 | AuditStore 부분 기록 |

구조적으로 **세 가지 서로 다른 모양**이다.
- **A형 plan 기반**(P1~P4): plan+preview+one-shot 소비까지 있고 저널만 없다.
- **B형 lease 기반**(P8·P9): lease가 plan 역할을 하고 preview가 없다. 대신 시그널 직전 재검증(uid/start_time/exe: dev_ports/termination.rs:553, agent_activity/termination.rs:201-224)이 A형보다 강하다.
- **C형 raw**(P5·P6·P7): plan·preview·저널 전부 없고 FE가 넘긴 식별자가 곧 실행 인자다.

⇒ **①(공통 확인/미리보기)의 진짜 비용은 다이얼로그가 아니라 C형 3개에 plan 단계를 새로 만드는 것**이다. 다이얼로그만 붙이면 "확인은 했는데 확인한 내용과 실행되는 내용이 다른" 구조(FE가 실행 인자를 계속 소유)가 남는다.

---

## 선행 구조 변경(순서)

1. **`StorageOperationGate` poison 복구 + "취소 커맨드는 게이트를 잡지 않는다" 규칙의 명문화·테스트**. 지금 취소가 동작하는 이유는 `cancel_large_file_scan`(storage_commands.rs:340)·`cancel_developer_artifact_scan`(:304)이 우연히 동기 `pub fn`이고 `storage_operation_gate`를 참조하지 않기 때문이다. 취소 대상 작업이 게이트를 쥔 채(cleanup.rs:31,124 / storage_commands.rs:171,263) 수 분간 돌므로, 누가 취소 핸들러를 `run_blocking`으로 감싸는 순간 영구 교착이다. 또 게이트 poison(operation_gate.rs:11 `.expect`)이 나면 아래 모든 작업이 무의미해진다. 영향: `src-tauri/src/operation_gate.rs:11`, `storage_commands.rs:304,340`. **비용 S**. 막히는 기능: ③⑤(사실상 전부).
2. **`PlanVault<P>` 추출 — TTL + cap 64 + one-shot take 로직 통합**. 완전히 동일한 로직이 `commands/cleanup.rs:94-104`(DeletePlan)와 `storage_commands.rs:73-83`(TrashPlan)에 2벌 있고, 보관 위치도 `AppState.delete_plans`(commands/state.rs:21)와 `StorageWorkflowState.trash_plans`(storage_commands.rs:41)로 갈려 있다. ①②④⑤가 전부 "plan을 하나 더 만든다"이므로 3벌·4벌로 늘어난다. 영향: `commands/cleanup.rs`, `storage_commands.rs`, `commands/state.rs`, 신규 `plan_vault.rs`. **비용 M**. 막히는 기능: ①②④⑤.
3. **`CancelRegistry` 추출 + `ScanEngine`/`CleanExecutor`에 취소 토큰**. 현재 `StorageWorkflowState`가 같은 맵을 2벌(storage_commands.rs:36,38) 들고 메서드 6개(:86-108)·자유함수 3개(:111-152)를 복제한다. `(OpKind, id)` 단일 키로 합치면 워크플로 추가 비용이 0이 된다. `LargeFileScanner::scan(..., cancel: Arc<AtomicBool>, ...)`(large_files/mod.rs:115-117, 체크 :140,:167)와 `DeveloperArtifactScanner::scan_workspaces`(developer_artifacts/mod.rs:150-152, 체크 :174,:223, rayon 워커까지 :245)가 이미 검증된 패턴이므로 그대로 승격. 영향: `storage_commands.rs:34-152`, `scanner/engine.rs:15`, `cleaner/executor.rs:36`. **비용 M**. 막히는 기능: ③⑤.
4. **`DeletionJournal` sink 트레이트 + JSONL 스토어 신규**. 상세는 아래 §저널. 영향: 신규 `journal/`, `cleaner/executor.rs:11,16`(구조체화), `trash_manager/mod.rs:233`, `lib.rs:205`(부트스트랩 주입). **비용 M**. 막히는 기능: ②(그리고 ⑤⑧의 기록 기반).
5. **`storage_commands.rs` → `commands/storage.rs` 이동 + `StorageWorkflowState`를 도메인 모듈로 하강(순환 해소)**. `storage_commands.rs:2`가 `commands::AppState`를 import하고 `commands/state.rs:23`이 다시 `StorageWorkflowState`를 필드로 갖는다. 위 2·3번이 만드는 신규 공용 레지스트리(PlanVault/CancelRegistry)를 **어느 쪽에 둘지 결정할 수 없게 만드는 원인**이 이 순환이다. 영향: `storage_commands.rs`, `commands/mod.rs`, `commands/state.rs:23`, `lib.rs:325` 명단. **비용 M**. 막히는 기능: ①③⑨.
6. **`ProcessLifecycleProvider` 도입 + 메모리 그룹 lease화**. 상세는 §Windows 패리티. `platform/mod.rs:13` 주석이 예고만 하고 끝낸 그 provider다. ⑩과 P7 lease화는 같은 작업이므로 분리하면 uid 판정이 5번째로 복제된다. 영향: 신규 `platform/process.rs`, `metrics/memory.rs:351-372`, `dev_ports/termination.rs`, `agent_activity/termination.rs:236`, `commands/system.rs:29`, `commands/state.rs`, `capabilities/main.json`. **비용 L**. 막히는 기능: ①⑩.
7. **`Supervisor` + 종료 훅**. `lib.rs:288-292`(awake watcher)·`:294-303`(ai_control tick) 두 detach 스레드에 join 핸들도 종료 플래그도 없고, `lib.rs:310`의 `run()` 콜백은 `Ready`/`Reopen`만 처리해 **`ExitRequested` 훅 자체가 없다**. 예약 정리(⑤)가 `execute_clean` 도중 `app.exit(0)`(lib.rs:263)로 죽으면 게이트를 쥔 채 반쯤 지운 트리 + 저널 누락이 남는다. 영향: `lib.rs:263,288-310`, 신규 `supervisor.rs`. **비용 M**. 막히는 기능: ⑤⑥⑧.
8. **settings revision 정합 수정 후 revision 6**. 백엔드 기본값 5(models/settings.rs:226)인데 FE 폴백이 `fetched.dashboard_tabs_revision ?? 3`(src/lib/stores/settings.svelte.ts:123)이다. 탭 재편(⑨)은 마이그레이션 6을 추가하는 일인데, 되감김 버그가 남은 채로 6을 얹으면 재편이 두 번 돌거나 아예 안 돈다. 영향: `src/lib/stores/settings.svelte.ts:123`, `models/settings.rs:258-358,407`. **비용 S**. 막히는 기능: ⑨.
9. **스토리지 3뷰 스토어 추출**. LargeFilesView(561줄)·ApplicationsView(527줄)·DeveloperArtifactsView(590줄)만 스토어 없이 스캔·취소·선택·플랜·실행 상태머신을 인라인으로 들고 있다. ①③②(공통 다이얼로그·취소·저널 뷰)는 전부 이 상태머신을 건드리므로 같은 수정을 세 파일에 3벌 하게 된다. 영향: `src/routes/dashboard/{LargeFiles,Applications,DeveloperArtifacts}View.svelte`, 신규 `src/lib/stores/*.svelte.ts` 3개. **비용 L**. 막히는 기능: ①③⑨.
10. **exclusions 매처 통일**. 크기 측정(scanner/size.rs:194 `contains`)과 삭제(safety/tree_deleter.rs:450 exact/prefix) 규칙이 다르다(1차 리뷰 확인). ⑦이 사용자에게 exclusions를 여는 순간 "제외했는데 지워졌다"를 그대로 출하한다. 영향: `scanner/size.rs`, `safety/tree_deleter.rs`, `scanner/walker.rs`. **비용 S**. 막히는 기능: ⑦.

---

## 기능별 구조 적합도

| 기능 | 현 구조 수용 여부 | 필요 변경 | 위험 | 권장 시점 |
|---|---|---|---|---|
| ① 공통 확인/미리보기 다이얼로그 | **부분** — A형(P1~P4)은 preview 계약이 이미 있음. C형(P5·P6·P7)은 plan/preview 자체가 없어 다이얼로그를 붙여도 "확인한 것 ≠ 실행되는 것" | C형 3개에 plan+preview 단계 신설(선행 2·6). FE는 `CleanupReviewDialog.svelte:9`의 `ScanItem[]` 하드코딩을 `{id,name,bytes,risk?}` 계약으로 일반화(:35-37 문구도 props로) | quick.json이 `allow-create-delete-plan`+`allow-execute-clean`을 보유 → **Svelte 다이얼로그로는 강제 불가**. QuickPanel.svelte:178-184는 plan→execute 연속 호출로 다이얼로그를 아예 통과하지 않음. 강제는 Rust에서 | 선행 2 직후 (1차) |
| ② 삭제 저널 | **미수용** — 기록 지점이 없음. `CleanResult`는 executor.rs:80-88에서 조립돼 cleanup.rs:148에서 채널로 나가고 소멸 | 선행 4 (sink 트레이트 + JSONL). 실행기 2개를 free static → 필드 보유 구조체로 | 경로 평문 저장 시 프라이버시 후퇴(현재 로그는 `sanitize_log` 경유, diagnostics/mod.rs:91). 파일 퍼미션 0600 지정이 저장소 전체에 0건 | 선행 4 (1차) |
| ③ 스캔/클린 취소 | **부분** — large_files/developer_artifacts만 보유. `ScanEngine::scan`(scanner/engine.rs:15)·`CleanExecutor::execute`(cleaner/executor.rs:16)·`SafeTreeDeleter`(tree_deleter.rs:41,99)는 토큰 없음 | 선행 1·3 | 게이트 교착(§선행1). 클린 취소는 **타깃 경계에서만** 허용해야 하며(executor.rs:36 루프), 트리 중간 중단은 반쯤 지운 디렉터리. 게다가 executor.rs:319-333이 Partial을 `success:true`로 보고 중 → 이걸 먼저 고치지 않으면 취소가 "성공"으로 기록됨 | 선행 1·3 직후 (1차) |
| ④ Trash 경유 옵션 | **부분** — Trash 쪽 sink 주입은 이미 있음(`TrashExecutor::execute_with(plan, F)` trash_manager/mod.rs:242, 테스트 :842가 실제로 주입). 반대로 `SafeTreeDeleter`엔 seam 없음 | §Trash 참조. `clean_target`(executor.rs:94) 수준의 per-target sink + `CleanItemResult`에 `sink` 필드 | `delete_contents`(:41)는 자식 N개 삭제 → Trash로 바꾸면 Trash 최상위에 N개 항목. TOCTOU dirfd 검증(tree_deleter.rs:159,210,231)이 `trash::delete` 경로에선 소멸. `total_reclaimed_bytes`가 0이 되어 디스크 델타 UI가 깨짐 | ②·⑨ 이후 (2차). 단 P6(models_inventory/deleter.rs:96 영구삭제)만은 지금 고칠 것 |
| ⑤ 예약/자동 정리 | **미수용** — 백그라운드에서 파괴 작업을 안전하게 돌릴 골격이 없음 | 선행 1·2·3·4·7 전부 | `create_delete_plan`은 신선한 `last_scan` 요구(cleanup.rs:71-78), `execute_clean`은 `*scan = None`(:146)과 `validate_for_cleanup`(:142) — 스캔·플랜·실행을 **게이트 1회 획득 안에서 연속** 수행해야 함. 그러지 않으면 사용자가 창을 여는 것만으로 예약이 무효화. 종료 훅 부재로 실행 중 종료 시 복구 불가 | 선행 7 이후 (3차, 가장 늦게) |
| ⑥ 자동 업데이트 | **미수용, 그러나 비용은 낮음** — `tauri-plugin-updater` 의존성 없음(Cargo.toml), `tauri.conf.json`에 `plugins` 섹션·`createUpdaterArtifacts` 없음 | conf에 updater + pubkey/endpoint, `capabilities/main.json`에 `updater:default`(quick엔 **금지**), CSP `connect-src`에 업데이트 호스트 추가(현재 `'self' ipc: http://ipc.localhost`만 — **최초의 외부 오리진**), CI에 `TAURI_SIGNING_PRIVATE_KEY` | 미서명 배포 + release.yml 자기 체크섬 검증(1차 리뷰) 상태라 minisign 키가 **유일한 무결성 앵커**가 됨. 키 유출 = 전 사용자 임의 코드 실행. 반대로 도입 안 하면 사고 시 회수 수단이 계속 0 | 독립 트랙, 조기 착수 권장 |
| ⑦ 시그니처 커스터마이즈 | **위험 — 현 설계로는 수용 불가** | §시그니처. `load_from_dir`(registry.rs:55) 부활 금지, `ZenithSettings`에 축소된 override만 | `register`(registry.rs:80)가 id로 blind insert → 사용자 TOML이 내장 시그니처를 **조용히 덮어씀**. `models/signature.rs:8`에 `deny_unknown_fields` 없음 + `platforms` 기본 빈 배열 = 전 플랫폼(:30,:67). `risk="safe"`면 자동선택 대상 | 선행 10 이후 (3차) |
| ⑧ 알림 센터 | **부분** — 발신자 2개 이미 존재(agent_activity/notifications.rs:94, ai_control_center/notifications.rs:20), 둘 다 `AppHandle`로 직접 토스트 | 저널과 동일 패턴(sink 트레이트로 도메인은 항목만 반환). dedupe 필터가 `GLOBAL_STORE`(agent_activity/mod.rs:41) 안에 있어 센터 상태의 소유자가 둘이 됨 → 먼저 `AppState`로 이동 | 낮음. 다만 `ai_control_center/runtime.rs:91,123`(도메인이 `AppHandle` 직접 참조 + `std::thread` 안 `block_on`)을 그대로 복제하면 테스트 불가 코드가 늘어남 | ④와 같은 2차 |
| ⑨ 탭 17개 → 그룹 재편 | **대부분 수용** — `Dashboard.svelte:75 tabGroups`가 이미 Storage/Runtime/AI 3그룹을 정의. `SegmentedTabs`·`segmentedTabs.ts` 존재 | `tabGroups`는 `DashboardTab` 10개만 덮고, 실제 라우팅 집합 `Tab`(:41)은 `large-files`/`applications`/`developer-artifacts`/`disks` 4개를 더 가짐 — 이들이 `tabDefs`(:62)·`tabGroups` 밖. 이 4개를 1급으로 올리는 게 작업의 본체 | 선행 8(revision 정합). `quickPanel.ts`는 순서 헬퍼만 있어 영향 미미(:3-30). SettingsView(798줄)의 `ReorderControls` 연동이 실제 마이그레이션 접점 | 선행 8 직후 (1차, 가장 값싼 UX 개선) |
| ⑩ Windows 패리티 | **미수용** — 종료 구현 4벌이 각자 `cfg` 분기 | 선행 6 | **필요한 SID 구현이 이미 저장소 안에 있다**(agent_activity/mod.rs:406) — 잘못된 모듈에 있을 뿐. `dev_ports/termination.rs`의 uid 상수 1000·양쪽 다 `TerminateProcess`, `agent_activity/termination.rs:236` `send_sigterm`이 `cfg(unix)` 전용은 모두 이 provider 부재의 증상 | 선행 6 (1차~2차) |

---

## 파괴 작업 통합 설계 스케치

```rust
// src-tauri/src/destructive.rs — 실행기는 분리한 채 "계약"만 공통화한다
pub trait DestructiveOperation {
    type Plan;                      // DeletePlan | TrashPlan | ProcessGroupLease
    type Preview: serde::Serialize + specta::Type;   // FE가 보는 유일한 표면
    type Outcome;
    const KIND: OpKind;             // Clean|Trash|DockerPrune|ModelDelete|ProcessStop|PortRelease|AgentStop

    fn plan(&self, req: PlanRequest) -> Result<Self::Plan, String>;
    fn preview(plan: &Self::Plan, ttl: Duration) -> Self::Preview;
    fn execute(&self, plan: Self::Plan, cancel: &CancelToken,
               journal: &dyn DeletionJournal,
               on_event: &mut dyn FnMut(OpEvent)) -> Self::Outcome;
    /// execute_clean 의 `*last_scan = None`(cleanup.rs:146) 자리를 계약화
    fn invalidate_after(&self, state: &AppState);
}

pub struct PlanVault<P>(Mutex<HashMap<Uuid, Stored<P>>>);  // cleanup.rs:94 ⊕ storage_commands.rs:73
impl<P> PlanVault<P> { fn store(&self, id: Uuid, p: P); fn take_fresh(&self, id: Uuid) -> Result<P, String>; }

pub struct CancelToken(Arc<AtomicBool>);                   // large_files/mod.rs:117 패턴 승격
pub struct CancelRegistry(Mutex<HashMap<(OpKind, String), Entry>>);  // 맵 2벌·메서드 6개 대체

pub trait DeletionJournal: Send + Sync { fn record(&self, entry: JournalEntry); }
pub struct JournalEntry { pub v: u8, pub ts: u64, pub kind: OpKind, pub plan_id: Uuid,
    pub item_id: String, pub name: String, pub path: Option<PathBuf>,   // 파일에만, IPC로는 안 나감
    pub path_digest: String, pub bytes: u64, pub sink: Sink, pub outcome: ItemOutcome }
pub enum Sink { Permanent, Trash }
pub enum ItemOutcome { Done, Failed(CleanFailureReason), Skipped, Cancelled }
```

### 각 경로 매핑과 저항 지점

| 경로 | Plan / Preview / Outcome | 적합도 | 저항 지점 (파일:라인) |
|---|---|---|---|
| P1 generic clean | `DeletePlan` / `PlanPreview` / `CleanResult` | **높음** | `CleanExecutor`가 필드 없는 free static(cleaner/executor.rs:11,16) → journal 보유 구조체로 전환 필요. `invalidate_after`는 cleanup.rs:146 그대로 이관 |
| P2~P4 trash | `TrashPlan` / `TrashPlanPreview` / `TrashResult` | **높음** | sink 주입 seam은 이미 있음(trash_manager/mod.rs:242). 단 P3(app uninstall)은 `execute_with` 안에 "번들 실패 시 잔여 스킵" 순서 의존(:254-270)이 있어 `execute`를 항목 독립 루프로 일반화하면 깨짐 |
| P5 docker prune | 신규 `DockerPrunePlan` / 추정치 preview / `u64` | **중간** | 경로 기반이 아님. `prune_category`(docker/adapter.rs:436)는 signature_id → `docker … prune -f` 직행(:448-476). dry-run이 없어 preview는 `docker system df` 기반 **추정치**뿐 — 계약에 `estimated: bool`을 넣지 않으면 preview가 거짓말이 됨. 저널은 항목 단위가 아니라 카테고리 단위로만 기록 가능 |
| P6 model delete | 신규 `ModelDeletePlan` / 항목 1개 preview / `u64` | **중간** | `delete_by_id`가 스캔→해석→삭제를 한 함수 안에서 수행(models_inventory/deleter.rs:11-23) → 스캔 결과를 plan에 고정해야 one-shot이 성립. Ollama 분기(:36)는 외부 CLI라 경로 저널이 불가능하고 blobs 델타 계산(:74)만 남음 |
| P7 terminate group | `ProcessGroupLease` / 프로세스 목록 preview / `usize` | **낮음(현재)** → 아래 최소 변경 후 높음 | 백엔드 권위가 이름 차단목록뿐(metrics/memory.rs:275-304). 시그널 직전 재검증 0건(:366-369이 `can_terminate_process` 이름 검사 후 곧장 `kill_with`) |
| P8 release listener | lease / 리스트 항목 / `ReleaseDevelopmentListenerResult` | **높음(모범)** | preview 개념만 없음. `force_authorized` 2단계(dev_ports/termination.rs:503)를 `Preview→Confirm` 계약의 정본으로 삼을 것 |
| P9 stop agent | `StopLease` / — / 결과 | **높음** | `execute_graceful_stop`(agent_activity/termination.rs:186)이 uid·start_time·exe·cwd 4중 검증(:201-224)으로 가장 엄격. Windows에서 `send_sigterm`이 `cfg(unix)`(:236)라 계약상 `Unsupported`를 표현할 자리가 없음 → 선행 6에서 해결 |

### `terminate_process_group`을 lease 모델로 끌어오는 최소 변경

P8(dev_ports)이 정본이다. 그 형태를 그대로 복사한다.

1. `MemoryInspector::sample()`(`AppState.memory_sampler`, commands/state.rs:24)이 이미 FE에 보여줄 프로세스 그룹 스냅샷을 만든다 — **여기서 lease를 발급**한다. `list_listeners`가 스냅샷 생성 중 `create_lease`를 호출하는 구조(dev_ports/termination.rs:439-457)와 동일.
2. `ProcessGroupLeaseStore`를 `AppState`에 추가(state.rs:25 `dev_port_store` 옆). 보관 값: `pids[]`, 그룹명, PID별 `(owner, start_time, exe)`, `can_terminate`, `force_authorized`, `expires_at`. **새 enum을 만들지 말고 `ReleaseMode`(models/dev_ports.rs) 재사용**.
3. 시그니처 교체: `terminate_process_group(name: String, force: bool)` → `terminate_process_group(lease_id: String, mode: ReleaseMode)` (commands/system.rs:29). `force`가 FE 판단이 아니라 **직전 graceful 실패 확인에서만 승인**되게(dev_ports/termination.rs:503 `force_authorized`) 만드는 것이 이 변경의 핵심 이득이다.
4. `metrics/memory.rs:353-372`를 재작성: 전체 프로세스 재스캔 대신 lease의 PID 집합만 조회하고, 시그널 직전 PID별 `(owner, start_time, exe)`를 lease 값과 대조(dev_ports/termination.rs:553과 동일 조건). 불일치 = `OwnershipChanged` 반환, 시그널 없음.
5. FE: `src/lib/stores/memory.svelte.ts:83`이 `(name, force)` → `(leaseId, mode)`. `ProcessMemory` 모델에 `lease_id: Option<String>` 추가(현재 노출 중인 `pid`/`pids`는 이 기회에 제거 가능).

비용 추정: Rust ~150줄(그중 절반은 lease store, `DevelopmentPortStore`를 제네릭화하면 더 줄음), FE ~30줄. **선행 6(`ProcessLifecycleProvider`)과 반드시 같이 할 것** — 따로 하면 owner 판정이 5번째로 복제된다.

---

## 삭제 저널 설계 (분석 항목 2)

**위치.** `CleanResult`는 cleaner/executor.rs:80-88에서 조립되어 :90 `Finished` 이벤트로 채널에 실리고 :92에서 반환된 뒤 cleanup.rs:148에서 소멸한다. 여기에 기록을 넣는 세 선택지 중:
- (a) 실행기 내부에서 직접 파일 쓰기 — config dir이 `AppHandle` 경유로만 얻어지고(lib.rs:205) 실행기는 Tauri-free static이다. `ai_control_center/runtime.rs:91`이 도메인에 `AppHandle`을 끌어들여 테스트 불가가 된 실수의 반복.
- (b) 커맨드 계층(cleanup.rs:148 이후, storage_commands.rs:483 이후) — 호출지 2곳이지만 **새 파괴 커맨드마다 사람이 기억해야** 한다. 1차 리뷰가 지적한 "명단 4곳 수동"과 같은 실패 유형.
- (c) **권장: sink 트레이트 주입.** `CleanExecutor::new(journal: Arc<dyn DeletionJournal>)`로 구조체화하고 구체 구현은 부트스트랩(lib.rs:205, `AuditStore::load`가 이미 하는 자리 :224)에서 만든다. 도메인은 Tauri를 모르고, 저널 없이는 실행기를 **구성할 수 없어** 강제가 구조적이며, 테스트는 `Vec` sink를 넣는다.

**포맷: JSONL append-only.** 항목당 1줄, `{v,ts,kind,plan_id,item_id,name,path,path_digest,bytes,sink,outcome}`. 회전은 diagnostics의 크기 기준 회전(diagnostics/mod.rs:10 `MAX_LOG_BYTES`, :102 검사, :118 `append(true)`) 패턴을 재사용하되 **truncate가 아니라 rename 회전**으로.

**기존 `AuditStore` 재사용은 불가.** 4가지 구조적 사유:
1. `save`가 매번 전체 `VecDeque`를 직렬화해 통째로 다시 쓰고(audit.rs:76-81), 512KB를 넘으면 **저장을 거부**한다(:77-79). 대량 정리 1회로 캡에 닿으면 그 시점부터 저널 전체가 조용히 유실 — 삭제 기록이 절대 해서는 안 되는 실패 모드.
2. `load`가 파일이 크거나(:20-22) 파싱 실패면(:23-25, :27 `unwrap_or_default`) **전량 폐기**한다. 한 바이트 손상 = 전 이력 소멸.
3. `safe_label`(:84)이 `[A-Za-z0-9_.-]` 외 전부 제거 + 48자 절단, `message`는 `sanitize_log` + 240자 절단(:53-56). 경로도 파일명도 살아남지 못한다.
4. 보존 정책이 AI 컨트롤 선호값(`retention_days`, :51) 기반이다. 삭제 이력의 보존 기간이 AI 정책 설정에 종속되는 건 소유권 혼선.

⇒ 신규 `journal/` 모듈. 다만 `sanitize_log`(diagnostics/mod.rs:62)는 **재사용**할 것 — 저널의 `error_message` 필드에 자격증명이 섞여 들어오는 경로가 실재한다(executor.rs:52-56이 이미 절대경로를 로그로 흘리고 있음).

**프라이버시: 경로 저장 vs 해시 — 둘 다, 분리해서.**
- 파일에는 전체 경로를 넣는다(그게 기능의 목적이다). 대신 **0600으로 생성**한다 — 현재 저장소에 퍼미션 지정이 0건이라(settings_store.rs, diagnostics, audit) 저널이 그 관행을 그대로 따르면 로그보다 더 나쁜 노출이 된다(로그는 최소한 `sanitize_log`를 거친다).
- IPC로는 전체 경로를 내보내지 않는다. 스캔이 이미 쓰는 `display_parent`/`name` 힌트 형태 + `path_digest`(솔트 SHA-256, 솔트는 config dir 보관)만 노출. `agent_activity/projects.rs:188`의 무솔트 opaque_id 전례를 복사하지 말 것.
- "경로 기록 / 다이제스트만" 설정 토글 + 저널 삭제 커맨드를 1급으로. 기본값은 경로 기록(그게 유용하니까)이되 사용자가 끌 수 있어야 한다.

---

## 취소 설계 보충 (분석 항목 3)

- **취소는 게이트 밖에 있어야 한다.** 이건 설계 선택이 아니라 물리다 — 취소 대상이 게이트를 쥐고 있다(cleanup.rs:31,124 / storage_commands.rs:171,263). 현재 두 취소 커맨드가 동기이고 게이트를 참조하지 않는 것(storage_commands.rs:304,340)은 **문서화되지 않은 우연**이므로 규약 + 테스트로 고정하라.
- **취소 레지스트리의 소유자.** 현재 `StorageWorkflowState`(storage_commands.rs:34)에 있는데 제너릭 스캔/클린 상태는 `AppState`(state.rs:16,21)에 있다. 제너릭 취소를 `StorageWorkflowState`에 넣으면 `commands ⇄ storage_commands` 순환이 더 깊어진다 → 선행 5가 3번보다 먼저이거나 동시여야 한다.
- **스캔 취소와 클린 취소의 의미를 통일하지 말 것.** 스캔 취소 = 부분 결과 폐기(`cancelled: true`, storage_commands.rs:208, developer_artifacts/mod.rs:183). 클린 취소 = **다음 타깃 전 정지**, 타깃 내부 중단 금지. 유일한 안전 체크포인트는 `for (index, target)` 루프 진입(executor.rs:36)이다. `SafeTreeDeleter::delete_contents`(tree_deleter.rs:41) 내부에 토큰을 넣으면 반쯤 지운 디렉터리가 남고, 그 상태를 표현할 결과 타입이 지금 없다.
- **선결 조건:** executor.rs:319-333이 Partial을 `success:true, failure_reason:None`으로 보고하고 `total_failed_bytes`를 누락한다(1차 리뷰). 이걸 먼저 고치지 않으면 "취소됨"이 "성공"으로 저널에 남는다.

---

## Trash 경유 설계 보충 (분석 항목 4)

**`trash_manager`와 `cleaner`의 전면 병합은 하지 말 것.** 두 타깃 타입이 구조적으로 다르다 — `DeleteTarget`(models/plan.rs:17)은 `strategy`·`exclusions`·`min_age_days`·`identity`를 갖고 트리 워크를 하고, `TrashTarget`(trash_manager/mod.rs:19)은 `scope`(:29 4종)를 갖고 노드 하나를 통째로 옮긴다. 합치면 `Option` 투성이 최소공배수 구조체가 되고 각 실행기가 런타임에 다시 좁힌다 — 지금보다 나쁘다.

**병합 가능한 것은 sink 하나다.** 그리고 절반은 이미 되어 있다: `TrashExecutor::execute_with(plan, move_to_trash: F)`(trash_manager/mod.rs:242-244)가 sink 파라미터화되어 있고 테스트(:842)가 실제로 주입한다.

**`SafeTreeDeleter` 뒤에 sink를 넣는 비용은 낮지 않다.** 세 가지가 구조적으로 어긋난다:
1. `delete_contents`(tree_deleter.rs:41)는 root를 남기고 **자식들을** exclusions 존중하며 지운다. `trash::delete`는 노드 단위만 가능 → 캐시 디렉터리 하나가 Trash 최상위 항목 수만 개가 된다.
2. 워커의 per-entry `O_NOFOLLOW` dirfd + identity + owner 검증(tree_deleter.rs:159,210,231,294)이 `trash::delete`에는 없다 — 이 모듈의 존재 이유가 Trash 모드에서 조용히 사라진다.
3. 회수량 회계가 다르다. Trash는 사용자가 비우기 전까지 0바이트다. `CleanResult.total_reclaimed_bytes`(executor.rs:81)와 `actual_disk_free_delta`(:74)가 둘 다 ~0을 보고해 UI가 "0바이트 정리됨"을 표시한다.

⇒ **올바른 seam은 한 단계 위, `clean_target`(executor.rs:94)에서 타깃별로 sink 선택**이다. `CleanStrategy::DeletePath`(노드 단위) 타깃만 Trash 허용, `DeleteContents`·`DockerPrune`은 Permanent 고정. 그리고 `CleanItemResult`에 `sink` 필드와 `bytes_pending_trash`를 분리 추가 — **여기가 진짜 비용이다**(응답 계약 변경이지 deleter 리팩터가 아니다).

**지금 당장 값싸게 고칠 것:** ④가 지적한 불일치의 역방향이 하나 더 있다. `models_inventory/deleter.rs:96`이 HF/LM Studio 모델을 `SafeTreeDeleter::delete_path`로 **영구 삭제**하는데, 형제 워크플로(large files)는 Trash로 보낸다. 이건 노드 단위 경로라 `trash::delete`로 바꾸는 것만으로 끝난다 — sink 추상화 전에 먼저 처리할 것.

---

## 스케줄러·업데이터·알림 보충 (분석 항목 5)

**현재 백그라운드 루프 4개, 모두 소유자 없음.**
| 루프 | 위치 | 깨우기 | 종료 수단 |
|---|---|---|---|
| awake watcher | lib.rs:288-292 | Condvar (power/watcher.rs:28,81,423) | 없음 |
| ai_control tick | lib.rs:294-303 | Condvar (runtime.rs:28,56,76), 5s/60s | 없음 |
| power 규칙 평가 | power/watcher.rs:452 (1차 리뷰) | — | 없음, 내부에 `.unwrap()` |
| runtime 내부 async | runtime.rs:123 `block_on` (std::thread 안) | — | 없음 |

`lib.rs:310`의 run 콜백은 `Ready`/`Reopen`만 처리한다 — **`ExitRequested` 훅이 아예 없다.** ⑥⑧에는 이게 미관 문제지만 ⑤에는 정합성 문제다: 예약 정리가 `execute_clean` 도중 `app.exit(0)`(lib.rs:263)으로 죽으면 게이트를 쥔 채 반쯤 지운 트리가 남고 저널도 안 남는다.

**Supervisor 최소 형태:** `{name, interval, cancel: Arc<AtomicBool>, JoinHandle}` 목록 + `RunEvent::ExitRequested`에서 부르는 `shutdown()`. **기존 Condvar 두 개를 대체하지 말고 감싸라** — 이미 "설정 변경 시 즉시 깨어남"을 제공하며, 순진한 `sleep` 루프로 바꾸면 그 능력을 잃는다.

**⑤가 현 파이프라인을 그대로 못 쓰는 이유:** `create_delete_plan`은 신선한 `last_scan`을 요구하고(cleanup.rs:71-78), `execute_clean`은 `validate_for_cleanup`(:142) 후 `*scan = None`(:146)을 한다. 예약 실행은 스캔→플랜→실행을 **게이트 1회 획득 안에서 연속** 수행해야 하며, 그러지 않으면 사용자가 대시보드를 여는 것만으로 예약이 무효화된다. 반가운 점 하나: `on_event: FnMut(CleanEvent)`(executor.rs:17-18)가 제네릭이라 스케줄러가 Tauri `Channel` 대신 저널 기록 클로저를 넘기는 건 **지금 구조로 이미 된다**.

**업데이터(⑥) 도입 시 변경 표면:** ① `tauri.conf.json`에 `plugins.updater`(pubkey + endpoints) 및 `bundle.createUpdaterArtifacts`, ② `capabilities/main.json`에 `updater:default` — `quick.json`에는 **절대 부여 금지**(현재도 read-mostly를 표방하면서 `allow-execute-clean`을 갖고 있는 전례가 있다), ③ CSP `connect-src 'self' ipc: http://ipc.localhost`에 업데이트 호스트 추가 — 앱 생애 **최초의 외부 오리진**이므로 별도 검토, ④ CI에 `TAURI_SIGNING_PRIVATE_KEY`. 미서명 배포 + 자기 체크섬 자기검증(1차 리뷰) 상태에서 minisign 키가 유일한 무결성 앵커가 되는 것은 위험이자 **현재의 "회수 수단 0"에 대한 가장 강한 해법**이다.

**⑧ 알림 센터:** 발신자 2개(agent_activity/notifications.rs:94, ai_control_center/notifications.rs:20)가 `AppHandle`로 직접 토스트한다. 저널과 같은 처방 — 도메인은 항목만 반환, 셸이 토스트+영속. 단 dedupe 필터가 `GLOBAL_STORE`(agent_activity/mod.rs:41) 안에 살아서 센터 상태의 소유자가 둘이 된다. 필터를 `AppState`로 먼저 옮길 것.

---

## 시그니처 커스터마이즈 보충 (분석 항목 6)

**`load_from_dir`(registry.rs:55)를 그대로 살리는 것이 최악의 선택인 이유** — "임베드 = 검토됨" 계약이 무너지는 지점이 하나가 아니다:
1. `register`(registry.rs:80)가 id로 blind `insert` → 사용자 TOML이 `system.developer_temp` 같은 **내장 시그니처를 조용히 덮어쓴다**. `paths`를 덮으면 임의 삭제 루트.
2. `models/signature.rs:8`에 `deny_unknown_fields`가 없고 `platforms`(:30) 기본값이 빈 배열 = 전 플랫폼(:67-69). 오타 하나가 범위를 조용히 넓힌다.
3. `risk`(:12)가 평범한 필드다. 사용자 파일이 `risk = "safe"`를 선언하면 자동선택 대상이 된다(1차 리뷰: `intensive.user_app_caches` risk=safe → 자동선택).
4. 하류 안전장치 전부가 "이 시그니처는 검토됐다"를 전제한다. `safety/planner.rs:95-99`는 min_age 시그니처에 대해 `path == root`를 이미 허용한다.

**안전하게 열 수 있는 최소 표면 — 로더를 만들지 말고 설정을 확장하라.**
- 이미 `ZenithSettings.excluded_signatures`(cleanup.rs:25에서 소비)가 시그니처 **비활성화**를 제공한다. 여기에 `signature_overrides: HashMap<String, SignatureOverride>`를 추가하고, `SignatureOverride`는 **`extra_exclusions: Vec<String>`과 `min_age_days`(증가 방향만)만** 갖는다.
- 병합 지점은 `SignatureRegistry::get`(registry.rs:85)·`by_category`(:95)·`by_category_for_mode`(:103) — 모든 소비자가 자동으로 본다. 새 로더 경로가 생기지 않는다.
- 범위를 넓힐 수 있는 것(`paths`·`risk`·`strategy`·`include_prefixes`·`platforms`·`intensive_only`)은 override 불가. **새 시그니처 추가는 범위 밖** — 그건 `signatures/*.toml`에 대한 PR이고, 임베드 설계가 사주는 것이 바로 그 리뷰 게이트다.
- 선결: exclusions 매처 불일치(선행 10). 이걸 안 고치면 ⑦은 "제외했는데 지워졌다" 버그를 사용자에게 직배송한다.

---

## Windows 패리티 보충 (분석 항목 8)

`platform/mod.rs:11-16` 주석이 "process lifecycle provider는 담당 이슈에서 도입"이라 적어놓고 끝났다. 그 사이 종료 구현이 4벌이 됐고 각자 다른 `cfg` 분기를 갖는다. **필요한 Windows 구현은 이미 저장소 안에 있다** — `agent_activity/mod.rs:406`이 SID 비교를 올바르게 한다. 모듈 위치가 틀렸을 뿐이다.

```rust
// src-tauri/src/platform/process.rs
pub struct OwnerId(String);      // uid 문자열화 | SID — u32 uid 타입 제거가 핵심
pub struct ProcessFacts { pub owner: OwnerId, pub start_time: u64,
                          pub exe: PathBuf, pub cwd: Option<PathBuf> }

pub trait ProcessLifecycleProvider: Send + Sync {
    fn current_owner(&self) -> OwnerId;
    fn inspect(&self, pid: u32) -> Option<ProcessFacts>;
    fn request_stop(&self, pid: u32) -> Result<(), StopError>;  // SIGTERM | CTRL_BREAK/WM_CLOSE
    fn force_stop(&self, pid: u32) -> Result<(), StopError>;    // SIGKILL | TerminateProcess
}
pub enum StopError { Unsupported, NotPermitted, Gone, Io(String) }
```

설계 요점 둘:
- `OwnerId`를 불투명 newtype으로 만드는 것이 핵심 수다. `u32` uid 타입이었기 때문에 Windows에서 `1000` 상수가 **컴파일에 성공했다**. 문자열 newtype이면 그 코드가 애초에 안 짜인다.
- `request_stop`/`force_stop`을 **별개 메서드**로 두면(오늘의 `force: bool`이 아니라) "이 플랫폼엔 graceful이 없다"를 `StopError::Unsupported`로 표현할 수 있고, `PlatformCapabilities`가 그걸 그대로 UI에 노출한다. 오늘은 두 경우 모두 조용히 `TerminateProcess`다.

주의: `PlatformCapabilities::current()` 본체가 `models/platform.rs:226`에 있다(1차 리뷰). 새 provider의 가용성 플래그를 거기에 또 쓰면 "모델이 플랫폼 정책을 소유"하는 분열이 깊어진다 — provider 도입과 함께 판정을 `platform/capabilities.rs:15`(현재 19줄 위임 껍데기)로 끌어올릴 것.

---

## 하지 말아야 할 구조 변경

1. **모든 파괴 경로를 관통하는 단일 구체 타입 `DestructiveTarget`/`DestructivePlan`.** 타깃들이 진짜로 다르다 — `DeleteTarget`은 `strategy`·`exclusions`·`min_age_days`(models/plan.rs:17-28), `TrashTarget`은 `scope` 4종(trash_manager/mod.rs:19-44), lease는 `pid`/`port`/`start_time`. 공통 구조체는 `Option` 수프가 되고 각 실행기가 런타임에 다시 좁힌다 — 컴파일러 강제력이 오히려 지금보다 줄어든다. 공통화할 것은 **연관 타입 계약 + 저널 엔트리 + PlanVault** 셋뿐이고, 실행기는 분리된 채로 둔다.

2. **범용 이벤트 버스 / 커맨드 버스 / 제네릭 스케줄러 프레임워크.** `Supervisor`는 80줄이면 된다 — 이름 붙은 태스크 Vec + 취소 플래그 + `shutdown()`. 기존 Condvar 깨우기 두 개(power/watcher.rs:81, runtime.rs:56)는 잘 동작하며 "설정 변경 시 즉시 깨어남"까지 제공한다. 결함은 **종료 훅 부재**(lib.rs:310)이지 루프 모양이 아니다. 같은 맥락에서 **`ExecutionBudgets`(execution_budget.rs:114)를 스케줄러·업데이터로 확장하지 말 것** — `acquire_storage_read`는 호출자가 0(:139), `acquire_subprocess`는 1이다. 이미 과설계된 모듈을 키우고 과소설계된 게이트를 방치하는 방향이다.

3. **⑦을 위해 `load_from_dir`(registry.rs:55)를 부활시키거나 시그니처 플러그인 포맷을 만드는 것.** 위 §시그니처의 4가지 우회로가 전부 열린다. 사용자가 실제로 원하는 것은 "이 캐시는 건드리지 마"인데, 그건 이미 있는 `excluded_signatures`의 세분화(exclusion 추가 + min_age 상향)로 100% 충족된다. 플러그인 시스템은 그 요구의 20배짜리 표면적이다.

---

## 총평

파괴 경로 9개가 plan형·lease형·raw형 셋으로 갈려 있고, **가장 잘 설계된 것(dev_ports lease)과 가장 위험한 것(terminate_process_group)이 같은 저장소에 공존**한다 — ①은 다이얼로그 작업이 아니라 raw형 3개에 plan 단계를 만드는 작업이다.
②③④⑤가 전부 같은 세 개의 부재를 공유한다: plan 저장소 통합(`PlanVault`), 취소 레지스트리 통합, 실행 결과 sink. 이 셋을 먼저 만들면 네 기능이 각자 새 배관을 깔 필요가 없고, 안 만들면 `storage_commands.rs`의 "맵 2벌 + 메서드 6개" 복제 패턴이 4벌·6벌로 번진다.
`StorageOperationGate`가 여전히 모든 것의 앞을 막는다 — poison 패닉(operation_gate.rs:11)뿐 아니라, **취소 커맨드가 게이트를 잡으면 영구 교착**이라는 미문서화 규약이 ③⑤의 전제다.
값싼 순서는 명확하다: ⑨(그룹 골격이 Dashboard.svelte:75에 이미 있음) → ①②③(선행 1~4) → ⑥(독립 트랙, 회수 수단 0을 메우는 유일한 수단) → ⑩⑧④ → ⑤⑦(가장 늦게).
과설계 위험은 통합 자체가 아니라 **통합의 입도**에 있다 — 실행기를 합치려는 순간 최소공배수 타입이 태어나고, 계약만 합치면 오늘의 좋은 부분(TOCTOU 검증·lease 재확인·one-shot plan)을 잃지 않는다.


---

# 부록 — FE코드리뷰 차원 원본 (improve-code.md)

## Zenith FE 개선 실행성 리뷰 — UI/UX 개선 착수 전 코드 상태 진단

대상: `zenith` FE (`src/`, Svelte 5 runes + TS + Tailwind 4). 실측 기준 — 뷰 17개(6,957줄) · 공용 컴포넌트 25개(1,978줄, 최상위 22 + `ai-activity/` 3) · 스토어 12개(1,730줄) · API/유틸 20개(4,843줄) · 테스트 32파일 207 `it()`.
1차 리뷰(`code.md`·`report.md`)에서 확인된 FE 결함을 "개선을 막는가"의 관점으로 다시 판정했다. 읽기 전용 — 수정 없음.

---

## 먼저 고칠 것(개선 착수 전)

- `src/lib/utils/tauri.ts:1-326` | export 62개 중 61개가 `api.X()`/`storageApi.X()` 무동작 재수출(실로직은 `tauriStartWindowDrag` 하나), 뷰·스토어 28개 파일이 예외 없이 이 층만 경유 | ①③④가 전부 신규 커맨드를 동반하는데 커맨드 1개마다 이 파일에 래퍼 1개가 공짜로 늘어난다. 개선 착수 전에 걷어내야 커맨드 추가 비용이 파일 10개→9개로 선형 감소한다.
- `src/lib/stores/scan.svelte.ts:374` | `runScan`이 in-flight `scanRequest` 존재만 보고 반환해 `categories` 인자를 무시 — 요청 동일성 개념이 없음 | ③ 취소·진행률은 "지금 도는 게 어느 요청인가"를 식별해야 성립한다. 취소 버튼을 붙이면 사용자가 누른 스캔이 아닌 다른 스캔이 취소된다.
- `src/routes/dashboard/ApplicationsView.svelte:149` + `LargeFilesView.svelte:192` | `inspectApp`/`scanFiles` 모두 세대 토큰·재진입 가드 없음, LargeFiles는 :443 "Scan again"이 `isScanning`으로 비활성화되지도 않음 | ① 공통 확인 다이얼로그의 전제는 "확인 시점 대상 = 실행 대상"이다. stale 응답이 `inspection`/`items`를 갈아끼우면 다이얼로그가 A를 보여주고 B를 지운다.
- `src/routes/quick/QuickPanel.svelte:174-180` | `handleCleanSafe`가 `selectQuickCleanDefaults()` → `cleanSelected()` 직행, 확인 UI가 존재하지 않음(플랜 프리뷰 상태 변수조차 없음) | ①에서 가장 위험한 경로인데 다이얼로그를 끼워 넣을 중간 상태가 없어, 컴포넌트 교체가 아니라 흐름 재작성이 된다.
- `src/routes/quick/QuickPanel.svelte:106` vs `:137` | `startPolling` 조건은 `hasSection('memory') && memoryAvailable`, `stopPolling` 조건은 `hasSection('memory')`뿐인 refcount 비대칭 | ⑤ 탭/섹션 재편은 `hasSection` 조건을 직접 흔든다. 비대칭을 남긴 채 섹션을 재편하면 재편 조합마다 새 누수 경로가 생긴다.
- `src/lib/stores/settings.svelte.ts:118-122` | `performLoad` 폴백이 `dashboard_tabs` 9개 배열 + `dashboard_tabs_revision ?? 3`인데, 생성자 기본값(`:22-23`)은 7개 + revision 5이고 백엔드 `src-tauri/src/models/settings.rs:226`도 5 | ⑤ 탭 재편은 revision 6 마이그레이션이다. 폴백이 3으로 되감으면 `settings.rs:258-320`의 revision 1~5 마이그레이션이 재실행되어 사용자가 지운 탭이 되살아난다.
- `src/lib/stores/scan.svelte.ts:167-236` | `reclaimableBytes` 등 집계 6개가 risk 조건만 다른 동일 이중 루프이고 클래스 getter라 비메모이즈 — StorageView·QuickPanel이 렌더마다 전체 항목을 6회 재순회 | ⑦ 예상/실측 이중 표기는 집계축을 2배로 늘린다. 6회가 12회가 되는데 `$derived.by` 전환 없이는 항목 수에 그대로 비례한다.
- `src/lib/components/CleanResultModal.svelte:33` + ad-hoc 오버레이 5곳(`ModelsView:195`·`AwakeView:450`·`DockerView:303`·`MemoryView:319`·`DevelopmentServersView:269,290`) | 네이티브 `<dialog>`/`showModal()`을 쓰는 곳은 `CleanupReviewDialog.svelte:28`이 유일 — 나머지는 `fixed inset-0` div라 Escape·포커스 트랩·포커스 복귀가 전부 없음(`aria-modal`은 전 저장소 4건) | ②와 ⑧의 공통 기반이 접근성 0인 상태. 결과 모달을 상세화할수록 키보드 사용자가 빠져나올 수 없는 화면이 커진다.

---

## 컴포넌트 재사용 표

사용처 수는 테스트 제외(뷰 + 다른 컴포넌트에서의 import 실측).

| 컴포넌트 | 사용처 수 | 중복 구현 위치 | 공통화 판정 |
|---|---|---|---|
| `Button.svelte` | 26 | 없음 | 정착 — 전 뷰가 사용. variant/size 계약 그대로 재사용 가능 |
| `Card.svelte` | 20 | 없음 | 정착 |
| `Badge.svelte` | 11 | `RiskBadge`가 래핑(정상) | 정착 |
| `ProgressBar.svelte` | 8 | 없음. 단 `aria-valuenow` 전 저장소 1건뿐 | 재사용 O, ③ 진행률 전에 role="progressbar" 보강 필요 |
| `DeletingDots.svelte` | 7 | 없음 | 정착 |
| `PageHeader.svelte` | 7 | `StorageView`·`CategoryDetailView`·`ApplicationsView`·`LargeFilesView`·`DeveloperArtifactsView`·`ProjectCockpitView`·`AiControlCenterView`·`QuickPanel`이 자체 헤더 마크업 | **미완 통일** — 10뷰 중 7만 사용, 탭 재편(⑤) 시 헤더 불일치가 드러남 |
| `InlineNotice.svelte` | 6 | 인라인 에러 렌더 4곳(`AiControlCenterView:112`, `StorageView:323`, `CategoryDetailView:190`, `UsagePanel:34`/`ToolAdaptersPanel:66`/`ProjectsPanel:96`) | **미완 통일** — ⑩ 토스트 도입 시 에러 표시 경로가 2계통으로 갈림. props(variant/title/message/onDismiss/actionLabel)는 이미 충분 |
| `CleanResultModal.svelte` | 3 (`StorageView`·`CategoryDetailView`·`QuickPanel`) | 없음 | ② 기반으로 적합. 단 `<dialog>` 미사용 |
| `EmptyState.svelte` | 3 (`ModelsView`·`DevelopmentServersView`·`DockerView`) | ad-hoc 중앙정렬 빈/로딩 블록 12곳(`Dashboard:328`, `AiControlCenterView:113`, `DiskView:99` 등) | **공통화 필요** — 12곳을 흡수하면 ⑥ 스킵 사유 표시를 한 컴포넌트에 붙일 수 있음 |
| `Switch` / `Checkbox` | 3 / 3 | 원시 `<input type="checkbox">` 직접 사용(`DeveloperArtifactsView:507` 등) | 부분 통일 |
| `ByteValue.svelte` | 2 (`MemoryView`·`StorageView`, 총 19회) | `formatBytes()` 직접 호출 18파일 66회 | **공통화 필요** — ⑦ 이중 표기의 유일한 진입점이어야 하는데 66:19로 우회가 3배 |
| `CleanupReviewDialog.svelte` | **1** (`StorageView:363`) | 파괴 확인이 4계통으로 분산(아래 표) | **최우선 공통화** — ① 그 자체 |
| `SegmentedTabs.svelte` | **1** (`StorageView`) | `Dashboard.svelte:212-247` 사이드바 nav 자체 구현, `ProjectCockpitView`도 자체 탭 | ⑤ 대상. 키보드 처리(`utils/segmentedTabs.ts`)는 이미 분리돼 재사용 가능 |
| `SelectionToolbar` / `CategoryCard` / `ItemRow` / `ScanFreshnessNotice` / `QuickUsageGauges` / `ReorderControls` | 각 1 | — | 단일 소비자 전용. 재사용 자산으로 계산하지 말 것 |
| `ai-activity/{ProjectsPanel,ToolAdaptersPanel,UsagePanel}` · `AiUsageCards` | 각 1 | — | ProjectCockpit 전용 서브트리 |

### 파괴 작업 확인 UI — 현재 4계통

| 계통 | 구현 | 호출부(파일:라인) | ①에서 필요한 것 |
|---|---|---|---|
| A. 네이티브 dialog | `CleanupReviewDialog.svelte` | `StorageView.svelte:363` (트리거 `:85 handleCleanSelected`, 실행 `:94 confirmCleanup`) | 그대로 일반화 대상 |
| B. 인라인 플랜 패널(모달 아님) | 페이지 하단 카드 + TTL 카운트다운 | `LargeFilesView.svelte:417-450`(실행 `:257`), `ApplicationsView.svelte:409+`(실행 `:205`), `DeveloperArtifactsView.svelte:466-510`(실행 `:338`) | `TrashPlanPreview`(item_count/allocated_size/expires_at) + `partialCleanupConfirmed` 같은 추가 체크박스 슬롯 |
| C. ad-hoc `fixed inset-0` 오버레이 | div 오버레이, 포커스 관리 없음 | `DockerView.svelte:303`(실행 `:31`), `MemoryView.svelte:319`(실행 `:90`), `DevelopmentServersView.svelte:269`·`:290`(실행 `:75`,`:100`, 포커스는 `:107 focusDialog` 수동), `ModelsView.svelte:195`(실행 `:57`), `AwakeView.svelte:450`(폼 모달, ① 범위 밖) | 제목/본문/위험도/2버튼 + 이중 확인(Quit Normally vs Force Quit) 지원 |
| D. **확인 없음** | — | `QuickPanel.svelte:174`(`handleCleanSafe`), `CategoryDetailView.svelte:74`(`cleanSelected`), `DockerView.svelte:122`·`:156`·`:190`·`:224`(prune 4종 즉시 실행) | 새 상태 변수 + 다이얼로그 삽입 = 흐름 신설 |

---

## 개선별 구현 비용

"건드릴 파일 수"는 생성 파일(`bindings/tauri.ts`) 포함, 테스트 포함 실측 추정.

| # | 개선 | 건드릴 파일 수 | 재사용 가능 자산 | 신규 커맨드 | 비용 | 선행 조건 |
|---|---|---|---|---|---|---|
| ① | 파괴 확인 공통 다이얼로그 | 14 (컴포넌트 1 + 뷰 9 + 테스트 3 + utils 1) | `CleanupReviewDialog`(dialog·focus 복귀·oncancel 완비), `RiskBadge`, `Button variant="destructive"`, `ttlRemaining`/`formatCountdown` | 없음 | **M** | 세대 토큰 3곳(scan:374, Applications:149, LargeFiles:192), QuickPanel 흐름 신설 |
| ② | 결과 모달 상세화(Partial·잔여·사유) | FE 5 + BE 3 + bindings 1 = 9 | `CleanResultModal`(status/partial/failed 분기 이미 존재 :20-30), `cleanResult.ts:9 cleanOutcome` | 없음(기존 `CleanResult` 필드 확장) | **M**(BE 동반 시 L) | `cleaner/executor.rs:321` Partial을 `success:true`로 보고 + `total_failed_bytes` 미계상, `:336` 실패 사유가 영어 문자열 매칭 — 고치지 않으면 표시할 진실이 없음 |
| ③ | 스캔/클린 취소 + 진행률 | 13~15 (build.rs·lib.rs·capabilities 2·commands 2·bindings·native·mock·utils/tauri·scan store·뷰 3) | 대용량파일/아티팩트는 **이미 있음** (`bindings:80 cancelLargeFileScan`, `:89 cancelDeveloperArtifactScan`, `cancelled` 이벤트) → 버튼 노출만; 클린 진행률도 `scan.svelte.ts:445` index/total/percent 존재 | `cancel_scan`, `cancel_clean` 2개 | **L** | `scanner/engine.rs:15` 취소 토큰 부재 + 배타적 `operation_gate`, `runScan:374` 요청 동일성 부재. 메인 스캔은 percent 자체가 없어 `ScanEvent` 확장 필요 |
| ④ | 삭제 히스토리 뷰(신규) | 16~20 (BE 신규 모듈+영속 5, `DashboardTab` enum·revision 마이그레이션 3, FE 신규 뷰 1 + Dashboard/settings 4, 테스트 3) | `PageHeader`, `EmptyState`, `ItemRow`, `ByteValue`, `formatTimeAgo`, `settings_store.rs` 영속 패턴 | `get_clean_history`(+`clear_clean_history`) | **L** | **삭제 내역 영속 기록이 아예 없음**(`cleaner/executor.rs:81` — 실패만 로그). 데이터 소스 신설이 본체이고 뷰는 부수 |
| ⑤ | 탭 17→그룹 재편 | 10 (Dashboard 1, settings store 1, Rust settings 1, `dashboardNavigation.ts` 1, 테스트 3, 기타 3) | **`Dashboard.svelte:74-85 tabGroups` + `:212-217` 그룹 헤더 렌더가 이미 구현돼 있음**, `SegmentedTabs` + `utils/segmentedTabs.ts` 키보드 처리 | 없음 | **S**(라벨/그룹 재배치만) / **M**(탭 병합·신설 포함) | 기본 탭 배열이 3곳 중복(`settings.svelte.ts:22`, `:123`, `Dashboard.svelte:212` 인라인 폴백) — 하나로 합치고 revision 6 추가. `models/settings.rs:258-320` 마이그레이션 체인에 6 추가 |
| ⑥ | 스킵 사유·불완전 스캔 표시 | 3~6 | **DeveloperArtifacts는 이미 완비** (`:173-186 statusLabel`, `:551-579 incomplete_reason`, `skipped_entries`) — 그 패턴을 이식 | 없음(LargeFiles) / 있음(메인 스캔 필드 추가) | **S**(LargeFiles·Applications) / **M**(메인 스캔) | 없음. 착수 난이도 최저 — 우선 처리 권장 |
| ⑦ | 회수량 예상/실측 이중 표기 | 20+ (`ByteValue` 1, `format.ts` 1, `formatBytes` 직접 호출 18파일) | `ByteValue.svelte`(12줄, 확장 여지 큼), `size.allocated ?? size.logical` 관례 | 없음 | **M** | `formatBytes` 직접 호출 66회를 `ByteValue`로 수렴시키는 게 선행. 안 하면 표기가 화면마다 갈림. 덤으로 `format.ts:5` 1024진수에 SI 라벨(KB/MB) 오표기도 여기서 정리 |
| ⑧ | 키보드 내비게이션·aria | 25+ (전 뷰) | `utils/segmentedTabs.ts handleSegmentedTabKeydown`(테스트까지 있음), `CleanupReviewDialog`의 포커스 복귀 패턴 | 없음 | **L** | ① 선행 필수 — 모달 5종을 `<dialog>`로 통일하지 않으면 뷰마다 포커스 트랩을 따로 구현하게 됨 |
| ⑨ | i18n | 40+ (전 뷰·컴포넌트 + 테스트 21) | 없음(라이브러리·키 체계 0) | 없음 | **L** | 테스트 257건이 영어 문자열을 직접 `toContain` — 문자열을 키로 바꾸는 순간 21개 테스트 파일이 동시에 깨진다. i18n 착수 전 테스트를 `data-testid`/구조 기반으로 이관하는 게 진짜 선행 작업 |
| ⑩ | 알림/토스트 체계 | 6~9 (`App.svelte` 1, 신규 store 1, 신규 Toast 1, InlineNotice 소비 6) | `InlineNotice.svelte`(variant/title/message/onDismiss/actionLabel/`role=status`↔`role=alert` 이미 구현) | 없음 | **S~M** | 스토어 12개의 에러 표현이 3종(`error instanceof Error ? .message : String()` / `e?.toString()` / `err?.message \|\| String(err)`)으로 갈려 있음 — 토스트 host가 소비할 단일 형태로 통일 필요 |

### API 계층 실측 — 커맨드 1개 추가 시 건드리는 파일

`cancel_clean` 하나를 추가하면 **9~10개 파일**:

1. `src-tauri/src/commands/cleanup.rs` — 커맨드 본체
2. `src-tauri/build.rs:2` — `COMMANDS` 배열(수동)
3. `src-tauri/src/lib.rs:325` — `collect_commands![]`(수동)
4. `src-tauri/capabilities/main.json` — `allow-cancel-clean`(수동, 현재 62줄)
5. `src-tauri/capabilities/quick.json` — Quick Panel에서도 쓸 경우(수동)
6. `src/lib/bindings/tauri.ts` — 생성(`just generate-bindings`, CI가 `git diff --exit-code`로 검증)
7. `src/lib/api/native.ts` — 래퍼
8. `src/lib/api/mock.ts` — **`satisfies ZenithApi`(:1145)가 강제**하므로 누락 시 컴파일 에러
9. `src/lib/utils/tauri.ts` — 무동작 래퍼(제거하면 이 줄이 사라짐)
10. `src/lib/models/types.ts` — 신규 타입이 있을 때만

`test/apiContract.test.ts`는 `Object.keys` 동일성만 보므로 7·8을 채우면 자동 통과 — 추가 작업 없음. **계약 동기화 비용 자체는 낮다**(`satisfies` + 키 집합 테스트 + CI bindings drift 검증이 이미 3중으로 막고 있음). 실제 낭비는 2·3·4·5의 수동 명단 4곳과 9의 무동작 층이다.

### 탭 재편(⑤) 영향 상세

- `Dashboard.svelte:62-72 tabDefs` — 10개 정의(`storage/disk/docker/models/memory/development_servers/projects/ai_control/usage/awake`) + `:41 type Tab`의 서브루트 4개(`large-files`/`applications`/`developer-artifacts`/`disks`) + `settings` = 실제 라우트 15
- `Dashboard.svelte:74-85 tabGroups` — Storage/Runtime/AI 3그룹이 **이미 존재**, `:212-217`에서 그룹 헤더·구분선까지 렌더 중. 재편은 이 맵 재정의가 본체
- `settings.svelte.ts:22` 기본 배열(7개) ↔ `:123` 폴백 배열(9개) ↔ `Dashboard.svelte:212` 인라인 폴백(7개) — **3곳이 서로 다름**
- `src-tauri/src/models/settings.rs:225-320` — `DashboardTab::ALL`, revision 1~5 마이그레이션 체인, `:244-253` 정규화/중복제거. 재편은 revision 6 블록 추가
- 테스트: `dashboard.test.ts`(3건, SSR 렌더), `settings.test.ts`(21건, 마이그레이션·저장 큐), `redesign-controls.test.ts:27`(`normalizeDashboardTab`), `quickPanel.test.ts`(17건, 섹션은 `quick_panel_sections`로 별도 — dashboard_tabs와 무관하므로 **영향 없음**)

---

## 패턴 제안 코드

```ts
// src/lib/utils/request.svelte.ts — 세대 토큰 + 인자별 in-flight dedupe (스토어·뷰 공용)
export function createRequest<A extends unknown[], R>(run: (...args: A) => Promise<R>) {
  let generation = 0;
  let inflight: { key: string; promise: Promise<R | null> } | null = null;
  return {
    get isRunning() { return inflight !== null; },
    /** 같은 인자면 진행 중 프로미스를 공유하고, 다른 인자면 이전 세대를 폐기한다. */
    async call(...args: A): Promise<R | null> {
      const key = JSON.stringify(args);
      if (inflight?.key === key) return inflight.promise;
      const mine = ++generation;
      const promise = run(...args)
        .then((value) => (mine === generation ? value : null))   // stale 응답 폐기
        .finally(() => { if (inflight?.key === key) inflight = null; });
      inflight = { key, promise };
      return promise;
    },
    /** 진행 중 요청을 무효화한다(취소 커맨드와 짝지어 호출). */
    invalidate() { generation++; inflight = null; },
  };
}
```

```ts
// src/lib/components/ConfirmDialog.svelte — CleanupReviewDialog 일반화 props
interface ConfirmDialogProps {
  title: string;                 // "Review cleanup" / "Quit Chrome?" / "Prune unused Docker volumes?"
  description: string;           // 되돌릴 수 없음 등 결과 설명
  items?: { id: string; name: string; risk?: RiskLevel; bytes?: number }[]; // 없으면 목록 생략
  warning?: string;              // rebuild/partial 경고 (기존 :40)
  blockedReason?: string;        // 만료·스캔 변경 등 실행 차단 사유 (기존 :51 disabled)
  expiresAt?: number;            // TrashPlanPreview.expires_at — 카운트다운 자동 표시
  acknowledge?: string;          // 체크해야 실행 가능 (DeveloperArtifacts:507 partial 확인)
  confirmLabel?: string;         // 기본 "Confirm"
  extraAction?: { label: string; onSelect: () => void }; // MemoryView "Force Quit"
  onCancel: () => void;
  onConfirm: () => void;
}
```

`<dialog>` + `showModal()` + `oncancel` + 포커스 복귀는 `CleanupReviewDialog.svelte:18-33`을 그대로 옮기면 되고, 위 props로 A·B·C 3계통과 D의 신설 지점을 전부 덮는다.

---

## 접근성·i18n 실측

| 항목 | 실측값 | 분포 / 비고 |
|---|---|---|
| `aria-*` 총계 | **78** | `aria-label` 46 · `aria-labelledby` 10 · `aria-hidden` 9 · `aria-modal` 4 · `aria-selected` 2 · `aria-controls` 2 · `aria-valuenow/min/max` 각 1 · `aria-orientation` 1 · `aria-describedby` 1 |
| `aria-*` 0건 파일 | 43개 중 **12개** | `ModelsView`(모달 보유), `DiskView`, `SettingsView`(798줄, role 4건만), `ItemRow`, `CategoryCard`, `CleanResultModal`, `EmptyState`, `ProgressBar`, `Card`, `Badge`, `RiskBadge`, `ByteValue` |
| `role=` | 41 | `role="dialog"` 4(모달 6종 중 4) · `role="status"`/`"alert"`는 `InlineNotice`에 집중 |
| 키보드 핸들러(`onkeydown`) | **4** | `SegmentedTabs` 1 · `DevelopmentServersView` 1 · `MemoryView` 1 · `ProjectCockpitView` 1. 나머지 39파일 0 |
| `tabindex` | 8 | `ai-activity/*` 3 · `ApplicationsView` 2 · `LargeFilesView` 2 · `ProjectCockpitView` 2 · `SegmentedTabs` 1 · `StorageView` 1 |
| 포커스 트랩·복귀 | **1** | `CleanupReviewDialog.svelte:18-25`만. 모달 6종 중 5종은 Escape로 닫히지 않음 |
| 하드코딩 영어 문자열 | **약 600** (중복 포함 834, 실효 추정 550~620) | 마크업 텍스트 노드 291 · script 내 문장 리터럴 393 · UI 속성(title/label/placeholder/aria-label) 150 |
| 상위 5파일 | `SettingsView` 74 · `AiControlCenterView` 60 · `DeveloperArtifactsView` 48 · `AwakeView` 45 · `ProjectDetailPanel` 35 | 공용 컴포넌트 쪽은 대부분 0~10 — **문자열이 뷰에 몰려 있어 i18n 추출 범위는 좁다** |
| i18n 라이브러리 | **없음** | `package.json` 의존성 0. `format.ts:68`이 `Intl.DateTimeFormat('en', …)`로 로케일 하드코딩, `formatBytes`도 영어 단위 고정 |
| i18n 도입 실제 병목 | **테스트 257건** | `toContain('...')` 영어 문자열 단정이 21개 테스트 파일에 분산. 문자열 → 키 치환 시 동시 붕괴 |

---

## 총평

파괴 작업 확인 UI가 4계통(네이티브 dialog 1 · 인라인 플랜 패널 3 · ad-hoc 오버레이 5 · **확인 없음 6곳**)으로 갈려 있고, 제대로 만들어진 `CleanupReviewDialog`가 정작 StorageView 1곳에만 쓰인다 — ①이 최대 효과 대비 최저 비용이다.
탭 재편(⑤)은 `Dashboard.svelte:74-85`에 그룹 맵과 헤더 렌더가 이미 있어 예상보다 싸지만, 기본 탭 배열이 3곳에서 서로 다르고 폴백 revision이 3(백엔드 5)이라 마이그레이션 되감김을 먼저 막아야 한다.
③취소와 ④히스토리는 FE 작업이 아니라 백엔드 신설이다 — 메인 스캔에 취소 토큰이 없고(`scanner/engine.rs:15`) 삭제 내역 영속 기록은 존재하지 않는다(`cleaner/executor.rs:81`).
테스트 207건은 전부 `svelte/server`의 SSR 문자열 렌더 또는 순수 함수이고 클릭·포커스를 검증하는 테스트가 0건이라, 새 다이얼로그의 "확인→실행" 흐름은 지금 스택으로는 검증 불가 — jsdom 환경 + `mount`/`flushSync` 도입이 ①과 세트다.
착수 순서 권장: ⑥(무비용) → `utils/tauri.ts` 제거 + 세대 토큰 3곳 → ①·⑩ → ⑤ → ⑦ → ②(BE 동반) → ③④ → ⑧⑨.


---

# 부록 — 보안 차원 원본 (improve-security.md)

## Zenith 개선안 보안 영향 리뷰 (읽기 전용)

대상: `zenith` v0.3.8 (Tauri 2 + Rust + Svelte 5, 로컬 데스크톱).
전제: 1차 리뷰(`security.md`, `report.md`) 결과를 반영한 재판정 상태. 위협모델 = 사용자 실수(주) > 침해 WebView(부, CSP `script-src 'self'`) > 같은 UID 로컬 프로세스(심층방어) > 공급망 > 네트워크 응답.
라인번호는 전부 실측값. 파일 수정·네트워크 접근 없음.

## 사전 정정 3건 (임무 지시문 전제 중 실측과 다른 부분)

1. **"absolute paths never cross into WebView"는 전역 계약이 아니다.** README.md:33-36의 해당 문장은 **Projects cockpit 항목의 하위 서술**이다("A local Projects cockpit that groups … PID, argv, prompts, transcripts, credentials, and absolute paths never cross into the WebView"). 반면 정리(cleanup) 경로의 `ScanItem.path`는 `String`으로 직렬화되어 이미 WebView로 넘어가고(`src-tauri/src/models/scan.rs:86`), FE가 이를 검색 필터에 쓴다(`src/lib/utils/cleanup.ts:23`). 따라서 **삭제 저널(②)이 경로를 담는 것 자체는 README:36 위반이 아니다.** 저널의 실제 리스크는 "새 영속 평문 파일 + 권한 0600 부재 + ⑨ 진단 내보내기가 이를 함께 수집"이다.
2. **`allowDowngrades: false`(tauri.conf.json:68)는 업데이터 설정이 아니다.** `bundle.windows` 하위, 즉 NSIS 인스톨러의 다운그레이드 거부 플래그다. macOS에는 대응물이 없고 `tauri-plugin-updater`는 Cargo.toml 의존성(21-41행)에 **없다**. ⑥ 도입 시 다운그레이드 차단은 별도로 설계해야 하며, 기존 플래그를 근거로 삼으면 안 된다.
3. **Quick Panel의 "Safe 전용"은 프론트엔드에만 있다.** `isQuickCleanEligible`이 `item.risk === 'safe'`로 거르지만(`src/lib/stores/scan.svelte.ts:325-328`), 백엔드 `create_delete_plan`은 Manual만 거부하고 Rebuild를 그대로 받는다(`src-tauri/src/safety/planner.rs:70-81`, `src-tauri/src/commands/cleanup.rs:60`). 즉 ⑫는 "권한 축소"가 아니라 **누락된 백엔드 게이트의 신설**이다. 저장소 자체 규약이 이미 이를 요구한다 — `AGENTS.md:39` "Destructive adapters belong to the main-window capability, not the quick panel."

---

## 기능별 보안 영향

| # | 후보 | 새 공격면 / 실수면 | 필수 설계 조건 | 판정 |
|---|---|---|---|---|
| ① | 파괴 작업 공통 확인 다이얼로그 | 신규 공격면 없음. 위험은 **확인 피로**로 인한 무의식 클릭, 그리고 "확인했다"는 착시. 현재 확인은 FE 한 곳뿐이고(`CleanupReviewDialog.svelte` — StorageView에서만 사용) 확인 시점이 **플랜 생성 이전**이다: `cleanItems`가 확인 후 `tauriCreatePlan`(scan.svelte.ts:452) → 즉시 `tauriExecuteClean`(:457). 사용자가 승인한 목록과 Rust가 실행하는 플랜이 서로 다른 객체다 | (a) 다이얼로그는 FE 선택이 아니라 **백엔드 `PlanPreview`를 렌더**해야 한다(순서를 prepare→표시→confirm→execute로 반전). (b) 플랜에 `stage: Created/Confirmed` + `confirmed_at`을 추가하고 `execute_clean`은 Confirmed 아닌 플랜을 거부. (c) 최상위 티어는 **백엔드 발급 챌린지 문자열** 왕복(`dev_ports`의 force-authorized lease와 동형, SAFETY.md:285-288). (d) `PlanTargetPreview`(models/plan.rs:41-48)에 `path`(~축약)·`last_modified`·`consequence` 추가 — 세 값 모두 `ScanItem`(scan.rs:86,95)과 `CacheMetadata.consequence`(scan.rs:53)에 이미 존재 | **채택** (①은 다른 모든 후보의 전제) |
| ② | 삭제 저널/히스토리 (경로 영속) | 새 영속 평문 파일. `settings_store::save`(settings_store.rs:93-99)와 `diagnostics::log_error`(diagnostics/mod.rs:116-120) 모두 **`set_permissions`/`mode(0o600)`가 프로덕션 코드에 0건** → 다중 사용자 머신에서 타 계정이 저널을 읽는다. 저널 내용 = 사용자의 프로젝트명·툴체인·작업 패턴. 리댁션은 경로에 전혀 적용되지 않는다(경로 문자열은 `SECRET_PATTERNS` 8종 중 어느 것에도 매칭되지 않음, diagnostics/mod.rs:40-59) | (a) 저널 파일 `OpenOptions::mode(0o600)` + 디렉터리 0700, Windows는 DACL을 현재 SID 단독으로. (b) 경로는 `normalized_log_path()`(diagnostics/mod.rs:147-155)와 동일하게 `~/` 축약 저장 — 사용자명 노출 제거. (c) **보존기간 상한(예: 30일 또는 500엔트리) + 자동 삭제**를 코드 상수로. (d) 저널 자체를 ⑨ 진단 내보내기의 기본 포함 대상에서 **제외**(별도 opt-in). (e) 해시화는 반대 — 저널의 목적이 "무엇을 지웠는지 사람이 확인"인데 해시는 그 목적을 없앤다. `~` 축약 + 0600 + 보존기간이 올바른 조합 | **조건부** (0600 선행 필수) |
| ③ | 스캔/클린 취소 | 스캔 취소는 안전. **클린 취소가 위험** — `SafeTreeDeleter`는 bottom-up 언링크라 중간에 끊으면 반쯤 지워진 트리가 남고, 사용자는 "취소했으니 안 지워졌다"고 오해한다. 또 `execute_clean`은 전역 `StorageOperationGate`를 쥐고 있고 이 게이트는 `.expect("poisoned")`(operation_gate.rs:11) — 취소 경로에서 패닉 1회면 재시작까지 모든 스토리지 명령이 죽는다 | (a) 스캔 취소는 `storage_commands.rs:111-152`의 기존 패턴(TTL 15분 + 캡 64 + 성공/취소/에러 전 경로에서 제거)을 `ScanEngine`에 그대로 이식 — 현재 `scanner/engine.rs`에는 취소 토큰이 **없다**. (b) 클린 취소는 **타깃 경계에서만** 확인하고 트리 중간에서는 절대 중단하지 않는다. (c) 취소 결과는 `CleanStatus::Partial`로 보고하고 남은 타깃을 명시(현재 Partial이 `success:true`로 보고되는 결함 선수정 필요). (d) 게이트를 `into_inner()` 복구로 전환 | **채택** (스캔) / **조건부** (클린) |
| ④ | Trash 경유 옵션 | (a) **디스크가 즉시 회수되지 않는다.** `executor.rs:76-79`가 `actual_disk_free_delta`를 실측 보고하므로 "12GB 정리" 옆에 "실제 여유공간 +0"이 뜬다 → 사용자가 다시 정리를 돌린다. (b) `~/.Trash`는 blacklist의 `sensitive_relative` 21개 목록에 **없고**(blacklist.rs:87-110), 홈 하위 미매칭은 :119에서 즉시 `return false` → 향후 어떤 시그니처가 Trash 하위를 가리켜도 차단되지 않는다. (c) Trash는 사용자 콘텐츠 폴더라 "Zenith가 Trash를 비운다"는 기능이 붙는 순간 사용자 파일 삭제 도구가 된다 | (a) 기존 `TrashExecutor`(trash_manager/mod.rs:235-307)와 `trash` crate(Cargo.toml:39)를 재사용 — 이미 대용량 파일·앱 언인스톨에서 검증된 경로다. 새 삭제 원시함수를 만들지 말 것. (b) `~/.Trash`·`%RECYCLE.BIN`을 `sensitive_relative`에 추가하고 :119 early-return 제거. (c) **Trash 비우기 기능은 만들지 않는다.** (d) Trash 경유는 Rebuild/Manual 티어에만 제공하고 Safe 캐시는 직접 삭제 유지 — 아니면 Trash가 캐시로 가득 찬다. (e) 회수 바이트 표기를 "Trash로 이동(비우기 전까지 미회수)"로 분리 | **조건부** |
| ⑤ | 예약/자동 정리 (무인 실행) | **가장 위험한 후보.** (a) ①의 확인 게이트를 정의상 우회한다. (b) `min_age_days` 신선도 재확인은 그 필드를 선언한 시그니처에만 적용된다(executor.rs:200-259) — `ai.claude.cache`, `dev.xcode.derived_data`, `dev.cargo.registry.cache` 등 **대다수 Safe/Rebuild 시그니처는 min_age가 없어** 사용자 부재 중 진행 중인 빌드의 산출물을 그대로 지운다. (c) 전역 게이트를 무인 작업이 수 분간 점유하면 사용자가 연 대시보드가 응답 없이 멈춘다. (d) 백그라운드 스레드는 이미 존재하므로(lib.rs:289,296) 실행 인프라 자체는 새 공격면이 아니다 | (a) **Safe 티어 + `min_age_days` 선언 시그니처로만 한정**. min_age 없는 시그니처는 무인 대상에서 제외(코드 상수로 강제, TOML 설정 아님). (b) 실행 직전 활동 감지 게이트: 대상 트리 최신 mtime이 N분 이내면 스킵. (c) AC 전원 + 유휴(idle) 조건. (d) 무인 실행은 **취소 가능해야 하고**(③ 선행) 결과를 저널(②)에 남겨야 한다. (e) 사용자 확인 없이 실행되므로 `terminate_process_group`·docker prune·모델 삭제는 절대 포함 금지 | **조건부** (①②③ + 무인 전용 티어 상수 전부 선행) |
| ⑥ | 자동 업데이트 (tauri-plugin-updater) | 현재 릴리스에 서명·notarization·provenance가 전무하고(release.yml:152-153,244) 체크섬은 자기 생성·자기 검증이다(release.yml:156-158,285-291). 이 상태로 업데이터를 붙이면 **릴리스 쓰기 권한을 얻은 공격자가 전 사용자에게 자동 배포**할 수 있다. 현재는 사용자가 수동으로 받아야 해서 폭발 반경이 작다. 사전 정정 2 참고 — `allowDowngrades:false`는 이 경로를 보호하지 않는다 | (a) **updater 전용 minisign 키쌍**을 GitHub Actions 시크릿이 아닌 오프라인/HSM 보관, 서명은 릴리스 산출 잡 밖에서. (b) `pubkey`를 `tauri.conf.json`에 하드코딩(WebView가 바꿀 수 없는 위치). (c) `endpoints`는 단일 고정 URL, 리다이렉트 비허용. (d) 버전 다운그레이드를 클라이언트에서 명시 거부(현재 버전 ≥ 원격 버전이면 무시). (e) **⑥은 코드서명(SignPath 완료) 이후로 미룬다** — 서명 없는 자동 업데이트는 서명 없는 수동 배포보다 명백히 나쁘다 | **기각(현 시점)** / SignPath 완료 후 재검토 |
| ⑦ | 시그니처 커스터마이즈 (사용자 TOML) | **"임베드 = 검토됨" 계약을 정면으로 깬다.** SAFETY.md:28이 삭제 실행 조건으로 "Its signature exists in the **embedded** registry"를 명시한다. 실제 위험: `SignatureRegistry::register`는 id로 **조용히 덮어쓴다**(registry.rs:80-82) → 사용자 TOML이 `ai.claude.cache`를 재정의해 `~/.docker`·`~/Downloads`·`~/.netrc`를 가리킬 수 있고, 이들은 blacklist의 홈 하위 early-return(blacklist.rs:119)에 걸려 **차단되지 않는다**. `Signature`에 `deny_unknown_fields`가 없고(models/signature.rs:7) `platforms` 기본값이 빈 배열 = 전 플랫폼 활성(:67-69). 이미 죽은 확장점 `load_from_dir`(registry.rs:55)이 존재해 한 줄만 살리면 활성화된다 | 허용 가능한 최소 표면은 **경로를 전혀 받지 않는 것**이다: (a) **exclusions 추가만** 허용 — 기존 시그니처 id에 제외 패턴을 더하는 것은 항상 보수적(덜 지움). (b) `min_age_days` **증가만** 허용(감소 금지). (c) 시그니처 비활성화는 이미 `excluded_signatures` 설정으로 가능하다(models/settings.rs:110) — 신규 표면 없이 재사용. (d) **금지: 새 id, 새 `paths`, `risk` 하향, `strategy` 변경, `intensive_only` 해제.** (e) 스키마에 `deny_unknown_fields` 추가하고 알 수 없는 키는 파일 전체 거부. "기존 루트 하위 경로만 허용"안은 반대 — 루트 자체가 `~/Library/Caches`처럼 광범위해서 실질 제한이 못 된다 | **조건부** (exclusions/min_age 전용 오버레이만) |
| ⑧ | 알림 센터 (OS 알림에 경로/프로세스명) | 잠금화면·화면공유·발표 중 노출. 다만 **이미 올바른 선례가 있다**: `AgentNotificationPreferences.hide_project_basename`(agent_activity/notifications.rs:73-77)와 "본문에 `/`가 없음"을 검증하는 테스트(:191). AI Control Center 쪽 알림은 `Recommendation.title/message`를 그대로 쓴다(ai_control_center/notifications.rs:36-40) — 이 문자열의 생성지가 늘어나면 통제가 무너진다 | (a) 알림 본문 포맷터를 **한 곳으로 통합**하고 `format_notification`과 같은 화이트리스트 방식(고정 문구 + 치환 슬롯) 유지. 문자열 연결로 페이로드를 만들지 말 것. (b) **경로·프로세스명·PID는 본문에 넣지 않는다.** 필요하면 "3개 항목, 4.2GB" 같은 집계만. (c) 기존 `hide_project_basename`를 알림 전반의 전역 스위치로 승격. (d) `body.contains('/')` 금지 테스트를 신규 포맷터에도 확장 | **조건부** |
| ⑨ | 앱 내 진단 내보내기 / 버그리포트 | 리댁션 커버리지가 실제로 부족하다(diagnostics/mod.rs:40-59): `Authorization: Basic dXNlcjpwYXNz`는 스킴만 가려지고 자격증명이 남고(:51), `github_pat_`·`gho_`·`AKIA`·`AIza`·`npm_`는 무변환, 값 문자셋 `[a-zA-Z0-9_.-]` 때문에 특수문자 비밀번호는 앞부분만 가려지며, `https://user:pass@host`는 미처리. 여기에 정리 실패 시 **절대경로 전체**가 로그에 남는다(cleaner/executor.rs:52-56). 내보내기는 이 파일을 **사용자가 제3자에게 전달**하게 만드는 기능이므로 기존 결함의 영향이 곱해진다 | (a) `sanitize_log`와 `ai_control_center/safety.rs:25-29`의 패턴셋을 **단일 공유 모듈로 통합**하고 Basic auth·신형 PAT·URL 자격증명·따옴표 내 임의 문자열을 추가. (b) executor.rs:52-56의 경로를 `~/` 축약으로 기록. (c) 내보내기 파일 0600 생성. (d) **내보내기 직전 전문 미리보기를 보여주고 사용자가 지울 수 있게** — 리댁션은 최선노력이지 보증이 아니며, README:101의 현재 문구가 보증처럼 읽힌다. (e) 저널(②)은 기본 미포함 | **조건부** (리댁션 통합 + 0600 선행) |
| ⑩ | CLI / 헤드리스 모드 | 현재 인자 파싱이 **전혀 없다**(main.rs 전 6줄). CLI를 붙이는 순간 Tauri capability 계층(`main.json`/`quick.json`)이 **완전히 우회된다** — capability는 WebView 호출만 게이트하지 Rust 내부 호출 경로를 막지 않는다. 즉 quick 패널에서 뺀 권한(⑫)이 CLI로 되돌아온다. 추가로 CI/스크립트에 `--yes`가 박히면 무인 실행(⑤)이 심사 없이 생긴다 | (a) CLI는 **동일한 `SafetyPlanner`·`Blacklist`·`ToctouGuard`를 반드시 경유**하고 독자 삭제 경로를 만들지 않는다. (b) `--path` 류 **임의 경로 인자 금지** — 시그니처 id와 티어만 받는다. (c) `--yes`는 `IsTerminal` 검사 통과 시에만 유효, 비대화형(파이프/CI)에서는 Safe 티어로 강제 하향. (d) `--tier manual` 불가(planner.rs:70이 이미 거부하나 CLI 표면에서도 노출하지 말 것). (e) 프로세스 종료·docker prune·모델 삭제는 CLI 미노출. (f) 모든 CLI 실행을 저널(②)에 기록 | **조건부** (①⑤⑫ 이후, 읽기 전용 서브커맨드부터) |
| ⑪ | 프로젝트별 캐시 뷰 (경로 노출 확대) | 노출 확대 폭이 실제로는 작다 — `ScanItem.path`가 이미 WebView로 가고(scan.rs:86), 개발자 산출물 워크스페이스는 사용자가 피커/명시 등록으로 승인한 루트만 다룬다(`storage_commands.rs:226-235`, `register_developer_home_workspace`). 실질 리스크는 (a) FE가 경로 문자열을 조립해 백엔드로 되돌려 보내는 패턴이 늘어나는 것, (b) 창 공유 중 프로젝트명 노출 | (a) **새 경로 수용 커맨드를 추가하지 않는다.** 뷰는 백엔드 인벤토리의 opaque id로만 동작. (b) 표시 경로는 전부 `~/` 축약. (c) `open_in_terminal`/`show_in_file_manager`(commands/system.rs:241,255)를 이 뷰에서 호출한다면, 두 커맨드가 현재 임의 절대경로를 `canonicalize`만 하고 수용하는 문제를 **먼저** 고쳐야 한다(최근 스캔 항목·등록 워크스페이스 멤버십 검증). (d) 민감 프로젝트명 마스킹 토글(⑧의 `hide_project_basename`와 동일 스위치 재사용) | **채택** (경로 수용 커맨드 무증가 조건) |
| ⑫ | Quick Panel 삭제 권한 축소 | 축소 자체는 순수 개선. 유일한 함정은 "권한만 빼고 백엔드 게이트를 안 만드는 것" — 그러면 메인 창의 동일 결함(FE만 Safe 판정)이 남는다. 현재 quick은 장식 없는 투명 always-on-top 창인데(tauri.conf.json:31-43) 메인과 동일한 삭제 권한을 갖고(quick.json:15-16), 확인 다이얼로그 없이 `selectQuickCleanDefaults()→cleanSelected()` 1클릭으로 실행된다(QuickPanel.svelte:178-184) | 아래 §quick capability 축소안 참조 | **채택 (최우선)** |

---

## 확인 다이얼로그 티어 정책

전제: **FE 확인은 보안 경계가 아니다**(SAFETY.md:3-4가 명시). 백엔드가 강제할 수 있는 것은 ① 플랜의 **티어 상한**, ② **2단계 왕복** 구조, ③ **챌린지 문자열 대조**뿐이다. ③만이 "사람이 실제로 읽었다"를 Rust가 알 수 있는 유일한 수단이며, 이는 `dev_ports`의 force-authorized lease가 이미 쓰는 패턴이다.

| 작업 | 티어 | 확인 방식 | 백엔드 강제 방법 | 근거 (파일:라인) |
|---|---|---|---|---|
| Safe 캐시 정리 (`ai.*.cache`, `dev.go.build`, `container.docker.builder`) | **Safe** | 요약 1줄 + 단일 확인. 항목별 체크 불필요 | `execute_clean`이 `plan.risk.rebuild_count == 0 && manual_count == 0` 검증 후 실행 | `models/risk.rs:31` `is_auto_selectable`, `commands/cleanup.rs:114` |
| Rebuild 캐시 (`dev.cargo.registry.*`, `dev.npm.cache`, `dev.xcode.derived_data`, `ai.cuda.compute_cache`) | **Rebuild** | 항목 목록 + 명시적 확인 버튼. 목록은 백엔드 `PlanPreview` 렌더 | 플랜 `stage` 필드 도입: `prepare_delete_plan` → `confirm_delete_plan(plan_id)` → `execute_clean`. Confirmed 아닌 플랜 거부 | `commands/cleanup.rs:60,104,128`, `models/plan.rs:31-38` |
| Intensive 광역 캐시/로그 (`system.intensive.user_app_caches`, `system.intensive.application_logs`) | **Rebuild로 강등 권고** (현재 `safe`) | 항목별 체크 + 경로·최종수정·왜 안전한지 표시 | TOML의 `risk`를 `rebuild`로 변경 → `is_auto_selectable()` 미통과 → 기본 선택 제외. 현재는 스캔 즉시 자동 선택 | `signatures/system.toml:44,64`, `models/risk.rs:31` |
| `$TMPDIR` 접두사 정리 (`system.developer_temp`) | **Safe 유지** (min_age 3일) | 요약 + 접두사 목록 링크 | 실행 직전 전체 트리 신선도 재확인이 이미 강제됨 | `cleaner/executor.rs:200-259`, `signatures/system.toml:37` |
| `docker image prune -a -f` (`container.docker.unused_images`) | **Rebuild + 타이핑** | 백엔드 발급 챌린지 문구 타이핑 (예: `prune images`) | 챌린지를 Rust가 생성·보관·상수시간 대조. 시그니처 id 화이트리스트로 해당 커맨드만 챌린지 요구 | `signatures/containers.toml:23-29`, `docker/adapter.rs:457-464` |
| `docker volume prune` (`container.docker.unused_volumes`) | **Manual** | 노출하지 않음 (현행 유지) | `strategy = "manual"` → planner가 거부. TOML 한 줄 변경으로 활성화되는 지뢰이므로 코드 레벨 거부도 병행 | `signatures/containers.toml:41-47`, `safety/planner.rs:70,79` |
| 로컬 모델 영구 삭제 (Ollama/HF/LM Studio/MLX) | **Manual + 타이핑** | 모델명 타이핑 + "Trash로 가지 않음" 명시 | `delete_local_model`이 신선 인벤토리에서 재해석 + 챌린지 대조. HF/LM Studio는 Trash 아닌 영구 삭제라 다른 워크플로와 불일치 — 문구로 반드시 고지 | `signatures/models.toml` 전 항목 `manual`, `models_inventory/deleter.rs:82-106` |
| 대용량 파일 / 앱 언인스톨 | **Trash** | 목록 확인 (현행) | `TrashPlan` TTL 300초 + `validate_target` 스코프 검증 (이미 구현) | `trash_manager/mod.rs:16,310-323` |
| 앱 프로세스 강제 종료 | **Manual 2단계** | graceful 시도 → 남아 있을 때만 force 확인 | **현재 강제 장치 없음.** `terminate_process_group(name, force)`가 FE 불리언을 그대로 받는다. dev_ports 패턴(30초 1회성 lease + force 전용 2차 lease)으로 교체 필요 | `commands/system.rs:29`, `dev_ports/store.rs:23,105-115`, SAFETY.md:265-266 |
| 개발 포트 force 해제 | **2단계 (구현 완료)** | 유예 후 2차 확인 | `force_authorized` lease를 유예 경과 후에만 신규 발급 — 참조 구현 | `dev_ports/store.rs:23`, SAFETY.md:285-288 |

**미리보기에 무엇을 보여야 하는가** (현재 `CleanupReviewDialog.svelte:45-46`은 이름·티어·바이트 3개뿐):

1. `~/` 축약 경로 — 사용자가 "그 폴더가 맞나"를 판단하는 유일한 단서. `ScanItem.path`(scan.rs:86)에 이미 있음.
2. **최종 수정 시각** — "3일 전"과 "10분 전"은 완전히 다른 결정. `ScanItem.last_modified`(scan.rs:95)에 이미 있음.
3. **왜 안전한가 / 지우면 무슨 일이 생기는가** — `CacheMetadata.consequence`·`artifact_kind`(scan.rs:49-57)에 이미 있고 화면에는 안 쓰인다.
4. 크기는 `size_semantics`(scan.rs:41-46) 함께 — `ConservativeLowerBound`인 항목에 정확한 숫자처럼 보이는 표기는 오해를 부른다.

즉 **필요한 데이터는 전부 이미 IPC를 건너오고 있고, 다이얼로그가 렌더하지 않을 뿐이다.** 새 데이터 노출 없이 판단 가능성을 크게 올릴 수 있다 — 다만 `PlanTargetPreview`(models/plan.rs:41-48)에는 이 4개가 없으므로, "다이얼로그를 PlanPreview 기반으로 반전"하려면 프리뷰 구조체 확장이 동반되어야 한다.

**확인 피로 방지:** 위 표에서 Safe 티어는 항목별 확인을 요구하지 않는다. 확인 횟수를 늘리는 대신 **티어를 정확히 매기는 것**이 해법이다 — 현재 `system.intensive.user_app_caches`가 `safe`라서 서드파티 캐시가 무확인 자동 선택되는 것(system.toml:41-58)이 확인 피로가 아니라 **오분류** 문제다.

---

## quick capability 축소안

현재 `src-tauri/capabilities/quick.json`은 `core:*` 3개 + `allow-*` 15개. 설명은 "Read-mostly … backend-owned safe cleanup plans"(:4)인데 실제로는 임의 항목 선택 삭제가 가능하다.

**유지 (12개)** — 전부 읽기 전용:
`core:default`, `core:window:allow-hide`, `core:window:allow-start-dragging`, `allow-get-ai-usage`, `allow-get-ai-control-quick-summary`, `allow-get-agent-quick-summary`, `allow-get-last-scan`, `allow-get-memory-metrics`, `allow-get-disk-metrics`, `allow-get-awake-state`, `allow-get-settings`, `allow-get-platform-capabilities`, `allow-open-dashboard-window`, `allow-toggle-quick-panel`, `allow-get-app-version`

**제거 (2개)**:
- `allow-create-delete-plan` (quick.json:15) — 임의 항목 선택 표면. Rebuild 티어 도달 가능.
- `allow-execute-clean` (quick.json:16) — `docker image prune -a -f`(adapter.rs:457-464)와 `npm/pnpm/uv cache prune`까지 도달.

**조건부 유지 (1개)**:
- `allow-start-scan` (quick.json:13) — 패널의 핵심 기능이라 필요하지만(QuickPanel.svelte:125-129), 전역 `StorageOperationGate`(operation_gate.rs:11)를 잡는 무거운 작업이다. 유지하되 ③의 취소 토큰 도입 후 재검토.

**신규 (2개, 쌍으로 동작)**:
```
allow-prepare-quick-safe-clean
allow-execute-quick-safe-clean
```

### 신규 커맨드 시그니처

```rust
/// Quick 패널 전용. FE는 어떤 항목도 선택하지 못하며 scan_id만 제시한다.
/// 선택·티어 판정·플랜 구성이 전부 백엔드 소유다.
#[tauri::command]
#[specta::specta]
pub async fn prepare_quick_safe_clean(
    scan_id: String,
    state: State<'_, AppState>,
) -> Result<PlanPreview, String> {
    // 1. last_scan.scan_id == scan_id 확인 (cleanup.rs:71-78과 동일)
    // 2. 선택은 백엔드가 계산: risk == RiskTier::Safe && exists
    //    && size.reclaimable() > 0 && 설정의 quick-clean 카테고리 활성
    //    (현재 FE의 scan.svelte.ts:325-328 로직을 Rust로 이관)
    // 3. SafetyPlanner::create_plan_from_scan 으로 플랜 생성
    // 4. 사후 단언: plan.risk.rebuild_count == 0 && plan.risk.manual_count == 0
    //    위반 시 즉시 Err — 시그니처 TOML이 바뀌어도 fail closed
    // 5. plan.origin = PlanOrigin::QuickSafe, plan.stage = Created 로 저장
}

/// 준비된 QuickSafe 플랜만 실행한다. 메인 창이 만든 플랜은 거부한다.
#[tauri::command]
#[specta::specta]
pub async fn execute_quick_safe_clean(
    plan_id: uuid::Uuid,
    on_event: Channel<CleanEvent>,
    state: State<'_, AppState>,
) -> Result<CleanResult, String> {
    // plan.origin == PlanOrigin::QuickSafe 아니면 Err
    // plan.stage == Confirmed 아니면 Err (①의 2단계와 동일 게이트)
    // 이후는 execute_clean(cleanup.rs:114-156)과 동일 경로 재사용
}
```

지시문이 제시한 `execute_quick_safe_plan(plan_id)` 단일 커맨드 대신 쌍으로 나눈 이유: 플랜 생성 권한을 quick에서 완전히 빼면 quick이 참조할 `plan_id`가 존재하지 않고, 미리보기 없는 1클릭 삭제가 그대로 남는다. `prepare`가 **선택 인자를 받지 않는다**는 점이 핵심 — FE가 기여하는 값은 `scan_id` 하나뿐이다.

부수 효과: `PlanOrigin`이 생기면 `create_delete_plan`(메인 창)에도 `PlanOrigin::Main`이 붙어, 향후 CLI(⑩)가 제3의 origin으로 들어와도 실행 경로별 정책 분리가 가능해진다.

---

## 선행 필수 보안 수정 (기능 착수 전)

1. **`terminate_process_group` lease화** — `commands/system.rs:29` + `metrics/memory.rs:353-380`. dev_ports 패턴(30초 1회성 lease + force 전용 2차 lease, `dev_ports/store.rs:105-115`)으로 교체하고 kill 직전 uid·start_time·self-PID 재확인. 보호 목록을 `dev_ports/classifier.rs:180-195`와 단일 상수로 통합. → **①의 "graceful 먼저" 전제이자 ⑤⑩에서 이 커맨드를 배제할 근거.**
2. **quick capability 축소 + 백엔드 Safe 티어 강제** — `capabilities/quick.json:15-16` 제거, 위 §의 커맨드 쌍 신설, `plan.risk.rebuild_count/manual_count == 0` 사후 단언. 저장소 자체 규약 위반 해소(`AGENTS.md:39`). → **①⑤⑫의 전제.**
3. **로그·설정·저널·감사 파일 0600 + 리댁션 통합** — `diagnostics/mod.rs:116-120`, `settings_store.rs:93-99`, `ai_control_center/audit.rs`에 `OpenOptions::mode(0o600)` + 디렉터리 0700(Windows는 DACL). `sanitize_log`(diagnostics/mod.rs:40-59)와 `ai_control_center/safety.rs:25-29`를 공유 모듈로 합치고 Basic auth·`github_pat_`·`gho_`·`AKIA`·`AIza`·URL 자격증명 추가. `cleaner/executor.rs:52-56`의 절대경로를 `~/` 축약. → **②⑨의 전제.**
4. **blacklist fail-open 3곳 + 자격증명 규칙** — `safety/blacklist.rs:119` 홈 하위 early-return 제거, `safety/symlink.rs:24`(stat 실패→false)와 `:163-168`(canonicalize 실패→Ok) 차단으로 승격. `sensitive_relative`(blacklist.rs:87-110)에 `.netrc`·`.git-credentials`·`.docker`·`.npmrc`·`Downloads`·`.Trash`·`Library/Mobile Documents`·`Library/CloudStorage` 추가. → **④⑤⑦의 전제** (⑦의 오버레이가 안전한지는 blacklist가 실제로 닫혀 있느냐에 달려 있다).
5. **`ScanEngine` 취소 토큰 + `StorageOperationGate` poison 복구** — `scanner/engine.rs`에 `storage_commands.rs:111-152` 패턴 이식(TTL + 캡 + 전 경로 제거), `operation_gate.rs:11`의 `.expect("poisoned")`를 `into_inner()` 복구로. → **③⑤의 전제이자, 무인 실행이 사용자 작업을 영구 블로킹하지 않게 하는 조건.**

### Windows 패리티의 보안 순서

기능 확장 전 **필수**는 (a)와 (b) 두 가지다.

- **(a) 안전 테스트 이식 — 가장 먼저.** `tests/safety_tests.rs:1`·`tests/dev_ports_tests.rs:1`의 `#![cfg(unix)]` 때문에 삭제-안전 통합 테스트 25개가 Windows에서 컴파일조차 되지 않는데, CI는 `windows-latest`에서 `cargo test`를 초록으로 보고한다(ci.yml:147, release.yml:214). 즉 아래 (b)(c)를 고쳐도 **회귀를 잡을 수단이 없다.** 리스크가 가장 낮고 이후 모든 수정의 게이트이므로 1순위.
- **(b) uid 1000 → SID 검증 — 실제 구멍이므로 기능 확장 전 필수.** `dev_ports/termination.rs:74,148,288,345`가 상수 1000을 반환해 소유자 필터(:375)·타사용자 차단(`classifier.rs:67-75`)·시그널 직전 uid 대조(:553)가 Windows에서 전부 무조건 통과한다. 같은 저장소 `agent_activity/mod.rs:406-422`에 정상 SID 비교 구현이 이미 있으므로 이식만 하면 된다.
- **(c) graceful 분리 — ⑤⑧ 착수 전 필수, 그 외 기능에는 선택.** `termination.rs:189-215`가 graceful/force 양쪽 다 `TerminateProcess`를 호출해 SAFETY.md:284 "SIGTERM only"가 Windows에서 거짓이다. 무인 실행(⑤)이나 알림 기반 액션(⑧)이 붙으면 사용자 부재 중 저장 안 된 작업이 유실된다. 미구현 상태로 둘 거라면 `PlatformCapabilities`에 Unavailable로 명시하고 UI에서 숨겨야 한다.
- **(d) `toctou.rs` `BACKUP_SEMANTICS` — ⑤ 착수 전 필수.** Windows에서 디렉터리 identity가 (0,0)으로 떨어지고 `:108` 가드가 검증을 통째로 스킵한다. 사람이 지켜보는 삭제에서는 오탐 수준이지만, 무인 실행에서는 유일한 안전망이 사라지는 것이다.

---

## 공급망 개선 체크리스트

- [ ] **전 서드파티 액션 커밋 SHA 핀.** 특히 `dtolnay/rust-toolchain@stable`(ci.yml:29,89,126 / release.yml:34,91,179)은 **강제 이동 브랜치**이고, `contents: write` 토큰을 받는 `softprops/action-gh-release@v2`(release.yml:294)와 `taiki-e/install-action@just`(ci.yml:114)도 가변 참조다. `actions/*` 공식 액션 포함 전부 SHA로. 주석에 원래 태그 병기.
- [ ] **Dependabot 활성화** (`.github/dependabot.yml` 부재) — `github-actions`·`cargo`·`npm` 3개 ecosystem. SHA 핀과 세트로만 의미가 있다.
- [ ] **pnpm lifecycle script 차단.** `pnpm install --frozen-lockfile`이 6곳에서 스크립트 허용 상태로 실행된다(ci.yml:51,178,209 / release.yml:54,114,201). `.npmrc`도 `onlyBuiltDependencies`도 없고 pnpm 9 고정이라 기본 차단이 아니다.
  - **주의:** 무조건 `--ignore-scripts`를 붙이면 esbuild 등 네이티브 바이너리 설치가 깨질 수 있다. 권장 경로는 **pnpm 10 + `onlyBuiltDependencies: ["esbuild"]`** (필요 패키지만 화이트리스트). pnpm 9를 유지해야 한다면 `--ignore-scripts` 적용 후 `pnpm build`가 통과하는지 먼저 확인.
- [ ] **SLSA build provenance.** `release-mac`(release.yml:132-158 이후)과 `release-windows`(:220-250 이후) 각 잡에 `actions/attest-build-provenance@<sha>`를 추가하고, 해당 잡에만 `permissions: id-token: write, attestations: write`를 부여(현재 릴리스 워크플로 기본은 `contents: read`, release.yml:9 — 좋은 기본값이므로 유지하고 잡 단위로만 승격). 사용자 검증 절차는 `gh attestation verify <파일> --repo jaeyoung0509/zenith`.
- [ ] **체크섬 자기검증 고리 인지.** release.yml:291의 `shasum -a 256 -c SHA256SUMS.txt`는 **같은 잡이 :285-290에서 합친 매니페스트로 자기 산출물을 검증**하는 동어반복이다. 제거할 필요는 없지만(다운로드 손상 탐지에는 유효) 이것을 변조 방지로 문서화해서는 안 된다 — 아래 README 문구 참조.
- [ ] **SignPath 진행 중 임시 조치 3가지.** ① 릴리스 노트에 커밋 SHA와 워크플로 실행 URL 병기(현재 BUILD_INFO에 커밋만 있음, release.yml:150,242) ② provenance attestation 선도입(코드서명보다 먼저 가능하고 서명 도착 후에도 유효) ③ `scripts/install_release_app.sh:104-113`의 `validate_bundle`에 서명 도착 전까지는 릴리스 SHA256과의 대조를, 도착 후에는 `codesign --verify --deep --strict` + `spctl --assess`를 필수 게이트로.
- [ ] **`cargo audit` / `cargo deny` + `pnpm audit --audit-level=high`를 ci.yml에 추가.** 현재 의존성 취약점 게이트가 0건이다.
- [ ] **`xattr -cr` 안내 재검토** (README.md:95). 사용자에게 Gatekeeper 검역을 스스로 제거하라고 가르치는 것은 서명 도입 이후에도 남으면 안 되는 습관이다. 서명·notarization 완료 시 이 줄을 삭제한다는 것을 CODE_SIGNING_POLICY.md에 명시.

### README 문구 수정 1건

**현재** (README.md:71-73):
> Confirm that the installer came from this repository's GitHub Release and verify its SHA256 value against `SHA256SUMS.txt` before choosing **More info → Run anyway**.

**수정안:**
> Confirm that the installer came from this repository's GitHub Release, then compare its SHA256 value against `SHA256SUMS.txt` before choosing **More info → Run anyway**. Note that `SHA256SUMS.txt` is produced by the same release run that built the installer, so a matching checksum only proves the download was not corrupted or truncated — it is not tamper-evidence, and it cannot detect a replaced release asset. Until code signing and build provenance are in place, treat every Zenith installer as unverified regardless of a matching checksum.

(한국어 요지: "이 체크섬은 다운로드 손상 확인용이며, 릴리스 자산 자체가 교체된 경우는 탐지하지 못한다"를 명시.)

---

## 총평

12개 후보 중 순수 개선은 ⑫ 하나뿐이고, 나머지는 전부 "선행 수정이 끝났다는 조건 하에" 안전하다 — 그리고 그 선행 수정 5건은 이미 1차 리뷰에서 나온 항목과 정확히 겹친다.
가장 위험한 조합은 **⑤(무인) + ⑦(사용자 TOML)**이다. 둘 다 "사람이 검토했다"는 Zenith의 유일한 실질 방어를 제거하며, 동시에 도입되면 blacklist 하나에 모든 것이 걸리는데 그 blacklist는 지금 홈 하위에서 기본 허용이다(blacklist.rs:119).
⑥(자동 업데이트)은 현 시점 유일한 **기각** 항목이다 — 서명·provenance 없는 상태의 자동 배포는 지금의 수동 배포보다 폭발 반경이 명백히 크고, `allowDowngrades:false`는 이 경로를 전혀 보호하지 않는다.
①(확인 다이얼로그)의 핵심은 확인을 늘리는 게 아니라 **티어를 정확히 매기고**(`system.intensive.*`의 `risk = "safe"`가 오분류) **확인 시점을 플랜 이후로 옮기는 것**이다 — 현재는 승인 대상과 실행 대상이 서로 다른 객체다.
착수 순서 권고: ⑫ → 선행수정 1·3·4 → ① → ③ → ②⑧⑪ → ⑨④ → (Windows 패리티 후) ⑤⑩ → (서명 후) ⑥⑦.
