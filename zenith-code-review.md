# 종합 코드 리뷰 리포트 — jaeyoung0509/zenith v0.3.8

범위: 저장소 전체(develop, shallow clone 2026-09-09). Tauri 2 + Rust ~30k줄 + Svelte 5 ~16k줄.
차원 6개 병렬: 코드리뷰(code.md) · 아키텍처(architecture.md) · 비관적사고(pessimist.md) · 보안(security.md) · 데이터(data.md) · 운영체제(os.md). 원본은 같은 폴더.
같은 파일:라인을 여러 차원이 지적하면 한 항목으로 묶고 차원 태그 병기. 라인번호는 각 에이전트 실측값.

---

## 🔴 Critical (12)

1. **[보안][아키텍처][비관][OS] `commands/system.rs:29` + `metrics/memory.rs:353-380` — `terminate_process_group(name, force)`**
   FE가 준 이름 문자열로 전체 프로세스 테이블을 훑어 동명 프로세스 전부에 신호. lease·uid·start_time·self-PID 검증 없음, `force=true`면 즉시 SIGKILL. 보호 목록(:275-304)은 리터럴 이름뿐이라 iTerm2/Ghostty/Alacritty/kitty/WezTerm/Warp 미보호 → 터미널 SIGKILL 시 PTY 하위 셸·에이전트 연쇄 SIGHUP. dev_ports/agent_activity가 갖춘 lease 모델과 정반대. 파괴적 커맨드 중 "Rust가 결정 소유" 원칙을 깨는 유일한 지점.
   → dev_ports 패턴(30초 1회성 lease + force 2단계)으로 통일, kill 직전 uid/start_time/self 재확인, 보호 목록 단일 상수(classifier.rs:180-195와 병합).

2. **[비관][보안] `routes/quick/QuickPanel.svelte:178-184` + `capabilities/quick.json:15-17` — 확인 없는 1클릭 영구 삭제**
   Quick Panel "Clean"이 `selectQuickCleanDefaults()→cleanSelected()`로 플랜 생성·실행 연속 호출. `CleanupReviewDialog`("cannot be undone")는 StorageView 한 곳만 사용. quick capability는 "read-mostly"라 해놓고 `create-delete-plan`·`execute-clean` 부여. always-on-top 투명 창에 메인과 동일 삭제 권한.
   → 영구 삭제 경로 전부 ReviewDialog 경유 강제. quick엔 백엔드 생성 Safe 전용 플랜만 실행하는 별도 커맨드.

3. **[아키텍처][코드][비관] `operation_gate.rs:11` — 전역 `Mutex<()>` `.expect("poisoned")`**
   스캔·클린·대용량·앱인벤토리·휴지통 전부 직렬화하는 게이트. 임계구역 안 수 분짜리 FS I/O 중 패닉 1회 → 영구 poison → 재시작까지 모든 스토리지 명령 패닉. 문서 "fail closed"가 아니라 영구 장애. 저장소 전체 poison 정책 혼재(`expect` 88곳 / `into_inner` 35곳 / `map_err` 1곳, `commands/ai.rs:85·110·275` 같은 락 3정책).
   → 상태 없는 게이트는 `into_inner()` 복구 또는 `try_lock`. 락별 정책을 타입 래퍼로 고정.

4. **[OS][보안] `safety/blacklist.rs:48,114,143,170` — 대소문자 구분 비교**
   APFS 기본·NTFS 모두 case-insensitive인데 블랙리스트 전부 exact 비교. `~/documents`, `~/.SSH`, `C:/windows/...` 통과. (보안 차원은 현재 진입 경로가 `$HOME`·TOML·`read_dir`이라 실측 도달 미확인 "추정"으로 Minor 분류 — 병기.)
   → 플랫폼별 케이스 폴딩 후 비교, 변형 케이스 단위 테스트.

5. **[비관][보안] `safety/blacklist.rs:85-121` — 홈 하위 기본 허용(early return)**
   `sensitive_relative` 21개에 없으면 :119 `return false`로 즉시 허용, 이후 검사도 스킵. 누락: `Library/Mobile Documents`(iCloud Drive 실체), `Library/CloudStorage`, `Dropbox`, `Downloads`, `.Trash`, `.netrc`, `.git-credentials`, `.docker/config.json`, `.npmrc`, `.config`(gcloud 제외). iCloud "데스크탑·문서" 동기화 켜면 `~/Desktop`→`Mobile Documents/...`라 symlink.rs:163 canonical 재검사도 무력. Windows OneDrive KFM은 하드코딩 `home.join("Documents")`와 불일치(:97-110 vs paths.rs:125-149 SHGetKnownFolderPath).
   → allowlist(시그니처 루트 하위만)로 반전, known-folder API로 보호 목록 생성, 자격증명 파일 추가.

6. **[OS][보안] `safety/tree_deleter.rs:159,210` — dirfd 보유하고도 경로 재해석 삭제**
   `prepare_directory`(:231)가 `O_DIRECTORY|O_NOFOLLOW` fd 확보했는데 실제 삭제는 `fs::remove_file(path)`/`remove_dir(path)`. identity 검증(:154)→unlink 사이 상위 컴포넌트 심링크 교체 TOCTOU 창.
   → `unlinkat(dirfd, name, 0|AT_REMOVEDIR)`로 전환.

7. **[데이터][비관] `safety/toctou.rs:36,108,132` — 디렉터리 TOCTOU 사실상 no-op**
   (a) Windows `capture()`가 `File::open`으로 디렉터리 열기 → `BACKUP_SEMANTICS` 없어 항상 실패 → identity (0,0) → :108 가드로 검증 스킵. (b) 전 플랫폼: 디렉터리는 dev/ino만 비교, mtime/size 스킵(:132) → 플랜 후 300초 내 빌드 시작해도 inode 동일하면 통과, 빌드 중 DerivedData/target 삭제. `min_age_days` 없는 타깃은 실행 직전 신선도 재확인 자체 없음(executor.rs:200-259는 min_age만).
   → `BACKUP_SEMANTICS|OPEN_REPARSE_POINT`로 열고 (0,0)은 실패 처리. 디렉터리도 mtime 대조, 전 타깃 실행 직전 최근변경 재확인.

8. **[OS][보안][비관] `dev_ports/termination.rs:74,148,288,345` — Windows uid 상수 1000**
   `current_uid()`·프로세스 uid 모두 하드코딩 → 소유자 필터(:375)·타사용자 차단(classifier.rs:67)·시그널 직전 uid 대조(:553) 전부 무조건 통과. SYSTEM/타계정 리스너가 종료 후보. 같은 저장소 `agent_activity/mod.rs:406`은 SID 비교 정상 구현.
   → SID 대조로 통일.

9. **[OS][보안][비관] `dev_ports/termination.rs:189-215` + `agent_activity/termination.rs:178` — Windows graceful 부재**
   dev_ports: signal 15/9 모두 `TerminateProcess`(차이는 종료코드). agent: `send_sigterm`이 `cfg(unix)` 전용, Windows는 `Err` 반환 → 에이전트 정지 기능 공백. SAFETY.md "SIGTERM only" 위반.
   → `CTRL_BREAK_EVENT`/`WM_CLOSE` graceful 구현, 실패 시만 승격. 미구현이면 capabilities에 Unavailable 명시.

10. **[OS] `tooling.rs:195,222,233,243` — reap 후 `kill(-pid, SIGKILL)`**
    `try_wait`가 `Some(status)` 반환(이미 reap) 후 `cleanup_and_drain(true)`가 PGID kill. reap된 PID 즉시 재사용 가능 → 무관 프로세스 그룹 SIGKILL. 정상 종료마다 실행되는 경로.
    → 정상 경로 `kill_tree=false`, 타임아웃 경로는 `wait()` 전에 그룹 kill.

11. **[보안][비관] `ai_usage/mod.rs:434-518` — OAuth state 없음 + 콜백 무검증**
    인가 URL에 `state` 없음(:448-452). 콜백 리스너가 경로·메서드·Origin 검증 없이 첫 연결의 `code` 추출(:470-476) → 180초 창 동안 임의 웹페이지/로컬 프로세스가 `127.0.0.1:<port>/?code=<공격자>` → 공격자 OpenRouter 키 저장(PKCE로 못 막음). 부수: 바인드 `127.0.0.1` vs 콜백 URL `localhost`(::1 선점 위험), 토큰 교환 리다이렉트 10회 기본(:502), code 없는 첫 연결이면 즉시 Err(DoS).
    → 128비트 state 발급·상수시간 대조, `/callback` GET 한정, 콜백 URL 127.0.0.1 고정, `redirect::Policy::none()`.

12. **[코드][비관] `tests/safety_tests.rs:1` + `tests/dev_ports_tests.rs:1` — `#![cfg(unix)]`로 Windows 안전 테스트 0건**
    25개 삭제-안전 통합 테스트 Windows 미컴파일. ci.yml:146·release.yml:214 windows-latest `cargo test` 초록 → NSIS 배포. blacklist.rs:225-311 Windows 전용 verbatim/UNC/ADS 로직 자동 테스트 0건. docs/WINDOWS.md:107 "same checks on windows-latest" 거짓.
    → Windows 픽스처 이식, 그전까지 문구 철회.

---

## 🟠 Major (요약, 영역별)

### 삭제 경계·시그니처
- [보안] `safety/symlink.rs:24` `is_symlink` stat 실패→`false`(안전 판정), `:163-168` canonicalize 실패→`Ok(())`. fail-open 2곳, SAFETY.md:298 "fail closed" 위반.
- [OS] `safety/tree_deleter.rs:294,341` Windows `prepare_directory` no-op, `validate_entry_owner` unix 전용 → Windows에 소유자·identity·READONLY 해제 부재. `\\?\` 미부착으로 260자 초과 삭제 실패, SHARING_VIOLATION 재시도 없음.
- [비관] `safety/tree_deleter.rs:407-448` 엔트리마다 canonicalize 3회, 전역 게이트 쥔 채 단일 스레드, `execute_clean` 취소 커맨드 없음 → 응답 없는 앱 강제 종료 = 반쯤 지운 캐시. [코드] `scanner/engine.rs:15`도 취소 토큰 없음.
- [코드][비관] `cleaner/executor.rs:319-333` Partial을 `success:true, failure_reason:None`, `total_failed_bytes` 미계상. `:336` 실패 분류가 영어 문자열 매칭(비영어 로케일 전부 Unknown).
- [코드][데이터] `signatures/system.toml:9` `$TMPDIR`+`${TEMP}` 동일 확장, dedup 없음(walker.rs:164 idx별 id) → temp 회수량 2배, 플랜 중복. [데이터] `signatures/ai.toml:50` macOS에서 `${ROAMING_APP_DATA}`=리터럴 경로 중복 4개.
- [비관][데이터] `signatures/system.toml:3-38` `system.developer_temp` 기본 스캔·safe·자동선택, 판정은 `$TMPDIR` 직계 자식 이름 접두사+mtime만(`claude-`, `pytest-of-`…). 사람이 만든 동명 디렉터리 3일 방치 시 `delete_directory`. [보안] `:41-58` `intensive.user_app_caches` risk=safe → 자동선택.
- [데이터][비관] `scanner/size.rs:194` vs `tree_deleter.rs:450` exclusion 매칭 규칙 불일치(contains vs exact/prefix), walker.rs:287은 확장 안 함 → "크기에서 제외"≠"삭제 안 함".
- [코드] `models/signature.rs:8` `deny_unknown_fields` 없음(`platforms` 오타 시 전 플랫폼 활성). `signatures/registry.rs:81` id 중복 조용히 덮어씀. [아키텍처][비관][데이터] `registry.rs:55` `load_from_dir` 미사용 dead 확장점, 살아나면 임의 TOML→삭제 루트.
- [보안] `safety/planner.rs:95-99` min_age 시그니처에서 `path == root` 허용(워커만 막음). `:54` `create_plan` pub 노출.
- [OS][비관] `safety/blacklist.rs:78` ADS 콜론 검사가 전 플랫폼 → macOS 합법 ':' 파일명 차단·바이트 오차.

### 프로세스·포트
- [코드][OS][비관] `agent_activity/termination.rs:228` 주석 "ancestry check"지만 `parent_pid` 수집 후 미사용. `:150` 보호 판정 `contains("sh")` → ssh/flash/*-shim 영구 정지 불가.
- [비관] `agent_activity/mod.rs:149-163` exe basename만으로 `can_stop:true` → 동명 개인 스크립트 Stop 대상. `commands/ai.rs:118-131` kill()==0이면 즉시 Ok, 사후 확인 없음.
- [보안][비관] `dev_ports/classifier.rs:349-397` argv 부분문자열 매칭(`nodemon`/`uvicorn`/`webpack`&&`serve`), 보호 판정은 완전일치 → 비대칭. gunicorn 워커 오탐.
- [비관] `dev_ports/termination.rs:518-532` 리스너 미발견을 시그널 전인데 `Released` 보고. `:594-671` 유예 1.5초 후 force 유도. [코드][OS] `:600` 폴링 15회마다 lsof fork(2초 타임아웃), discovery 1회 실패 시 조기 Err.
- [OS] `dev_ports/discovery.rs:116` `::ffff:127.0.0.1`·`fe80::` → Network 오분류. `:86` lsof `+c 0` 없어 command 9자 절단.
- [OS][코드] `metrics/memory.rs:354` 호출마다 `System::new_all()`+`refresh_processes` 이중 전체 스캔. `power/watcher.rs:261,450` 규칙 활성 시 5초마다 전체 프로세스 스캔(Keep Awake가 배터리 소모). `:452` 무한 스레드 안 `.unwrap()`.
- [OS] `platform/system_actions.rs:31~112` spawn 후 `wait()` 없음 → 좀비 누적. `diagnostics/mod.rs:219` Windows에서도 `open` 실행 → 항상 실패.

### IPC·경계·아키텍처
- [보안] `commands/system.rs:241,255,267` `open_in_terminal(path)`/`show_in_file_manager(path)` FE 임의 경로 수용(canonicalize만). [아키텍처] `LargeFilesView.svelte:541` FE가 `${display_parent}/${name}` 경로 조립(`/` 하드코딩).
- [보안][비관] `agent_activity/events.rs:138` `post_agent_event.cwd`로 `git -C <cwd> status`, `ai_control_center/git.rs:341-373` `--no-ext-diff`/`--no-textconv`/`fsmonitor=`/`GIT_CONFIG_NOSYSTEM` 없음 → 공격자 `.git/config` 저장소면 RCE(압축파일·공유볼륨 전제). `:407-432` untracked 파일 전문 diff → `commands/ai.rs:854`로 WebView 반환(.env 평문 유출).
- [보안] `tooling.rs:37-41,601` 상속 PATH 우선 + `~/.cargo/bin` 등 후보, 실행파일 검증 없음(cache_providers `validate_executable`과 비일관). [OS] resolve 캐시 없음.
- [아키텍처] `commands/state.rs:12` AppState 23필드 god object. `storage_commands.rs:2↔state.rs:23`, `platform/paths.rs:80↔safety/blacklist.rs:26` 순환. `agent_activity/mod.rs:41` GLOBAL_STORE 전역 싱글턴. `ai_control_center/runtime.rs:33,91,123` Arc 9개 + `AppHandle` 직접 참조 + std::thread 안 `block_on`. `tooling.rs`(cfg 29개) platform 밖, `models/platform.rs:226`이 능력 판정 소유.
- [아키텍처] `build.rs:2`·`lib.rs:325`·`main.json`·`quick.json` 커맨드 명단 4곳 수동, CI는 bindings drift만 검증. `execution_budget.rs:150` `acquire_storage_read` 호출 0, `acquire_subprocess` 1곳(스폰 47곳) — 과설계.
- [아키텍처][데이터] 설정 RMW 3곳 복제(`system.rs:183`, `ai.rs:527,655`), 두 WebView last-write-wins, sanitize 경로 갈림. FE `settings.svelte.ts:118-122` 폴백 revision 3(백엔드 5) → 마이그레이션 되감김.

### 데이터·영속
- [데이터] `settings_store.rs:98` fsync 없음·temp명 고정. `models/settings.rs:100` unknown 필드 미보존 → 다운그레이드 시 영구 소실. `ai_control_center/audit.rs:20,27` 512KB 초과/파싱 실패 시 전량 무음 폐기, `commands/ai.rs:504` save 오류 `let _`.
- [데이터] `cleaner/executor.rs:81` 삭제 내역 영속 기록 없음(실패만 로그). [보안][비관] `:52-56` 실패 시 절대경로 로그 → diagnostics:210 → WebView(README "absolute paths never cross" 반례). 로그·설정·감사 파일 0600 지정 0건.
- [데이터][OS][비관] `scanner/size.rs:48` `blocks()*512` 단순 합산, nlink/APFS clone 미고려. `docker/adapter.rs:283` GB를 1024³(Docker는 SI) → 7.4% 과대. `models_inventory/scanner.rs:108` Ollama 공유 레이어 중복 합산. `ai_control_center/budgets.rs:59` 기간 필터 없음·live+manual 이중 계상.
- [비관] `docker/adapter.rs:457-463` `image prune -a -f` 로컬 전용 이미지 복구 불가. `:481-488` volume prune 분기 코드 존재(TOML 한 줄 지뢰). `models_inventory/deleter.rs:96-101` HF/LM Studio 모델 Trash 아닌 영구 삭제(다른 워크플로와 불일치).
- [보안][데이터] `agent_activity/hooks.rs:172-179` 3rd-party 설정 재작성 시 umask 권한(0600 강등), 키 순서 재정렬.

### OS 지표
- [OS] `metrics/memory.rs:122` used/total 휴리스틱 → 상시 Warning 오탐(`kern.memorystatus_vm_pressure_level` 미사용). `:388` `vm_stat` 텍스트 파싱. `metrics/disk.rs:36` APFS 컨테이너 공유·purgeable 미반영. `power/assertion.rs:59` `PreventUserIdleSystemSleep`만(클램셸 미방어). `tauri.conf.json` bundle.macOS 부재(entitlements·hardened runtime·minimumSystemVersion).

### 보안 리댁션·공급망
- [보안][코드] `diagnostics/mod.rs:43-59` Basic auth·`github_pat_`·`gho_`·`AKIA`·`AIza`·URL 자격증명 미탐지, 특수문자 비번 부분 노출. `:63` 패턴 하드코딩 인덱스. `ai_control_center/safety.rs:156` `.env`/`.env.local` 스캔 제외.
- [보안] `release.yml:156,285` 자기 생성 체크섬으로 자기 검증(README:72는 변조 방지처럼 안내). 액션 전부 가변 태그(`softprops/action-gh-release@v2` contents:write). `pnpm install` lifecycle script 허용. `install_release_app.sh:104` codesign 검증 없음. [비관] `xattr -cr` 공식 안내 + updater 부재 + 단일 스쿼시 커밋(bus factor 1).

### FE·빌드
- [코드] `stores/scan.svelte.ts:374` in-flight dedupe가 인자 무시(카테고리 스캔→전체 결과). `ApplicationsView.svelte:149`·`LargeFilesView.svelte:192` 세대 토큰 없음 → stale 응답이 휴지통 대상 덮어씀. `QuickPanel.svelte:106/137` polling refcount 비대칭 → 숨은 창 3초 IPC 영구.
- [코드] `Justfile:117` cargo check가 dist/ 없이 실패, fmt/clippy 게이트 CI만. `scripts/tauri_release_build.sh:7` `-e` 없어 `mv` 실패 무시.

---

## 🟡 Minor (포인터)
- 코드: executor.rs:98 결과 조립 17회 중복 · developer_artifacts/mod.rs:766 recognize_* 14벌 · walker.rs:233 도달불가 분기 · cleanup.rs:65 TTL 상수 3곳 · utils/tauri.ts 무동작 위임 60개 · scan.svelte.ts:167 집계 5벌 비메모이즈 · native.ts:77 `as any` · vite.config.ts 타입체크 제외
- 아키텍처: bindings `location_hint` vs `display_path` 자기모순 · 스토리지 3뷰 스토어 부재 · lib.rs 트레이/윈도우 혼재·detach 스레드 · `CleanStrategy` FE 죽은 계약 · storage_commands cancel 6메서드 복제
- 데이터: `storage_commands.rs:155` `saturating_sub` TTL fail-open(scan.rs는 checked_sub) · `projects.rs:188` salt 없는 opaque_id · `format.ts:5` 1024진수+SI 라벨 · mock.ts 소수 바이트 · events.rs:82 ms/s 단위 미검증
- OS: blacklist에 `Downloads` 없음 · `OpenProcess` 실패 대부분 "종료됨" 처리 · `GetExtendedTcpTable` 버퍼 경쟁 재시도 없음 · launchd KeepAlive 재기동 미감지 · HOME 환경변수만 의존
- 보안: system_actions.rs Windows 쉘 상대경로 · toolchain 액션 브랜치 참조

---

## 차원별 요약

| 차원 | Critical | Major | Minor | 원본 |
|---|---|---|---|---|
| 코드리뷰 | 1 | 20 | 15 | code.md |
| 아키텍처 | 2 | 15 | 7 | architecture.md |
| 비관적사고 | 10 | 19 | 6 | pessimist.md |
| 보안 | 2 | 22 | 7 | security.md |
| 데이터 | 2 | 16 | 12 | data.md |
| 운영체제 | 4 | 20 | 7 | os.md |
| **병합 후** | **12** | 약 55 | 약 30 | — |

교차 지적(3차원 이상): `terminate_process_group`(4) · Windows uid 1000(3) · Windows TerminateProcess(3) · `agent_activity` ancestry 미구현(3) · operation_gate poison(3) · blacklist 홈 하위 허용(2+) · quick 1클릭 삭제(2) · `$TMPDIR` 중복(2) · exclusion 불일치(2).

---

## 총평

파일시스템 삭제 코어(O_NOFOLLOW dirfd·dev/ino 재검증·1회성 플랜·`remove_dir_all` 0건·lease 기반 포트 해제)는 1인 프로젝트로는 이례적으로 견고하고, 6차원 모두 이 점에 동의한다.
결함은 그 코어 **바깥과 Unix 바깥**에 몰려 있다. (1) 프로세스 종료 구현 4벌 중 `terminate_process_group`만 lease 없이 이름 매칭 SIGKILL, (2) Windows는 uid 위조·graceful 부재·안전 테스트 0건·TOCTOU (0,0)으로 문서 보증이 통째로 비어 있고, (3) 사용자가 실제 데이터를 잃는 지점은 확인 없는 Quick Clean·이름 접두사 판정·디렉터리 mtime 미검증·Partial=성공 보고 등 코어 위 계층이다.
문서의 never/always 단정 중 검증 결과 "미지킴/부분"이 절반을 넘는다(fail closed·force 전 normal·리댁션·absolute path·Windows 동등 검증). 미서명 배포 + updater 부재 + 스쿼시 히스토리 조합에서 사고 시 회수 수단이 없다.

**우선 조치 3가지**
1. `terminate_process_group`을 dev_ports lease 모델로 교체 + 보호 목록 단일화 + Quick Panel 삭제 경로에 확인 다이얼로그 강제(quick capability 축소).
2. Windows 안전 공백 일괄 해소: uid→SID, `TerminateProcess` graceful 분리, `toctou.rs` 디렉터리 핸들, `safety_tests` Windows 이식(그전까지 Windows 릴리스 문구 수정).
3. 삭제 경계 fail-open 3곳(`symlink.rs:24,163`, `blacklist.rs:119` early-return) + 대소문자 폴딩 + `unlinkat` 전환 + `operation_gate` poison 복구.


---

# 부록 — 코드리뷰 차원 원본 (code.md)

## Zenith 코드 리뷰 — 정확성·재사용·단순화·효율·일관성

대상: `zenith` (Tauri 2 + Rust ~30k / Svelte 5 + TS ~16k). 경로는 저장소 루트 기준.

## Critical

Critical | src-tauri/src/operation_gate.rs:11 | `StorageOperationGate::run`이 `Mutex<()>`를 `.expect("storage operation gate poisoned")`로 잠근다 — 게이트 안에서 도는 스캔·클린·대용량파일·아티팩트·언인스톨·휴지통 중 하나라도 패닉하면 락이 영구 오염돼 이후 모든 스토리지 명령이 프로세스 수명 내내 패닉한다. 보호하는 데이터가 없는 순수 직렬화 락이므로 `unwrap_or_else(|p| p.into_inner())`로 복구하라.

## Major

Major | src-tauri/src/dev_ports/termination.rs:600 | 시그널 전송(:592) 이후 15회 폴링 루프가 `system.discover_listeners()?`로 조기 반환한다 — 폴링 도중 lsof가 한 번만 타임아웃해도 SIGTERM이 이미 전달된 상태에서 release_listener가 Err를 돌려주고 리스는 이미 소비된 뒤라, 죽어가는 프로세스를 실패로 표시한다. 폴링 단계의 discovery 실패는 "판정 불가"로 삼아 루프를 계속하고 :626 사후 검사로 흘려보내라.

Major | src-tauri/src/dev_ports/termination.rs:510 | 릴리스 1회에 `lsof -nP -a -iTCP`가 최대 17번(사전 1 + 폴링 15 + 사후 1) 기동되고 각각 2초 타임아웃을 가진다. 폴링 구간은 `kill(pid, 0)`이나 단일 프로세스 조회 같은 저비용 생존 확인으로 바꾸고, 전체 lsof discovery는 최종 확정에만 쓰라.

Major | src-tauri/src/agent_activity/termination.rs:150 | 보호 프로세스 판정이 `name_lower.contains(p)` 부분일치이고 목록에 `"sh"`가 있어, 이름에 sh가 든 모든 프로세스(`ssh`, `flash`, `*-shim`, `Cursor.sh`)가 보호 터미널로 분류돼 영영 정지할 수 없다. 같은 함수의 실행파일 분기(:158)는 `eq_ignore_ascii_case` 완전일치라 기준도 서로 다르다. 두 분기 모두 전체 이름 일치로 통일하라.

Major | src-tauri/src/agent_activity/termination.rs:228 | 주석은 "Terminal and protected **ancestry** check"라고 선언하지만 `is_terminal_or_protected`는 대상 프로세스만 본다 — 수집해 둔 `ProcessCheckInfo.parent_pid`(:78, :125)는 테스트 밖에서 한 번도 읽히지 않아, 보호 터미널 아래에서 뜬 CLI가 그대로 통과한다. 부모 체인을 실제로 순회하거나 주석과 필드를 제거해 보장 범위를 정직하게 맞춰라.

Major | src-tauri/src/commands/ai.rs:85 | 같은 `global_store()` 뮤텍스를 한 파일 안에서 세 가지 정책으로 잠근다 — `.expect(...)`(:85), `unwrap_or_else(into_inner)`(:110), 맨 `.unwrap()`(:275, 그리고 agent_activity/mod.rs:108). 오염 시 어떤 경로로 들어왔느냐에 따라 복구되기도 패닉하기도 한다. 스토어별로 정책을 하나 정해 헬퍼로 감싸라.

Major | src-tauri/src/commands/cleanup.rs:93 | docs/ARCHITECTURE.md:49는 "plans"를 poison recovery 대상으로 명시하는데 `delete_plans`는 :93·:127에서 `.expect("delete_plans poisoned")`로 fail closed다(`last_scan`도 :42·:55·:73·:137 동일). 문서화된 정책과 코드가 어긋나 있으니 둘 중 하나를 고쳐 단일 기준으로 맞춰라.

Major | src-tauri/src/power/watcher.rs:452 | 무한 백그라운드 스레드 안의 `wait_while(...).unwrap()` — 다섯 줄 위(:441~:447)는 같은 종류의 락을 `.expect()`로 다루는데 여기만 맨 unwrap이고, 오염되면 워처 스레드가 조용히 죽어 Keep Awake가 오류 표시 없이 영구 정지한다. 오염 복구 후 계속 대기하도록 바꿔라.

Major | src-tauri/tests/safety_tests.rs:1 | `#![cfg(unix)]`가 837줄 안전성 스위트(blacklist·planner·executor·symlink) 전체를 Windows에서 비활성화한다 — `.github/workflows/ci.yml:146`의 "Rust Unit & Safety Tests" 스텝은 Windows에서 안전성 테스트 0건을 돌리고 초록으로 통과한다. blacklist.rs:225-310의 Windows 전용 로직(verbatim `\\?\`, ADS, 드라이브 루트)은 인라인 테스트도 없어 완전 무검증이다. Windows에서 돌 수 있는 케이스를 분리하라(tests/dev_ports_tests.rs:1 동일).

Major | src-tauri/src/scanner/engine.rs:15 | `ScanEngine::scan`에는 취소 토큰이 전혀 없다(대용량파일·아티팩트 스캔은 storage_commands.rs:304·340에 취소 명령 보유). 게다가 배타적 `StorageOperationGate` 안에서 돌아서, 느린 전체 스캔이 다른 모든 스토리지 명령을 중단 불가 상태로 막는다. 다른 스캐너와 같은 `AtomicBool` 취소 핸들을 붙여라.

Major | src-tauri/src/cleaner/executor.rs:336 | 실패 사유 분류를 `io::Error` 문자열의 영어 부분일치("Permission denied", "No such file")로 한다 — 비영어 로케일에서는 모든 실패가 `CleanFailureReason::Unknown`으로 뭉개져 UI가 원인을 못 알린다. `TreeDeleteReport.errors`에 포맷된 문자열 대신 `io::ErrorKind`를 실어 나르라.

Major | src-tauri/src/cleaner/executor.rs:321 | 부분 성공을 `success: true`로 두면서 `total_failed_bytes`(:50) 누적에서도 빠뜨린다 — 대부분의 파일이 잠겨 회수 실패한 실행도 요약 수준에서는 완전 성공으로 보고된다. 미회수분을 실패 바이트로 계상하라.

Major | signatures/system.toml:9 | `paths = ["$TMPDIR", "${TEMP}"]`인데 platform/paths.rs:27이 두 플레이스홀더를 같은 `temp_dir()`로 해석하고 scanner/walker.rs:31 경로 루프에 dedup이 없다. walker.rs:164가 id를 `…0.<name>` / `…1.<name>`로 갈라 만들어 중복 제거도 안 되므로, 전 플랫폼에서 temp 회수 가능 용량이 2배로 표시되고 플랜에 같은 경로가 두 번 실린다. 하나만 남기거나 `resolve_paths`에서 정규화 후 중복 제거하라.

Major | src-tauri/src/models/signature.rs:8 | `Signature`에 `#[serde(deny_unknown_fields)]`가 없고 loader.rs:22-27은 빈 id만 거른다 — `platforms`를 `platform`으로 오타 내면 조용히 무시돼 Windows 전용 시그니처가 전 플랫폼에서 활성화되는 식으로 삭제 범위가 넓어져도 빌드·테스트가 못 잡는다. `deny_unknown_fields`를 붙이고 임베드 TOML 5개를 엄격 파싱하는 테스트를 추가하라.

Major | src-tauri/src/signatures/registry.rs:81 | `register()`가 `HashMap::insert`라 id 중복 시 앞 시그니처가 조용히 덮이고 load_from_dir:69는 그래도 count를 올린다. 현재 49개 id는 유일하지만 회귀를 막는 장치가 없으니 `load_embedded`에서 중복 id를 에러로 반환하라.

Major | src/lib/stores/scan.svelte.ts:374 | `runScan`이 in-flight `scanRequest`만 보고 반환해 `categories` 인자를 무시한다 — 자동 전체 스캔(:145) 진행 중 사용자가 특정 카테고리 스캔을 누르면 전혀 다른 스캔 결과를 받고 `lastScanTrigger`도 'auto'로 덮인다. 인자 키별 dedupe로 바꾸거나 인자가 다르면 큐잉하라(usage.svelte.ts:104·agentActivity.svelte.ts:88의 `force` 무시, aiControl.svelte.ts:26의 force 폐기도 같은 결함군).

Major | src/routes/dashboard/ApplicationsView.svelte:149 | `inspectApp`에 세대 토큰이 없어 A→B 연속 클릭 시 늦게 도착한 A의 응답이 `inspection`/`selectedRelatedIds`를 덮어써 B 화면에 A의 관련 항목이 뜬다 — 휴지통 대상이 뒤바뀌는 파괴적 오작동이다. 로컬 generation 변수로 stale 응답을 폐기하라.

Major | src/routes/dashboard/LargeFilesView.svelte:192 | `scanFiles`에 재진입 가드·세대 토큰이 없고 :443 "Scan again" 버튼이 `isScanning`으로 비활성화되지 않아, 먼저 시작한 스캔의 `handleScanEvent`가 새 스캔의 `items`에 계속 push되고 늦게 끝난 쪽이 결과를 덮어쓴다. 진입부 early-return과 세대 토큰을 함께 두라.

Major | src/routes/quick/QuickPanel.svelte:106 | `startPolling` 조건은 `hasSection('memory') && memoryAvailable`인데 `stopPolling`(:137) 조건은 `hasSection('memory')`뿐이라 refcount가 비대칭이다 — 패널이 열린 사이 memory 섹션을 끄면 해제가 스킵돼 3초 setInterval이 숨겨진 창에서 영구히 IPC를 때린다. 실제 구독 여부를 로컬 플래그로 기록해 그 플래그만 보고 해제하라.

Major | Justfile:117 | `check`가 `cargo check`(:118) → `pnpm build`(:120) 순인데 lib.rs:308 `generate_context!`가 요구하는 `dist/`는 .gitignore 대상이라, 클린 체크아웃이나 `just clean`(:183) 직후엔 cargo check가 먼저 실패한다. CI는 ci.yml:55·99·138에서 매번 `mkdir -p dist`를 하지만 Justfile은 generate-bindings(:96)에만 있다. check·test-rust 앞에도 넣어라.

Major | Justfile:101 | `check`·`test` 어디에도 `cargo fmt --check`와 `cargo clippy --all-targets -- -D warnings`가 없는데 CI는 ci.yml:102·105·141·144에서 이를 게이트로 건다 — 로컬은 통과하는데 CI만 깨지는 구조다. lint 레시피를 만들어 check·test 의존에 포함하라(AGENTS.md:16의 필수 명령 목록도 같은 누락).

Major | scripts/tauri_release_build.sh:7 | `set -uo pipefail`만 있고 `-e`가 없어 :42의 파괴적 `mv "$package_dir" "$repair_dir/"` 실패가 검사되지 않는다 — mv가 실패해도 `cargo fetch`와 전체 재빌드를 그대로 진행한 뒤 원래 실패만 보고해, 진짜 원인이 수십 분짜리 재빌드 뒤에 가려진다. `|| exit "$build_status"`로 감싸라.

## Minor

Minor | src-tauri/src/cleaner/executor.rs:98 | `CleanItemResult`를 :102~:345에서 17번 손으로 조립하며 `item_id`/`name`/`path`를 매번 `target`에서 복사한다. `fail(target, reason, msg)` / `ok(target, bytes)` 헬퍼 둘이면 357줄 중 150줄가량이 사라진다.

Minor | src-tauri/src/developer_artifacts/mod.rs:766 | `recognize_*` 14개(:766~:1160)가 마커 파일명·ecosystem·kind·rebuild hint만 다른 사실상 동일 함수다. 정적 테이블 + 제네릭 리졸버 하나로 약 400줄이 접히고, 1979줄짜리 이 파일도 recognizer/measurement/workspace 서브모듈로 갈라진다.

Minor | src-tauri/src/metrics/memory.rs:354 | `System::new_all()`이 이미 전체 갱신을 수행하는데 바로 다음 줄에서 `refresh_processes(ProcessesToUpdate::All, true)`를 또 호출해, 종료 요청마다 전체 프로세스를 두 번 훑는다. 둘 중 하나를 지워라.

Minor | src-tauri/src/dev_ports/termination.rs:518 | `found_listener`는 오직 `debug_assert_eq!(found_listener.pid, lease.pid)`(:533)에 쓰이는데 이 단언은 :511 `find` 술어상 항상 참이다 — 바인딩과 단언 모두 죽은 코드다.

Minor | src-tauri/src/scanner/walker.rs:233 | `match stats.newest_mtime { Some(existing) => … }`가 갓 초기화된 `stats` 위에서 실행돼 `Some` 분기가 도달 불가다. 실제 누적은 :301에서만 일어나므로 :233은 단순 대입으로 줄여라.

Minor | src-tauri/src/diagnostics/mod.rs:63 | `sanitize_log`가 `SECRET_PATTERNS`를 하드코딩 인덱스(`0..4`, 이후 `[4]`,`[5]`,`[6]`,`[7]`)로 접근한다 — 패턴을 하나 끼워 넣으면 치환 규칙이 조용히 어긋난다. "전체 치환"과 "캡처 보존 치환" 두 배열로 이름 지어 나눠라.

Minor | src-tauri/src/commands/cleanup.rs:65 | `const PLAN_TTL_SECS: u64 = 300`이 `create_delete_plan`(:65)과 `execute_clean`(:119)에 각각 선언돼 있고, models/scan.rs:145 `VALID_FOR_SECONDS = 300`과도 손으로 동기화해야 한다. 한 곳으로 올려라(commands/ai.rs:300·:378의 스냅샷 신선도 `< 10` 리터럴 중복도 동일).

Minor | src-tauri/src/safety/planner.rs:54 | `create_plan`이 `pub`이지만 프로덕션 호출처는 `create_plan_from_scan`(:48) 하나뿐이고, 직접 부르면 신뢰 스캔 검증을 우회한 채 `scan_id: String::new()`인 실행 불가 플랜을 만든다. `pub(crate)`로 좁혀라. 참고로 :40의 `trusted_items.len() != requested.len()`은 개수 비교라, 카테고리 간 id 중복이 있으면 개수는 맞는데 다른 요청 id가 조용히 빠질 수 있다.

Minor | src/lib/utils/tauri.ts:1 | 파일 전체(약 60개 함수)가 `api.X()`를 그대로 되돌려주는 무동작 래퍼이고 실제 로직은 `tauriStartWindowDrag` 하나뿐이다 — bindings → api/(native|mock|storage) → utils/tauri 3단 중 가운데 층이 값을 더하지 않는다. 화면에서 `api`/`storageApi`를 직접 쓰고 이 층을 제거하라(호출부가 전부 이 층만 거치므로 치환은 기계적이다).

Minor | src/lib/stores/scan.svelte.ts:167 | `reclaimableBytes` 외 4개 집계(:167~:228)가 risk 조건만 다른 동일 이중 루프 5벌이고, 클래스 getter라 `$derived`와 달리 메모이즈되지 않아 StorageView·QuickPanel이 렌더마다 전체 항목을 5회 재순회한다. `#sumSelected(pred)` 하나로 합치고 `$derived.by`로 감싸라.

Minor | src/lib/stores/settings.svelte.ts:118 | `performLoad`의 `fetched.X ?? 기본값` 폴백은 `ZenithSettings_Serialize`의 모든 필드가 필수라 실행되지 않는 죽은 코드이고, 그 기본값(:119-123, revision 3)이 생성자 기본값(:17-21, revision 5)과 다르다. 폴백을 지우고 기본 설정 상수 하나만 두라.

Minor | src/routes/dashboard/DeveloperArtifactsView.svelte:223 | `artifact_found`마다 `items.some(...)` 선형 탐색 + `[...items, x]` 전체 복사로 O(n²)다. 같은 스트리밍 문제를 LargeFilesView가 :55-57에서 Set + push로 이미 풀어 뒀으니 그 패턴을 그대로 쓰라.

Minor | src/lib/api/native.ts:77 | 49개 커맨드 중 `postAgentEvent`만 `event as any`로 캐스팅해 `IngestedAgentEvent_Deserialize` 계약 검증을 통째로 무력화한다. models/types.ts의 별칭을 Deserialize 형태로 맞추거나 변환 함수를 두고 캐스팅을 지워라.

Minor | vite.config.ts:21 | Vitest 설정을 `'vite'`의 `defineConfig`에 넣는데, tsconfig.json:18 `include`가 `src/**`만 잡아 vite.config.ts와 scripts/*.mjs|cjs는 `pnpm check`에서도 CI에서도 타입체크되지 않는다. `'vitest/config'`에서 임포트하고 tsconfig.node.json으로 설정·스크립트를 체크 대상에 포함시켜라.

Minor | scripts/verify_tailwind_build.mjs:5 | `readdirSync(dist/assets)`를 존재 확인 없이 호출해, 빌드가 중단된 상태에서는 :7의 의도된 "could not find a generated CSS asset" 대신 raw ENOENT 스택이 실패 메시지로 나온다. try/catch로 같은 메시지에 수렴시켜라.

### 총평

머지 가능하다 — 안전성 설계(리스·TOCTOU 재검증·블랙리스트 3중 검사)의 골격은 견고하고, 발견은 대부분 그 골격 주변의 일관성 결함이다.
가장 먼저 고칠 것은 poison 정책 통일이다. 특히 `operation_gate.rs:11`은 패닉 한 번으로 파괴적 워크플로 전부를 앱 수명 내내 죽이는 단일 지점이고, 같은 뮤텍스를 세 가지로 잠그는 `commands/ai.rs`가 그 정책 부재를 그대로 보여준다.
두 번째는 검증 공백이다. Windows CI의 "Safety Tests"가 `#![cfg(unix)]`로 0건을 돌리며 초록인 점과, `signatures` 스키마가 오타를 조용히 삼키는 점은 삭제 범위와 직결된다.
프론트엔드는 스토어 4곳·화면 2곳에 세대 토큰이 없어 stale 응답이 최신 상태를 덮는 동일 결함이 반복된다 — 개별 수정보다 공용 요청 세대 유틸 하나로 묶는 편이 낫다.
`$TMPDIR`/`${TEMP}` 중복(system.toml:9)은 사용자에게 보이는 수치가 2배로 틀리는 유일한 즉시 체감 버그라 우선순위를 올릴 만하다.

### 잘된 점

- IPC 계약 드리프트를 CI가 실제로 막는다 — ci.yml:53-59가 매 실행마다 바인딩을 재생성하고 `git diff --exit-code`로 검증하며, mock.ts는 `satisfies ZenithApi` + apiContract.test.ts로 native와 키 집합 동일성이 강제된다. 흔히 썩는 지점인데 자동화돼 있다.
- `execution_budget.rs`가 풀·세마포어 상한과 "permit → pool" 획득 순서까지 모듈 주석으로 명문화해 중첩 데드락을 설계로 배제했고, 왜 그 수치인지(기존 피크를 넘기지 않음)까지 근거를 남겼다.
- 프로세스 종료·포트 해제 경로가 `DevPortSystem`/`TerminationSystem` 트레이트로 추상화돼, 실제 프로세스를 죽이지 않고 PID 재사용·소유권 변경·시그널 거부 시나리오를 결정론적으로 테스트한다.


---

# 부록 — 아키텍처 차원 원본 (architecture.md)

## Zenith 아키텍처 리뷰 (읽기 전용)

대상: `zenith` (Tauri 2 + Rust ~30k / Svelte 5 + TS ~16k), 브랜치 `develop`
차원: 아키텍처(모듈 경계·의존 방향·상태 관리·플랫폼 추상화·FE 계층·확장성)

## 발견 목록

Critical | src-tauri/src/operation_gate.rs:11 | 전체 스캔·클린·대용량파일·앱인벤토리·휴지통실행을 직렬화하는 단일 전역 `Mutex<()>`를 `.expect("storage operation gate poisoned")`로 잠근다. 임계구역 안에서 수 분짜리 파일시스템 I/O가 돌기 때문에 스캐너·실행기 어디서든 패닉이 한 번 나면 게이트가 영구 poison 되고, 이후 모든 스토리지 워크플로가 재시작 전까지 패닉한다(문서가 말하는 "fail closed"가 아니라 영구 장애). 상태가 없는 `Mutex<()>`이므로 `unwrap_or_else(|p| p.into_inner())`로 복구하거나, `try_lock` 기반 "다른 작업 진행 중" 오류 반환으로 바꿔라.

Critical | src-tauri/src/commands/system.rs:29 | `terminate_process_group(name: String, force: bool)` — FE가 프로세스 그룹 "이름 문자열"과 종료 "전략(force)"을 직접 지정하고 백엔드는 이름이 일치하는 모든 프로세스에 SIGKILL/SIGTERM을 보낸다(metrics/memory.rs:353-372). 백엔드 권위는 하드코딩 차단목록(memory.rs:275-308)뿐이며 opaque ID·리스·PID 재사용 검증이 없다 — `dev_ports`/`agent_activity`가 이미 갖춘 리스 모델과 정반대다. ARCHITECTURE.md의 "FE는 opaque ID만 전달, Rust가 모든 파괴적 결정 소유" 주장이 깨지는 유일한 파괴적 커맨드이므로, 메모리 스냅샷이 발급한 일회용 리스 ID를 받도록 `release_development_listener` 패턴으로 통일하라.

Major | src-tauri/src/commands/state.rs:12 | `AppState`가 23개 필드(레지스트리·설정·스캔·플랜·OpenRouter키·싱글플라이트 2종·제너레이션 2종·스토리지·메모리·devport·에이전트캐시·AI컨트롤 3종·플랫폼·메트릭·예산)를 한 구조체에 담은 god object다. 모든 커맨드가 모든 도메인 락에 접근 가능해 경계가 컴파일러로 강제되지 않는다. Tauri는 `manage()`를 여러 번 호출할 수 있으니 `CleanupState`/`StorageState`/`AiState`/`SystemState`로 쪼개고 각 핸들러가 필요한 것만 주입받게 하라.

Major | src-tauri/src/storage_commands.rs:2 ↔ src-tauri/src/commands/state.rs:23 | `storage_commands`가 `crate::commands::AppState`를 import 하고, `commands::AppState`가 다시 `crate::storage_commands::StorageWorkflowState`를 필드로 갖는 모듈 순환이다. 게다가 IPC 경계가 `commands/`와 최상위 `storage_commands.rs` 두 곳으로 갈라져 문서의 "commands는 좁은 일반 IPC 경계" 서술과 어긋난다. `storage_commands`를 `commands/storage.rs`로 옮기고 `StorageWorkflowState`는 `large_files`/`applications`/`trash_manager`가 공유하는 도메인 모듈로 내려라.

Major | src-tauri/src/platform/paths.rs:80 ↔ src-tauri/src/safety/blacklist.rs:26 | 최하위 두 레이어가 서로를 참조한다(`platform::paths` → `safety::Blacklist::normalize_path`, `safety::blacklist`/`symlink` → `platform::NativePlatformPaths`). 경로 정규화가 안전정책 모듈에 얹혀 있어 의존 방향이 없다. `normalize_path`(verbatim `\\?\` 처리 등 순수 문자열 정규화)를 `platform`으로 내려 `safety → platform` 단방향으로 만들어라.

Major | src-tauri/src/tooling.rs:1 | 1,021줄·`target_os` 조건부 29개로 저장소 전체에서 플랫폼 분기가 가장 밀집한 모듈인데 `platform/` 밖 최상위에 있다. 반대로 `platform/capabilities.rs`는 19줄짜리 위임 껍데기이고 실제 능력 판정은 `models/platform.rs:226`에 있다(모델 레이어가 플랫폼 정책을 소유). 문서의 "platform이 런타임 플랫폼 계약을 소유" 주장과 코드 배치가 반대다. CLI 탐색·프로세스 스폰·CREATE_NO_WINDOW를 `platform/process.rs`로, 능력 판정을 `platform/capabilities.rs`로 옮겨라.

Major | src-tauri/src/commands/ai.rs:85 | 같은 `agent_activity::global_store()` 락에 대해 :85는 `expect("agent activity store poisoned")`(패닉), :110은 `unwrap_or_else(|p| p.into_inner())`(복구)로 정책이 갈린다. 저장소 전체로도 `expect(...poisoned)` 88곳 / `into_inner()` 복구 35곳 / 진짜 fail-closed(`map_err`) 1곳으로 섞여 있고, 분기선이 문서가 말한 "스토리지=복구, 그 외=fail closed"와 일치하지 않는다. 락별 정책을 타입으로 고정하라(예: `PoisonRecoverable<T>` / `FailClosed<T>` 래퍼) — 주석 규약으로는 유지되지 않는다.

Major | src-tauri/src/agent_activity/mod.rs:41 | `GLOBAL_STORE: OnceLock<Arc<Mutex<AgentActivityStore>>>` — 정지 리스와 알림 dedupe 필터가 `AppState` 밖 프로세스 전역 싱글턴에 산다. 주입 지점도 리셋 수단도 없어 이 상태를 건드리는 커맨드는 통합 테스트에서 격리 불가능하고, 문서가 말하는 "AppState 단일 권위"에도 예외를 만든다. `AppState`(또는 분리된 `AiState`)의 필드로 옮겨 다른 스토어와 동일한 생명주기에 두어라.

Major | src-tauri/src/ai_control_center/runtime.rs:91 | 도메인 런타임이 `tick(&self, app_handle: Option<&AppHandle>)`로 Tauri 타입을 직접 받고, :123에서 `tauri::async_runtime::block_on(...)`을 `lib.rs:296`의 순수 `std::thread` 루프 안에서 호출한다. 도메인 → 프레임워크 역참조 + 런타임 혼용이라 정책 평가 로직을 Tauri 없이 테스트할 수 없다. 알림 방출을 `trait AdvisorySink`로 추상화해 `tick`이 `Vec<Recommendation>`만 반환하고 커맨드/부트스트랩 층이 `AppHandle`을 붙이게 하라.

Major | src-tauri/src/ai_control_center/runtime.rs:33 | `AiControlRuntime::new`이 memory·devport·에이전트캐시·싱글플라이트·제너레이션·메트릭·컨트롤상태·awake·settings 등 Arc 9개를 받는다. 사실상 두 번째 `AppState`이며 AI 컨트롤 정책이 앱의 거의 모든 가변 상태에 결합돼 있다. 정책 평가에 필요한 값은 호출 시점 스냅샷 구조체(`ControlInputs`)로 넘기고, 런타임은 수집기 트레이트 몇 개만 보유하게 줄여라.

Major | src-tauri/src/commands/system.rs:183 · src-tauri/src/commands/ai.rs:527 · src-tauri/src/commands/ai.rs:655 | 설정 파일 read-modify-write가 3개 커맨드에 각각 복제돼 있고 모두 "락에서 clone → 파일 저장 → 다시 락 잡고 통째로 덮어쓰기" 패턴이다. clone과 재-락 사이의 다른 창(main/quick 두 WebView)의 변경은 조용히 유실된다. 검증도 갈린다 — `save_settings`는 `settings.sanitize()`, `save_ai_control_preferences`는 `budgets::sanitize()`만 탄다. `SettingsStore::update(|s| ...)` 형태의 단일 소유자(락 안에서 mutate → 원자적 저장)로 통합하라.

Major | src/routes/dashboard/LargeFilesView.svelte:541 | 프론트엔드가 `${item.display_parent}/${item.name}`로 파일시스템 경로를 문자열 조립해 `show_in_file_manager`에 넘긴다. "Svelte는 raw filesystem operation을 구성하지 않는다"는 아키텍처 원칙의 정면 위반이며, `/` 하드코딩이라 Windows에서도 깨진다. 백엔드가 이미 갖고 있는 인벤토리 항목 ID로 reveal 하는 커맨드(`reveal_large_file_item(scan_id, item_id)`)를 추가하라.

Major | src-tauri/src/commands/ai.rs:139 | 커맨드 계층이 8개 에이전트 툴 목록을 `const TOOLS`로 하드코딩하는데, 이미 `agent_activity/adapters.rs:12`의 `ADAPTERS` 테이블이 동일 집합을 소유한다(순서만 다름). 9번째 어댑터를 추가하면 통합 설정 화면에서 조용히 누락된다. `ADAPTERS.iter().map(|a| a.id)`로 파생시켜 단일 소스로 만들어라.

Major | src-tauri/src/models/settings.rs:375 | 프로바이더 ID의 진실 소스가 4곳으로 갈라져 있다 — Rust `SUPPORTED_PROVIDERS[5]`(:375)·`SUPPORTED_ACCOUNT_PROVIDERS[7]`(:381), FE `AiProviderId` 유니온(src/lib/models/types.ts:148, 7개, 퀵패널/계정 구분 없음), FE 옵션 배열 2개(src/routes/dashboard/SettingsView.svelte:54,61). 와이어 타입은 `Vec<String>`이라 컴파일러가 아무것도 잡지 못하고 불일치는 런타임 sanitize에서 조용히 삭제된다. Rust에 `ProviderId` enum(+ specta)을 도입해 바인딩으로 내보내라.

Major | src-tauri/src/build.rs:2 | 커맨드 명단이 build.rs `COMMANDS`(59개), lib.rs:325 `collect_commands!`, capabilities/main.json, capabilities/quick.json 4곳에 손으로 유지된다. CI는 바인딩 드리프트(`.github/workflows/ci.yml:59`)만 검사하고 이 4개 목록 간 정합성 검사는 없어, 등록 누락은 빌드가 아니라 사용자가 버튼을 누른 순간 권한 거부로 드러난다. `collect_commands!` 결과를 기준으로 build.rs·capabilities를 대조하는 테스트를 추가하라.

Major | src-tauri/src/execution_budget.rs:150 | 297줄 예산 모듈에서 `acquire_storage_read`는 호출자가 0이고 `acquire_subprocess`는 호출자가 1곳(commands/ai.rs:356)뿐인데, 서브프로세스를 띄우는 모듈은 12개(git·cache_providers·docker·developer_artifacts·power·metrics·dev_ports 등 47개 호출지점)다. 모듈 문서가 약속한 "thread/process 압력 경계"가 실제로는 거의 강제되지 않는 과설계다. 예산 취득을 `tooling`의 스폰 헬퍼 안으로 내려 전 경로가 자동으로 통과하게 하거나, 쓰이지 않는 축을 제거하라.

Major | src-tauri/src/metrics/memory.rs:353 | 프로세스 시그널링 구현이 4개로 흩어져 있고 안전 모델이 제각각이다 — `metrics/memory.rs:353`(이름 차단목록만), `dev_ports/termination.rs:175,207`(리스+PID재사용+TOCTOU 검증), `agent_activity/termination.rs:169`(리스+시작시각 검증), `tooling.rs:195,344,377`(자식 프로세스 그룹 SIGKILL). `platform/mod.rs:13` 주석은 "프로세스 생명주기 provider는 담당 이슈에서 도입"이라고 예고하지만 끝내 도입되지 않았다. 검증된 종료를 단일 `ProcessLifecycle` 서비스로 모아라.

Minor | src/lib/utils/tauri.ts:53 | `bindings/tauri.ts`(생성물) → `api/native.ts`+`api/storage.ts` → `api/index.ts`(native/mock 스위치) → `utils/tauri.ts` → stores/routes 로 IPC 경계가 4겹이고, 최상단 `utils/tauri.ts` 326줄은 전부 `api.x()` 한 줄 위임이다. 실제 단일 경계는 `api/index.ts`이며, 문서가 말하는 "utils/tauri.ts가 단일 invoke 경계 / api/storage.ts는 예외"는 코드와 다르다(어떤 스토어·라우트도 `api/*`를 직접 import 하지 않는다 — 예외 자체가 이미 사라졌다). 이름만 바꾸는 위임 계층을 제거하고 stores가 `api`를 직접 쓰게 하라.

Minor | src/lib/bindings/tauri.ts:1365 | `ProjectIdentity`가 "절대경로가 아닌 힌트"라고 명시된 `location_hint`(:1365)와 실제 경로인 `display_path`(:1367)를 동시에 노출하고, 화면은 후자만 쓴다(src/routes/dashboard/ProjectDetailPanel.svelte:133,142). 안전한 필드가 장식이 되고 계약이 자기모순이다. `display_path`를 main 창 전용 커맨드 응답으로 분리하든지 `location_hint`를 지워 계약을 하나로 만들어라.

Minor | src/routes/dashboard/DeveloperArtifactsView.svelte:1 | Large Files(561줄, IPC 10회)·Applications(527줄, 10회)·Developer Artifacts(590줄, 14회) 세 스토리지 워크플로만 스토어 없이 라우트 컴포넌트가 스캔 진행·취소·선택·플랜 프리뷰·실행 상태머신을 통째로 들고 있다. 나머지 9개 도메인은 모두 `lib/stores/*.svelte.ts`를 갖는다("stores: Svelte 상태와 생명주기 오케스트레이션"이라는 문서 서술과 불일치). 세 워크플로의 스토어를 추가해 뷰를 렌더링으로 되돌려라.

Minor | src-tauri/src/signatures/registry.rs:55 | `load_from_dir`는 호출자가 없는 죽은 확장 지점이며, 살아나면 "시그니처는 검토되어 바이너리에 임베드된다"는 안전 계약을 우회해 디스크의 임의 TOML을 등록하게 된다. 또한 새 카테고리 TOML 추가 시 registry.rs:6-10의 `include_str!` 상수 5개와 :36 배열을 함께 고쳐야 한다. 미사용 로더를 제거하고 임베드 목록은 디렉터리 기반 `include_dir`/빌드스크립트 생성으로 바꿔라.

Minor | src-tauri/src/storage_commands.rs:84 | `register_/remove_/*_cancel_signal` 6개 메서드가 large_file용과 developer_artifact용으로 문자 그대로 복제돼 있고, `StorageWorkflowState`는 독립 Mutex 8개를 든다. 취소 레지스트리를 `(kind, scan_id)` 키 하나로 합치면 워크플로 추가 비용이 필드 2개+메서드 3개에서 0으로 준다.

Minor | src-tauri/src/lib.rs:289 | `lib.rs` 427줄에 모듈 선언·AppState 조립·트레이 메뉴·퀵패널 좌표 계산·윈도우 생성·specta 등록이 섞여 있고, :289와 :296의 무한 `std::thread::spawn` 루프는 종료 핸들이 없어 앱 종료 시 detach 된다. 트레이/윈도우 배치는 `window/`(또는 `shell/`) 모듈로, 백그라운드 루프는 취소 토큰을 가진 `Supervisor`로 분리하라.

Minor | src/lib/models/types.ts:157 | `CleanStrategy` 5개 리터럴 유니온이 FE에 손으로 적혀 있는데 사용처가 0이고, Rust `models/clean_strategy.rs:5`의 동일 enum은 specta 타입이지만 어떤 커맨드도 노출하지 않아 바인딩에 나오지 않는다. 죽은 중복 계약이므로 삭제하라.

### 의존 방향 요약

```text
[ 의도한 방향 ]                        [ 실제 ]

  commands                              commands ──────┐
     |                                     |  ▲        │ (AppState)
     v                                     v  │        v
  domain (scanner/safety/...)          storage_commands ┘   ← 순환
     |                                     |
     v                                  domain: ai_control_center ─→ tauri::AppHandle
  platform                                       │  agent_activity ─→ GLOBAL_STORE(전역)
     |                                           │  ai_snapshots ─→ storage_commands (역참조)
     v                                           v
  models                                platform ⇄ safety          ← 순환
                                        tooling(29 cfg, 최상위)     ← platform 밖
                                        models/platform.rs         ← 모델이 플랫폼 정책 소유

FE:  bindings(생성) → api/native+api/storage → api/index → utils/tauri → stores → routes
     (문서: "utils/tauri = 단일 invoke 경계" / 실제: api/index가 경계, utils/tauri는 위임 4겹째)
     예외: LargeFiles/Applications/DeveloperArtifacts 라우트는 스토어 없이 utils/tauri 직접 호출
```

프로덕션 순환 3건: `commands ⇄ storage_commands`, `platform ⇄ safety`, (테스트 한정) `safety ⇄ developer_artifacts`·`large_files ⇄ developer_artifacts`.
`FileIdentity`가 `large_files` 안에 살면서 `applications`·`developer_artifacts`·`trash_manager`가 이를 import 하므로, 공용 모델이 형제 도메인에 얹혀 3개 도메인을 묶는다 → `models/`로 이동 권장.

### 문서-코드 불일치 목록

| # | 문서 주장 | 코드 실제 | 근거 |
|---|---|---|---|
| 1 | "FE는 opaque ID만 전달, Rust가 파괴적 결정 소유" | `terminate_process_group(name, force)`는 이름 문자열 + 종료 전략을 FE가 지정 | commands/system.rs:29, metrics/memory.rs:353 |
| 2 | "DeletePlan, **경로**, 전략, 파일시스템 identity는 Rust-private" | 플랜·전략은 실제로 private이나 `ScanItem.path`·`LocalModelItem.path`·`DevelopmentListener.working_directory`는 전부 FE로 나감 | bindings/tauri.ts:1642, :1140, :765 |
| 3 | "Svelte는 raw filesystem operation을 구성하지 않는다" | FE가 `${display_parent}/${name}`로 경로를 조립해 백엔드에 전달 | LargeFilesView.svelte:541 |
| 4 | "정규 경로는 backend-only, opaque ID와 parent/name 힌트만 IPC를 건넌다" | 같은 `ProjectIdentity`에 `display_path`(실경로)가 함께 있고 화면은 그것만 사용 | bindings/tauri.ts:1367, ProjectDetailPanel.svelte:133 |
| 5 | "PID는 IPC를 건너지 않는다" | agent_activity에서는 사실이나 `DevelopmentListener.pid`·`ProcessMemory.pid/pids`는 노출 | bindings/tauri.ts:765, :1333 |
| 6 | "스토리지 상태는 poison recovery, **그 외 도메인 락은 fail closed**" | fail-closed(`map_err`)는 1곳뿐, 나머지는 `expect()` 패닉 88곳 — 실패 시 닫히는 게 아니라 영구 고장 | operation_gate.rs:11, commands/ai.rs:85 vs :110 |
| 7 | "AppState / 워크플로 상태가 공유 권위" | 에이전트 정지 리스·알림 필터는 AppState 밖 프로세스 전역 싱글턴 | agent_activity/mod.rs:41 |
| 8 | "플랫폼 분기는 route/도메인에 퍼지지 않고 Rust 서비스 경계 뒤에 선택된다" | `target_os` 169회가 21개 파일에 분산, 최대 밀집은 `platform/` 밖 `tooling.rs`(29회) | tooling.rs, safety/blacklist.rs(13), power/assertion.rs(10) |
| 9 | "`platform`이 런타임 플랫폼 계약을 소유" | 능력 판정 본체는 `models/platform.rs:226`, `platform/capabilities.rs`는 19줄 위임 껍데기 | platform/capabilities.rs:15 |
| 10 | "`utils/tauri.ts`가 모든 Tauri 커맨드의 단일 invoke 경계, `api/storage.ts`가 인가된 예외" | 실제 invoke는 생성 바인딩에서만 일어나고 native/mock 스위치는 `api/index.ts`. `api/storage.ts`를 직접 쓰는 소비자는 없어 "예외" 자체가 소멸 | api/index.ts:10, utils/tauri.ts:1-2 |
| 11 | "커맨드 추가는 lib.rs·build.rs·capability 3곳" | 실제로는 lib.rs·build.rs·main.json·quick.json·bindings·api/native·api/mock·models/types·utils/tauri = 9곳, CI는 bindings 드리프트만 검증 | build.rs:2, lib.rs:325, ci.yml:59 |
| 12 | "Vendor hooks는 검증된 이벤트 브릿지가 생길 때까지 비활성" (일치) | `ADAPTERS` 8개 전부 `integration_available: false` — 문서대로다. 다만 `setup/remove_agent_integration` 커맨드와 hooks.rs 223줄은 활성 경로 없이 존재 | adapters.rs:12, commands/ai.rs:162 |

### 총평

핵심 삭제 파이프라인(scan_id → 선택 ID → 백엔드 전용 DeletePlan → 만료·identity 재검증 → 실행)은 문서대로 정확히 구현돼 있고, dev_ports 리스/TOCTOU 설계와 IPC capability 분리는 이 규모 데스크톱 앱에서 보기 드물게 견고하다.
문제는 그 규율이 균일하지 않다는 것 — `terminate_process_group`과 FE 경로 조립 두 지점이 "Rust가 모든 파괴적 결정을 소유한다"는 전제를 혼자 깨고 있고, 프로세스 종료 구현은 안전 모델이 다른 채로 4벌 존재한다.
구조적 최대 리스크는 `StorageOperationGate`의 패닉-on-poison이다. 상태 없는 게이트 하나가 앱 전체 스토리지 기능의 영구 단일 실패점이며, 문서가 선언한 "fail closed"와 정반대로 동작한다.
경계는 대체로 잘 잡혀 있으나 `commands ⇄ storage_commands`·`platform ⇄ safety` 순환과 23필드 `AppState`가 컴파일러 강제력을 없애, 규율이 리뷰어의 기억에만 의존하고 있다.
과설계는 두 곳에 몰려 있다 — 호출자 0~1개인 `ExecutionBudgets` 축과, 이름만 바꾸는 FE 위임 4겹째(`utils/tauri.ts`). 반대로 과소설계는 진실 소스 중복(프로바이더 ID 4곳, 툴 목록 2곳, 커맨드 명단 4곳)에서 나타난다.


---

# 부록 — 비관적사고 차원 원본 (pessimist.md)

## Zenith 비관적 리뷰 (Devil's Advocate / Pre-mortem)

대상: Zenith v0.3.8 (Tauri 2 + Rust ~30k / Svelte 5 ~16k), 읽기 전용 클론
관점: "이 소프트웨어가 사용자 데이터를 날리거나, 신뢰를 잃거나, 유지보수 불능이 되는 가장 그럴듯한 경로"
경로는 저장소 루트 기준 상대경로.

---

## 발견 목록

Critical | src/routes/quick/QuickPanel.svelte:178-184 | 메뉴바 Quick Panel의 "Clean" 버튼이 `selectQuickCleanDefaults()` → `cleanSelected()`로 곧장 이어지고, `src/lib/stores/scan.svelte.ts:449-457`에서 플랜 생성과 실행이 사용자 확인 없이 연속 호출된다. 정작 `src/lib/components/CleanupReviewDialog.svelte:37`은 "Cache cleanup cannot be undone in Zenith"라고 스스로 경고하지만 그 다이얼로그는 `src/routes/dashboard/StorageView.svelte:363` 한 곳에서만 쓰이고, `src/routes/dashboard/CategoryDetailView.svelte:121` 경로도 무확인이다. 즉 "5분 만료 one-shot 플랜"은 사용자 확인 게이트가 아니라 내부 토큰일 뿐이며, 실수로 누른 클릭 한 번이 영구 삭제로 직결된다. 영구 삭제 경로 전부를 CleanupReviewDialog 경유로 강제하라.

Critical | src-tauri/src/safety/blacklist.rs:85-120 | 홈 하위 경로가 `sensitive_relative` 목록에 없으면 119행에서 무조건 `return false`(=삭제 허용)로 조기 반환한다. 목록에는 `Library/Mobile Documents`(iCloud Drive 실체 경로), `Library/CloudStorage`(OneDrive·Google Drive·Dropbox 마운트), `Dropbox`, `Downloads`, `.Trash`, `.config`(gcloud 제외) 가 전부 빠져 있다. macOS "데스크탑 및 문서 iCloud 동기화"를 켜면 `~/Desktop`·`~/Documents`는 `~/Library/Mobile Documents/com~apple~CloudDocs/...` 로 향하는 링크가 되고, `src-tauri/src/safety/symlink.rs:163-168`의 canonical 재검사는 해석된 실경로를 다시 이 목록에 대조하므로 보호가 그대로 무력화된다. 홈 하위 기본 허용을 뒤집어 "시그니처 루트 하위만 허용"하는 allowlist로 바꾸고 클라우드 컨테이너 경로를 추가하라.

Critical | src-tauri/src/safety/toctou.rs:132-150 | 디렉터리 타깃은 dev/ino/is_dir만 대조하고 mtime·size 비교를 명시적으로 건너뛴다(주석 "For individual files"). `DeleteContents` 타깃은 사실상 전부 디렉터리이므로, 플랜 생성 후 300초 안에 사용자가 Xcode 빌드나 `cargo build`를 시작해도 inode가 동일한 한 검증을 통과해 빌드 중인 DerivedData/target 내용을 지운다. README.md:120-121의 "checked again immediately before deletion using filesystem identity metadata"는 디렉터리에 대해서는 사실상 no-op이다. 디렉터리도 mtime을 대조하고, `min_age_days`가 없는 타깃에도 실행 직전 최근변경 재확인을 넣어라.

Critical | src-tauri/src/cleaner/executor.rs:319-333 | 일부만 삭제되면 `status: Partial`인데 `success: true`, `failure_reason: None`으로 보고한다. Full Disk Access 미부여, 파일 잠금, 경로 문자 문제 등으로 절반이 남아도 UI는 성공으로 표시되고 무엇이 남았는지 목록도, 재시도·롤백 표식도 없다. 반쯤 지워진 빌드 캐시(=툴이 corrupt로 오동작하는 상태)가 "성공"으로 사용자에게 전달된다. Partial은 success=false로 두거나 최소한 잔여 항목과 사유를 노출하라.

Critical | src-tauri/src/metrics/memory.rs:350-380 | `terminate_group`은 프로세스 테이블 전체를 돌며 **정규화된 이름 문자열이 같은 모든 프로세스**에 신호를 보낸다. uid·PID·start_time 대조가 하나도 없고 `force=true`면 유예 없이 바로 `Signal::Kill`이다. 이름 정규화(같은 파일 186·202-203행)가 데스크톱 앱과 실행 중인 `claude`/`cursor-agent` CLI 세션을 한 그룹으로 묶어, "Force Quit" 한 번에 진행 중이던 에이전트 세션이 전부 SIGKILL된다. 사용자에게 보여준 PID 목록만 대상으로 하고, kill 직전 start_time을 재확인하라.

Critical | src-tauri/src/metrics/memory.rs:275-304 | 보호 목록이 `terminal|cmd|wt|powershell|explorer|svchost...` 뿐이라 iTerm2·Ghostty·Alacritty·kitty·WezTerm·Warp가 전부 종료 가능하다(같은 파일 186행에서 `/Applications/*.app`은 전부 `installed_user_app`으로 승격). 터미널 에뮬레이터가 SIGKILL되면 그 PTY 아래 셸·vim·에이전트가 SIGHUP으로 함께 죽고 미저장 버퍼가 사라진다. 같은 저장소의 `src-tauri/src/dev_ports/classifier.rs:180-195`는 정확히 이 이름들을 보호 대상으로 갖고 있어 정책이 모듈마다 갈린다. 보호 목록을 단일 소스로 통합하고 번들 ID 기준으로 판정하라.

Critical | src-tauri/src/dev_ports/classifier.rs:301,349-397,424-432 | 개발서버 양성 판정이 argv를 이어붙인 문자열의 **부분 문자열 검사**다(테스트 도구 경로 검증 `match_testing_tool_signature`:471-513 와 달리 exe 신뢰 검증이 없다). 인자 어딘가에 `nodemon`·`uvicorn`·`turbopack`·(`webpack`+`serve`)·(`cargo`+`watch`)가 들어가면 — 로그 경로, 설정 파일명, `-k uvicorn.workers.*` 같은 형태로 — 릴리스 대상이 되어 SIGTERM→SIGKILL을 받는다. DB 트랜잭션을 물고 있는 gunicorn/uvicorn 워커가 정확히 이 모양이다. 토큰/경로 컴포넌트 단위 매칭 + 실행파일이 패키지 설치 경로 하위인지 확인하라.

Critical | src-tauri/src/dev_ports/termination.rs:189-215 | Windows에서 Graceful과 Force가 **둘 다 `TerminateProcess`**이고 차이는 종료 코드뿐이다(206-207). 즉 Windows 사용자는 "Release"를 처음 누른 순간 하드 킬을 당하며 flush·atexit·미저장 처리가 전혀 없다. Graceful은 `CTRL_BREAK_EVENT`/`WM_CLOSE`로 바꾸고 `TerminateProcess`는 Force 전용으로 남겨라.

Critical | src-tauri/tests/safety_tests.rs:1 | 파일 최상단 `#![cfg(unix)]` 때문에 25개 삭제-안전 통합 테스트 전체가 Windows에서 컴파일조차 되지 않는다(`src-tauri/tests/dev_ports_tests.rs:1`도 동일). 그런데 `.github/workflows/ci.yml:146-147`과 `release.yml:214`는 windows-latest에서 `cargo test`를 돌려 초록불을 띄우고 `release.yml:170`이 그대로 NSIS 설치본을 배포한다. `blacklist.rs:225-311`의 Windows 전용 verbatim/UNC/ADS 정규화 코드는 삭제 권한 판정에 쓰이면서 자동 테스트가 0건이다. Windows 픽스처로 이식하거나, 그 전까지 docs/WINDOWS.md:107의 "same checks on windows-latest" 문구를 철회하라.

Critical | src-tauri/src/ai_control_center/git.rs:407-432 | `explicit_diff`가 untracked 파일을 `git diff --no-index /dev/null <path>`로 통째로 덧붙이고 `src-tauri/src/commands/ai.rs:854`가 그 문자열을 그대로 WebView로 반환한다. gitignore되지 않은 `.env`·`id_rsa`·`config.json`이 작업트리에 있으면 파일 내용이 평문 그대로 프론트로 넘어간다. docs/SAFETY.md:361-362의 "Raw secret bytes, credentials ... are never returned"와 정면 충돌. 반환 직전 `diagnostics::sanitize_log`를 태우거나 시크릿 형태 파일명을 diff 대상에서 제외하라.

Major | signatures/system.toml:3-38 | `system.developer_temp`는 `intensive_only`가 아니라 기본 스캔에 포함되고 risk=safe라 Quick Clean에서 자동 선택된다. 판정 기준은 `$TMPDIR` 직계 자식의 **이름 접두사**뿐이다(`src-tauri/src/scanner/walker.rs:129-136`): `claude-`, `codex-`, `gemini-`, `vite-`, `tauri-`, `discord-`, `pytest-of-` 등. 생성자·소유 도구 검증이 없어서 사람이 만든 `/tmp/claude-0/...` 류 에이전트 작업 디렉터리나 `pytest --basetemp` 산출물이 3일만 방치되면 `delete_directory`로 통째 사라진다. README.md:122-123의 "known tool prefixes"는 도구의 증거가 아니라 이름의 우연이다. 접두사 + 트리 내부 도구 마커를 함께 요구하거나 이 시그니처를 intensive_only로 내려라.

Major | src-tauri/src/scanner/walker.rs:155 | "비활동" 판정이 오직 mtime(그것도 트리 최댓값)이다. atime/ctime/birthtime을 보지 않으므로 오래전에 쓰였지만 지금도 활발히 읽히는 캐시(esbuild, metro, torchinductor)와 장기 실행 프로세스가 열어둔 임시 디렉터리가 stale로 분류된다. 반대로 시스템 시계가 앞으로 틀어지면(VM 스냅샷 복원, CMOS 방전, 듀얼부팅 RTC 불일치) 모든 후보가 즉시 "7일 경과"가 된다. `duration_since().unwrap_or_default()`가 0을 주는 덕에 뒤로 틀어진 시계만 보수적으로 막힌다. 열린 fd 확인 또는 atime 병행 + 시계 단조성 검사를 추가하라.

Major | src-tauri/src/scanner/walker.rs:280-290 | exclusion 매칭 규칙이 측정 단계와 삭제 단계에서 서로 다르다. 여기서는 `child_str.contains(ex)`(전체 경로 부분 문자열)이고 `src-tauri/src/safety/tree_deleter.rs:450-462`는 `file_name == ex` 또는 확장 경로 prefix다. 즉 "제외되어 나이 계산에서 빠진" 파일이 삭제 단계에서는 제외 대상이 아닐 수 있다. 지금은 min_age 시그니처에 exclusions가 없어 잠복 상태지만, TOML 한 줄이면 "최근 수정 파일을 제외해 stale로 오판 → 바로 그 파일을 삭제"가 성립한다. 매칭을 단일 함수로 통합하라.

Major | src-tauri/src/safety/tree_deleter.rs:407-448 | `delete_entry`가 엔트리마다 `fs::canonicalize`를 2회 호출하고 `validate_canonical_blacklist`가 1회 더 호출한다. DerivedData나 node_modules 급 수십만 파일이면 수십만~백만 회 realpath 시스템콜이 `src-tauri/src/operation_gate.rs:11`의 전역 락을 쥔 채 단일 스레드로 돈다. `execute_clean`에는 취소 커맨드가 없다(`src-tauri/permissions/autogenerated/`에 cancel_clean 부재). 응답 없는 앱을 사용자가 강제 종료하면 정확히 "반쯤 지워진 캐시"가 남는다. 루트 1회 canonicalize + openat 기반 상대 순회로 대체하고 취소 신호를 추가하라.

Major | src-tauri/src/commands/cleanup.rs:23,42,55,93,137 | 잠금 획득에 `.expect("... poisoned")`를 쓴다(저장소 전체 88곳). 잠금 보유 중 한 번이라도 패닉하면 Mutex가 poison되고 이후 `get_last_scan` 같은 동기 커맨드는 IPC 스레드에서 패닉해 앱이 재시작 전까지 영구 불능이 된다. 반대로 `src-tauri/src/storage_commands.rs:293,327`은 `unwrap_or_else(|p| p.into_inner())`로 **오염된 상태를 그대로 계속 쓴다** — 패닉으로 반쯤 갱신된 인벤토리에서 Trash 플랜이 만들어질 수 있다. 같은 저장소에 두 정책이 공존한다. 정책을 통일하고 poison 시 해당 상태를 폐기 후 재스캔을 요구하라.

Major | src-tauri/src/docker/adapter.rs:457-463 | `container.docker.unused_images`가 `docker image prune -a -f`를 돈다. 컨테이너에 물려 있지 않은 **모든** 이미지가 대상이라 로컬에서만 빌드해 레지스트리에 없는 이미지, `docker commit` 산출물, 내려간 레지스트리에서 받은 이미지가 복구 불가로 사라진다. signatures/containers.toml:26의 risk="rebuild" / "re-download may be required"는 재현 가능성을 전제하지만 로컬 전용 이미지는 재현 불가다. dangling 전용으로 좁히거나 이미지 목록 미리보기 후 개별 선택으로 바꿔라.

Major | src-tauri/src/docker/adapter.rs:446,481-488 | prune은 30초 타임아웃 뒤 클라이언트 프로세스만 죽인다. 데몬 측 prune은 계속 진행되므로 "실패"로 보고된 뒤에도 이미지가 계속 지워진다. 또한 `container.docker.unused_volumes` 분기(`docker volume prune -f`)가 이미 구현되어 있고 지금은 signatures/containers.toml:44-45의 `strategy = "manual"` 때문에만 막혀 있다 — TOML 한 줄이면 DB 볼륨 영구 삭제가 활성화되는 지뢰다. 타임아웃 시 "진행 중"으로 보고하고 volume prune 분기는 코드에서 제거하라.

Major | src-tauri/src/models_inventory/deleter.rs:96-101 | HuggingFace/LM Studio/MLX 모델은 Trash가 아니라 `SafeTreeDeleter::delete_path`로 영구 삭제된다. 앱 언인스톨·대용량 파일·개발 아티팩트는 전부 `src-tauri/src/trash_manager/mod.rs:238`의 `trash::delete`를 쓰는데 여기만 다르다. 게이트가 걸렸거나 삭제된 HF 리포지토리에서 받은 수십 GB 가중치는 복구 경로가 없다. `is_directly_scoped`(104-106)도 루트의 모든 하위를 허용해 직계 자식 제약조차 없다. 모델도 trash_manager 경유로 통일하라.

Major | src-tauri/src/safety/blacklist.rs:97-110 | 보호 목록이 `home.join("Documents")`처럼 하드코딩인 반면 Large Files 루트는 `src-tauri/src/platform/paths.rs:125-149`에서 `SHGetKnownFolderPath`로 리디렉션을 반영해 해석한다. OneDrive Known Folder Move가 켜진 Windows에서는 실제 문서 폴더가 `%USERPROFILE%\OneDrive\Documents`라 블랙리스트가 전혀 걸리지 않는다. `src-tauri/src/developer_artifacts/mod.rs:535-548`의 `protected_workspace_prefixes`도 같은 하드코딩이라 `~/OneDrive/Documents`를 개발 워크스페이스로 등록할 수 있다. 보호 목록도 known-folder API 결과로 생성하라.

Major | src-tauri/src/dev_ports/termination.rs:74,148,288,345 | Windows에서 리스너 uid를 `Some(1000)`으로, `current_uid()`도 1000으로 하드코딩한다. 그 결과 소유권 필터(termination.rs:375), root 소유 차단(`classifier.rs:58`), 타 사용자 차단(`classifier.rs:67`)이 전부 무력화되어 SYSTEM·다른 계정 서비스가 "내 프로세스"로 취급된다. 실제 토큰 SID를 조회하거나, 증명 불가 시 `classifier.rs:81-92`처럼 릴리스를 거부하라.

Major | src-tauri/src/dev_ports/termination.rs:518-532 | "현재 리스너 목록에서 못 찾음"을 `ReleaseOutcome::Released`로 보고하는데 이 분기는 **신호를 보내기 전**이다. lsof 레이스나 bind_address 표기 변화(127.0.0.1↔0.0.0.0, IPv4/IPv6 전환)만으로 "해제됨"이라는 거짓 성공이 뜨고 프로세스는 살아서 포트를 계속 쥔다. docs/SAFETY.md:288의 "reports an ownership change and never signals the replacement"와 어긋난다. "관측되지 않음"과 "종료됨"을 구분하라.

Major | src-tauri/src/dev_ports/termination.rs:594-595,659-671 | 유예가 100ms×15 = **1.5초**뿐이고 이후 `force_authorized: true` 리스를 발급해 Force를 유도하며, Force 경로(586-591)는 추가 유예 없이 즉시 SIGKILL이다. 종료에 1.5초 이상 걸리는 정상 개발서버(빌드 캐시 flush, DB 커넥션 정리)가 "StillListening"으로 표시되어 사용자를 강제 종료로 떠민다. 유예를 늘리고 SIGKILL 전 2차 SIGTERM을 두라.

Major | src-tauri/src/agent_activity/termination.rs:228-233 | 주석은 "Terminal and protected ancestry check"라고 하지만 `is_terminal_or_protected`(130-164)는 **대상 자신의 이름/exe basename만** 본다. `parent_pid`는 125행에서 수집만 되고 테스트 외 어디서도 읽히지 않는다. 존재하지 않는 보호를 문서화해두면 후속 변경이 이를 믿고 진행한다. 부모 체인을 실제로 검사하거나 필드와 주석을 함께 제거하라.

Major | src-tauri/src/agent_activity/mod.rs:149-163 | 종료 대상 판정이 "신뢰 경로 하위 + exe basename ∈ {claude, codex, gemini, grok, copilot, opencode, agy, antigravity, cursor-agent}"뿐이고 `can_stop: true`가 무조건 세팅된다. `~/.local/bin/grok`, `/usr/local/bin/codex` 같은 동명이인 개인 스크립트가 Stop 버튼과 SIGTERM을 받는다. 벤더 이벤트나 세션 파일 같은 보강 증거가 있을 때만 Stop을 노출하라.

Major | src-tauri/src/commands/ai.rs:118-131 | `execute_graceful_stop`은 `kill()`이 0을 반환하면 즉시 Ok이고 사후 검증이 없는데, 호출부가 캐시를 비우고 wake를 알려 UI는 "중지됨"으로 보인다. SIGTERM을 트랩하는 에이전트 CLI는 계속 살아서 파일을 쓰는데 사용자는 멈춘 줄 안다(dev_ports는 1.5초라도 폴링한다). 유예 후 PID/start_time을 재확인해 StillRunning을 보고하라.

Major | src-tauri/src/cleaner/executor.rs:52-56 | 삭제 실패 시 `result.path`(=사용자명이 포함된 절대경로)를 zenith.log에 기록한다. `src-tauri/src/diagnostics/mod.rs:210`이 그 로그 마지막 20줄을 `DiagnosticsSnapshot.recent_errors`에 담아 `src-tauri/src/commands/system.rs:317`의 get_diagnostics로 WebView에 넘기고 사용자가 export한다. `sanitize_log`(diagnostics/mod.rs:62-89)에는 홈 경로 마스킹 규칙이 없다. README.md:36의 "absolute paths never cross into the WebView"의 반례다. sanitize_log에 home→`~` 치환을 추가하라.

Major | src-tauri/src/ai_usage/mod.rs:446-452 | OAuth 인가 URL에 `state` 파라미터가 없다. 콜백 리스너(464행)는 127.0.0.1의 첫 연결을 무조건 수락해 code를 뽑아 513행에서 토큰 교환한다. 같은 머신의 다른 프로세스가 리슨 포트를 열거해 `?code=<공격자코드>`를 먼저 던지면 authorization code injection이 성립하고(PKCE로는 못 막는다) 사용자의 Zenith가 공격자 OpenRouter 키를 붙잡는다. state 난수 발급·검증을 추가하라.

Major | README.md:93-96 | 미서명·미공증 번들에 대해 `xattr -cr /Applications/Zenith.app`을 공식 안내한다. Gatekeeper 격리 속성을 사용자 손으로 제거하게 만드는 절차인데, `src-tauri/tauri.conf.json`에 updater 설정이 전무해(pubkey/endpoints 0건) 침해나 데이터 손실 버그 발생 시 강제 회수·자동 갱신 경로가 없다. 유일한 신뢰 앵커인 SHA256SUMS 검증도 순전히 사용자 자율이다. 우클릭-열기 절차만 남기고 xattr 안내는 삭제하라.

Major | .git (git log 결과: 커밋 1건, 작성자 1명 `jaeyoung`) | 전체 히스토리가 단일 커밋으로 스쿼시되어 있다. 미서명 바이너리를 배포하면서 릴리스 간 diff·리뷰 기록·bisect 근거가 전무하다는 뜻이고, CODE_SIGNING_POLICY.md:53-60이 약속한 "reviewed release workflow" 원산지 검증을 외부인이 재현할 수 없다. 30k줄 Rust에 bus factor 1이 그대로 남는다. 히스토리 보존 저장소를 공개하고 최소 2인 리뷰어를 명시하라.

Minor | src-tauri/src/safety/blacklist.rs:77-82 | ADS 방어로 경로 문자열에 콜론이 있으면(인덱스 1 제외) 전 플랫폼에서 블랙리스트 처리한다. POSIX에서 콜론은 합법 문자라 캐시 안에 콜론 든 파일이 하나만 있어도 `tree_deleter.rs:122-125`에서 조용히 `skipped_files`로 빠지고 보고 바이트가 실제와 어긋난다. 루트 경로에 콜론이 있으면 정리 전체가 실패한다. ADS 검사는 Windows에만 적용하라.

Minor | src-tauri/src/safety/tree_deleter.rs:465-475 | 회수 바이트를 `blocks()*512`로 합산한다. pnpm 하드링크 저장소나 APFS 클론에서는 같은 물리 블록을 여러 번 세므로 "확보 예정 용량"이 실제보다 부풀고, 사용자는 기대만큼 공간이 안 늘어난 것을 결함이나 과장으로 받아들인다. `actual_disk_free_delta`(executor.rs:76-79)로 사후 보정만 할 뿐 사전 표시는 그대로다.

Minor | src-tauri/src/signatures/registry.rs:55-77 | `load_from_dir`는 임의 디렉터리의 TOML을 그대로 등록한다(현재 호출부 없음, 실제로는 include_str! 임베딩만 사용). 이 함수가 언젠가 "사용자 정의 시그니처" 기능으로 연결되면 홈 하위 아무 경로나 삭제 루트로 승격되는데, 블랙리스트가 홈 하위를 기본 허용하므로 방어선이 없다. 지금 제거하거나 경로 allowlist를 전제로 재설계하라.

Minor | src-tauri/src/scanner/walker.rs:146,205 | 깊이 32 초과는 `complete=false`로 fail-closed라 안전 방향이지만, 중첩 node_modules/pnpm 트리에서는 상시 초과해 해당 후보가 조용히 사라진다. 사용자에게는 "정리할 게 없다"로만 보이고 이유가 표시되지 않아 신뢰가 아니라 혼란을 만든다. 스킵 사유를 UI에 노출하라.

Minor | src-tauri/src/diagnostics/mod.rs:30 | home() 실패 시 로그 디렉터리가 `std::env::temp_dir()/zenith_logs`로 떨어지는데 저장소 전체에 로그 파일 권한 지정(0600)이 한 건도 없어 기본 umask대로 0755/0644가 된다. 절대경로가 담긴 로그가 공용 /tmp에서 동일 호스트의 다른 사용자에게 읽힌다.

Minor | src-tauri/capabilities/quick.json:4 | description은 "Read-mostly menu-bar panel"인데 permissions에 `allow-create-delete-plan`(15행)과 `allow-execute-clean`(16행)이 들어 있다. `decorations:false` + `alwaysOnTop:true`(tauri.conf.json:38-40)로 항상 떠 있는 창에 영구 삭제 실행 권한이 붙은 셈이라 위 첫 번째 Critical(무확인 1클릭 삭제)과 정확히 겹친다.

---

### Pre-mortem: 6개월 후 가장 가능성 높은 사고 3건

### 1. "Quick Clean 눌렀더니 3일 전 실험 결과가 사라졌다" (가장 확률 높음)
사용자가 `/tmp` 아래에 `claude-*` 또는 `pytest-of-*` 이름의 작업 디렉터리를 두고 며칠 자리를 비운다. 다음 출근길에 메뉴바에서 습관적으로 Clean을 누른다.
`system.developer_temp`가 기본 스캔·Safe·자동선택이고, 확인 다이얼로그가 없으며(QuickPanel.svelte:180) 판정 근거는 이름 접두사와 mtime뿐이므로 디렉터리가 통째로 `delete_directory`된다.
휴지통이 아니라 영구 삭제라 복구 경로가 없고, 결과 모달에는 "Success"와 회수 용량만 뜬다. GitHub 이슈 제목은 "Zenith deleted my work directory"가 된다.

### 2. "빌드 도중 정리가 돌아 툴체인이 깨졌다"
사용자가 스캔 후 커피를 가지러 간 사이(5분 이내) IDE가 백그라운드 인덱싱/빌드를 시작한다. 돌아와 Clean을 누른다.
디렉터리 TOCTOU는 inode만 보므로(toctou.rs:132) 통과하고, `min_age_days`가 없는 DerivedData·target·Go build cache에는 실행 직전 신선도 재확인 자체가 없다. 삭제 중 파일 잠금으로 절반이 실패하지만 executor.rs:319는 `Partial + success:true`로 보고한다.
사용자는 "성공"을 보고 빌드를 재개하고, 이후 원인 불명의 링커 오류·인덱스 손상을 며칠 디버깅한 뒤에야 Zenith를 의심한다. 재현이 안 되므로 이슈는 "not reproducible"로 닫힌다.

### 3. "Force Quit 한 번에 터미널과 에이전트 세션이 같이 죽었다"
메모리 뷰에서 상위 점유 그룹으로 iTerm2 또는 이름이 같은 CLI 그룹이 뜬다. 사용자가 Force Quit을 누른다.
`terminate_group`(memory.rs:350-380)은 이름이 같은 모든 프로세스에 uid·PID 검증 없이 즉시 SIGKILL을 보내고, 보호 목록(275-304)에는 iTerm/Ghostty/Warp가 없다. 터미널이 죽으면 PTY 하위 셸·vim·에이전트가 SIGHUP으로 연쇄 종료된다.
결과 토스트는 "Force quit requested (N processes)"라는 성공 문구만 남기고(memory.svelte.ts:83-84) 부분 종료·실패 건수는 보고되지 않아, 사용자는 무엇을 잃었는지조차 목록으로 확인할 수 없다.

---

### 문서 "never/always" 반례표

| 문장 | 위치 | 반례 가능? | 근거 |
|---|---|---|---|
| "Documents, Desktop, and Movies remain blacklisted for generic signature-based cleanup" | docs/SAFETY.md:120-122 | 예 | blacklist.rs:97-110이 `home.join("Desktop")` 하드코딩. iCloud 데스크탑·문서 동기화 시 실경로는 `Library/Mobile Documents/...`, Windows OneDrive KFM 시 `%USERPROFILE%\OneDrive\Documents` — 둘 다 목록 밖 |
| "Planned files are checked again immediately before deletion using filesystem identity metadata" | README.md:120-121 | 예 | toctou.rs:132-150이 디렉터리는 mtime/size 비교를 건너뜀. DeleteContents 타깃은 사실상 전부 디렉터리 |
| "Zenith never scans or deletes all of `/tmp`" | README.md:122-123, docs/SAFETY.md:42-43 | 부분적 | 전체는 아니지만 판정이 이름 접두사뿐(walker.rs:129-136). 사용자가 만든 동명 접두사 디렉터리는 도구 산출물과 구분되지 않음 |
| "System paths, home roots, credentials, keychains ... and standard user-content folders are blocked" | README.md:117-118 | 예 | blacklist.rs:119가 목록 밖 홈 하위를 무조건 허용. `Downloads`, `.netrc`, `.npmrc`, `.docker/config.json`, `.config/gh`, `.Trash` 미포함 |
| "PID, argv, prompts, transcripts, credentials, and absolute paths never cross into the WebView" | README.md:36 | 예 | executor.rs:52-56이 절대경로를 로그에 쓰고 diagnostics/mod.rs:210 → commands/system.rs:317로 WebView에 전달. sanitize_log에 홈 경로 규칙 없음 |
| "Raw secret bytes, credentials ... are never returned, logged, or persisted" | docs/SAFETY.md:361-362 | 예 | git.rs:407-432가 untracked 파일 전문을 diff로 붙여 commands/ai.rs:854로 반환 |
| "100% Local: Zero telemetry, zero cloud tracking, and zero remote analytics" | README.md:100 | 부분적 | 텔레메트리는 실제로 없음. 다만 ai_usage/mod.rs:349,513이 openrouter.ai로 나가고, settings.rs:211-224가 공급자 CLI를 기본 활성으로 두어 collect_codex(135-142)가 `codex app-server --stdio`를 깨움 |
| "Zenith never exposes an arbitrary PID-kill command" | README.md:137 | 형식상 참, 실질 반례 | PID 대신 **이름 그룹** 킬을 노출(memory.rs:350-380). uid·start_time 검증 없이 동명 프로세스 전부에 SIGKILL이라 범위는 오히려 더 넓다 |
| "PID reuse, port handoff ... terminals, databases, container daemons, and Zenith itself fail closed" | README.md:155-156 | 예 | dev_ports는 대체로 지키지만 memory.rs의 terminate_group에는 PID 재사용 방어가 전무하고 보호 목록에 현대 터미널이 없음 |
| "`Manual`: stateful or ambiguous data; never executable by Generic Cleaner" | docs/SAFETY.md:240 | 현재는 참, 지뢰 | planner.rs:70-81이 Manual을 거부해 유효. 단 docker/adapter.rs:481-488의 volume prune 분기가 살아 있어 containers.toml:45 한 줄만 바꾸면 활성화됨 |
| "the executor repeats the full-tree inactivity check immediately before deletion" | docs/SAFETY.md:47 | 부분적 | executor.rs:200-259는 `min_age_days`가 있는 시그니처에만 적용. DerivedData·target·Go cache 등 대부분의 Safe/Rebuild 타깃에는 신선도 재확인이 없음 |
| "The safety tests are in `src-tauri/tests/`" (Windows CI 동등 검증 함의) | README.md:158-159, docs/WINDOWS.md:107 | 예 | safety_tests.rs:1의 `#![cfg(unix)]`로 Windows에서는 25건 전부 미컴파일. Windows CI는 빈 스위트로 초록불 |

---

### 총평

안전 코드는 진지하다 — TOCTOU 가드, 심볼릭 링크 조상 검사, fail-closed 측정, 트리 삭제기의 소유자/inode 재확인은 이 규모 1인 프로젝트로는 이례적으로 촘촘하다.
문제는 그 촘촘함이 **파일시스템 계층에만** 있다는 점이다. 정작 사용자가 데이터를 잃는 지점은 그 위 — 확인 없는 1클릭 실행, 이름 접두사와 mtime만 믿는 후보 선정, 디렉터리 mtime을 보지 않는 TOCTOU, "부분 삭제 = 성공" 보고 — 에 몰려 있다.
프로세스 종료 쪽은 두 갈래로 갈라져 dev_ports는 방어적이고 memory.rs의 이름 그룹 킬은 사실상 무방비인데, 문서는 전자의 기준으로 전체를 설명한다.
문서의 강한 단정(never/always)은 대체로 "코드 어딘가에 그런 검사가 있다"는 사실에 근거하지만, 그 검사가 실제 실행 경로 전부를 덮는지는 검증되지 않았고 Windows에서는 테스트 자체가 컴파일되지 않는다.
미서명 배포 + 자동 업데이트 부재 + 커밋 1개짜리 히스토리 + bus factor 1 조합에서, 위 사고 중 하나라도 실제로 터지면 되돌릴 수단도 신뢰를 회복할 근거도 남지 않는다.


---

# 부록 — 보안 차원 원본 (security.md)

## Zenith 보안 리뷰 (읽기 전용)

대상: `zenith` (Tauri 2 + Rust + Svelte 5, 데스크톱). 위협모델 = (a) 침해된 WebView/XSS가 IPC 호출, (b) 같은 UID의 다른 로컬 프로세스(파일시스템 레이스), (c) 공급망(의존성·릴리스), (d) 네트워크 응답(OAuth/usage API).
라인번호는 전부 실측값. 파일 수정·네트워크·라이브 시스템 변경 없음.

## 발견

Critical | src-tauri/src/commands/system.rs:29 + src-tauri/src/metrics/memory.rs:353-372 | `terminate_process_group(name, force)`가 FE 문자열과 force 불리언만으로 전체 프로세스 테이블을 훑어 즉시 SIGKILL한다. lease·TTL·일회성·uid 검사·start_time 재확인·자기 PID 비교가 전부 없고 보호는 하드코딩 이름 목록(memory.rs:277-304)뿐이라 `/Applications` 하위 앱은 사실상 전부 강제 종료 대상이다. SAFETY.md:265-266의 "force 전에 normal 종료를 먼저 제공"을 강제하는 코드가 Rust에 없는데, 같은 문서 3-4행이 "UI는 보안 경계가 아니다"라고 선언한다. 권장: dev_ports와 동일한 30초 1회성 lease + force-authorized 2단계 lease로 교체하고 kill 직전 uid/start_time/self-PID를 재확인하라.

Critical | src-tauri/src/ai_usage/mod.rs:446-476 | OpenRouter OAuth에 state 파라미터가 없고(448-452), 콜백 수신부가 경로·메서드·Origin을 전혀 검증하지 않는다(470-476: 요청 첫 줄의 두 번째 토큰을 그대로 URL로 붙여 `code`만 추출). 로그인 창이 열린 180초 동안 사용자가 방문한 임의 웹페이지나 로컬 프로세스가 `http://127.0.0.1:<port>/?code=<공격자코드>`를 치면 앱이 공격자 계정의 OpenRouter API 키를 저장한다(과금·사용량 귀속 탈취). 권장: 128비트 난수 state 발급·상수시간 대조, `path == "/callback"` + GET 한정.

Major | src-tauri/src/ai_usage/mod.rs:434,442 | 리스너는 `127.0.0.1:0`에 바인드하는데 콜백 URL은 `http://localhost:{port}`로 발급한다. localhost가 `::1`로 먼저 해석되는 환경이면 인가 코드가 `[::1]:port`를 선점한 로컬 프로세스로 배달된다 (추정 — 해석 순서 의존. 검증: 해당 포트로 `::1` 리스너를 먼저 띄우고 로그인 시도). 권장: 콜백 URL을 `127.0.0.1`로 고정.

Major | src-tauri/src/tooling.rs:37-41,601-604 | 공용 러너 `command(name)`이 상속 PATH를 **먼저** 뒤진 뒤 `~/.cargo/bin`·`~/.local/bin`·`~/.npm-global/bin`·`~/.volta/bin`·`~/.asdf/shims`를 후보에 추가한다. docker/git/ollama/codex/`open`/powershell.exe가 전부 이 경로를 타며 서명·소유자 검증이 없어, npm/cargo postinstall 하나가 그 이름의 바이너리를 심으면 Zenith가 실행한다. cache_providers/mod.rs:327 `validate_executable`은 신뢰 디렉터리 검증을 하는데 tooling에는 없어 정책이 비일관. 권장: 시스템 유틸은 절대경로 고정, 나머지는 `validate_executable` 재사용.

Major | src-tauri/src/commands/system.rs:241,255,267-280 | `open_in_terminal(path)`·`show_in_file_manager(path)`가 FE의 임의 절대경로를 `canonicalize`만 하고 그대로 `open`/`explorer.exe`/`powershell.exe`에 넘긴다. 홈 포함 검사·블랙리스트·백엔드 소유 경로 화이트리스트가 전무해, SAFETY.md:18-21의 "FE는 실행 가능한 경로를 제공할 수 없다" 원칙에서 벗어난 유일한 커맨드 쌍이다. 권장: 최근 스캔 항목·등록 워크스페이스·project_roots 멤버십 검증 추가.

Major | src-tauri/src/agent_activity/events.rs:138-142 → src-tauri/src/agent_activity/projects.rs:35-48 | `post_agent_event`의 `cwd`를 길이와 절대경로 여부만 검증하고 그 경로에서 `git -C <cwd> status`를 실행한다. `-c core.fsmonitor=`·`GIT_CONFIG_NOSYSTEM=1`·`safe.directory` 하드닝이 없어, 공격자가 `.git/config`를 제어하는 저장소를 가리키면 임의 명령이 실행된다. 단 전제가 좁다 — 평범한 `git clone`은 config를 옮기지 않으므로 압축파일·공유 볼륨으로 배달된 `.git` 디렉터리에 한정된다. 권장: cwd를 홈 하위·등록 워크스페이스로 제한하고 git 호출에 fsmonitor/hooksPath 무력화 env를 부여.

Major | src-tauri/src/ai_control_center/git.rs:341-357,359-373 | `run_git`에 `--no-ext-diff`가 없고 `run_diff_command`에 `--no-textconv`가 없다. 저장소의 `.gitattributes` + `.git/config`의 `diff.<driver>.textconv` 조합으로 diff 생성 시 임의 명령이 실행된다(위와 동일 전제). 참고로 경로 인자는 모두 `--` 뒤에 배치되어 옵션 인젝션은 차단돼 있다(389,398,422). 권장: 두 함수에 `-c core.fsmonitor= -c core.hooksPath=/dev/null --no-textconv` + `GIT_CONFIG_NOSYSTEM=1` 공통 적용.

Major | src-tauri/src/safety/symlink.rs:24 | `is_symlink`가 `symlink_metadata` 실패 시 `false`(= 심볼릭 링크 아님)를 반환한다. 권한 오류나 경합으로 stat이 실패한 경로가 "안전"으로 판정되고, 이 함수는 planner·tree_deleter·large_files·trash_manager의 모든 심볼릭 링크 게이트가 호출한다. SAFETY.md:298 "Safety checks fail closed"와 정면 배치. 권장: `Result`를 반환해 오류를 차단으로 처리.

Major | src-tauri/src/safety/symlink.rs:163-168 | `validate_canonical_blacklist`가 `canonicalize` 실패 시 `Ok(())`를 반환한다. cleaner/executor.rs:156과 tree_deleter.rs:143이 이 함수에 정규 경로 블랙리스트 재확인을 의존하므로, 경합으로 canonicalize가 실패하는 순간 검사가 통째로 건너뛰어진다. 권장: 실패를 에러로 승격.

Major | src-tauri/src/safety/tree_deleter.rs:159,210 | `prepare_directory`(231-241)가 `O_DIRECTORY|O_NOFOLLOW|O_CLOEXEC`로 fd를 확보해두는데도 실제 삭제는 `fs::remove_file(path)`/`fs::remove_dir(path)`로 전체 경로를 다시 이름 해석한다. dev/ino 재확인(359-377) 직후에도 중간 디렉터리 컴포넌트는 커널이 다시 따라가므로, 같은 UID의 다른 로컬 프로세스가 그 창에서 상위 디렉터리를 심볼릭 링크로 교체하면 삭제가 다른 트리로 유도된다. 권장: 확보한 dirfd 기준 `unlinkat(dirfd, name, ...)`/`unlinkat(..., AT_REMOVEDIR)`로 전환.

Major | src-tauri/src/safety/blacklist.rs:85-121 | 홈 하위 경로는 21개짜리 `sensitive_relative` 목록에 걸릴 때만 차단하고, 그렇지 않으면 119행에서 즉시 `false`를 반환해 이후 임시디렉터리·시스템 프리픽스 검사까지 건너뛴다. `~/.netrc`·`~/.git-credentials`·`~/.docker/config.json`·`~/.npmrc`·`~/.config`(gcloud 제외)·`~/Downloads`·브라우저 프로파일이 전부 미보호인데 README:113-114는 "credentials … are blocked"라고 단언한다. 권장: 자격증명 파일 규칙 추가 및 early-return 제거.

Major | signatures/system.toml:41-58 + src-tauri/src/scanner/walker.rs:177 | `system.intensive.user_app_caches`가 `risk = "safe"`라 `is_auto_selectable()`을 통과해 스캔 즉시 **자동 선택**된다. 보호는 대소문자 구분 이름 접두사 7개(`com.apple.` 등)뿐이라, 서드파티가 `~/Library/Caches`에 둔 비캐시 상태 데이터도 7일 무활동이면 기본 선택되어 항목 검토 없이 Quick Clean으로 지워진다. 권장: 최소 `rebuild` 티어로 낮춰 기본 선택에서 제외.

Major | src-tauri/capabilities/quick.json:4,15-17 | 설명은 "Read-mostly … backend-owned safe cleanup plans"인데 실제로는 `allow-start-scan`·`allow-create-delete-plan`·`allow-execute-clean`을 모두 부여한다. 장식 없는 투명 always-on-top 패널이 메인 창과 동일한 삭제 권한(항목 임의 선택, Rebuild 티어 포함, `docker image prune -a -f` 도달 가능)을 갖는다. 권장: quick에는 백엔드가 만든 Safe 전용 플랜만 실행하는 별도 커맨드를 노출.

Major | src-tauri/src/agent_activity/hooks.rs:172-179 | `atomic_write_json`이 임시파일을 기본 umask(통상 0644)로 생성해 `~/.claude/settings.json` 위로 rename한다. 사용자가 0600으로 잠근 설정의 권한이 조용히 강등되고, rename 직전 전체 내용이 0644 임시파일로 노출된다. 권장: 원본 `permissions()`를 임시파일에 복사하거나 0600으로 생성.

Major | src-tauri/src/diagnostics/mod.rs:43-59 | 리댁션 실측 우회 다수: `Authorization: Basic dXNlcjpwYXNz`는 스킴만 가려지고 자격증명이 남고(51), `github_pat_`·`gho_`·`AKIA`·`AIza`·`npm_`는 무변환, 값 문자셋 `[a-zA-Z0-9_.-]` 때문에 특수문자 비밀번호는 앞부분만 가려지며(`PASSWORD="p@ss w0rd!"` → `[REDACTED]@ss w0rd!`), `https://user:pass@host` URL 자격증명은 전혀 처리되지 않는다. README:101의 "automatically redact … tokens, passwords" 주장이 실제 커버리지를 과장한다. 권장: ai_control_center/safety.rs:25-29의 패턴셋과 공유 모듈로 통합.

Major | src-tauri/src/diagnostics/mod.rs:116-120 · src-tauri/src/settings_store.rs:98-99 · src-tauri/src/ai_control_center/audit.rs:78-81 | 로그·설정·감사 파일 어디에도 0600 지정이 없다(프로덕션 코드 전체에 `set_permissions`/`from_mode` 0건, 테스트에만 존재). 다중 사용자 머신에서 다른 로컬 계정이 `zenith.log`와 `ai-control-audit.json`을 읽는다. 권장: `OpenOptions::mode(0o600)` + 저장 디렉터리 0700.

Major | src-tauri/src/dev_ports/termination.rs:74,148,288,345 | Windows에서 `current_uid()`와 프로세스 uid가 상수 `1000`으로 날조된다. 소유자 필터(375), 분류기의 "다른 사용자 소유" 차단(classifier.rs:67-75), 시그널 직전 uid 대조(553)가 Windows에서 전부 무조건 통과하고 실제 방어는 OpenProcess ACL만 남는다. 같은 저장소 `agent_activity/mod.rs:406-422`는 실제 SID를 비교하므로 방식이 불일치. 권장: SID 대조로 통일.

Major | src-tauri/src/dev_ports/termination.rs:189-215 | Windows `send_signal`이 signal 값을 종료코드로만 쓰고(206) graceful/force 양쪽 다 `TerminateProcess`를 호출한다. SAFETY.md:284 "A normal release sends SIGTERM only"가 Windows에서 지켜지지 않아 저장되지 않은 작업이 유실된다. 권장: Windows에서는 graceful 경로를 비노출하거나 CTRL_BREAK_EVENT로 분리.

Major | src-tauri/src/dev_ports/classifier.rs:349-397 | 개발서버 양성 판정 상당수가 `argv_joined.contains(...)` 무앵커 부분문자열이고 일부는 두 문자열의 단순 AND(`"webpack" && "serve"`, `"cargo" && "watch"`)다. `nodemon`/`turbopack`/`@remix-run`은 프로세스 이름 게이트조차 없다. 보호 판정(142-152)은 반대로 완전일치라, 커맨드라인 어딘가에 그 문자열이 있는 무관한 리스너가 `can_release: true`로 승격되는 비대칭이 생긴다. 권장: 토큰 경계 기반 매칭으로 통일.

Major | src-tauri/src/metrics/memory.rs:231-237,277-304 | 자기 자신 보호가 리터럴 `"zenith"|"zenith.exe"` 완전일치뿐이고 `std::process::id()`/`current_exe()` 비교가 없어 리브랜딩 빌드(예: "Zenith Nightly")는 자기 자신을 SIGKILL한다. 터미널 보호도 `alacritty`·`kitty`·`wezterm`·`warp`·`tmux`를 빠뜨려 `dev_ports/classifier.rs:180-195`의 목록과 불일치. 권장: 보호 목록을 단일 상수로 통합하고 self-PID/self-exe를 무조건 skip.

Major | .github/workflows/release.yml:156-158,285-291 + README.md:72 | 아티팩트를 만든 잡이 SHA256SUMS를 계산해 같은 릴리스에 동봉하고, publish 잡이 그 매니페스트로 자기 산출물을 검증한다(동어반복). 그런데 README:72는 사용자에게 "설치 전 SHA256SUMS.txt와 대조하라"고 안내해 변조 방지 보증처럼 읽힌다. 릴리스 쓰기 권한을 얻은 공격자는 바이너리와 체크섬을 함께 교체하면 된다(서명·notarization·provenance attestation 전무). 권장: OIDC 기반 build provenance 또는 분리 서명 도입, 그전까지는 문서에 "다운로드 손상 확인용"임을 명시.

Major | .github/workflows/ci.yml:29,89,126 · .github/workflows/release.yml:34,91,179,294 | 서드파티 액션이 전부 가변 태그/브랜치 참조다. `dtolnay/rust-toolchain@stable`은 강제 이동 브랜치이고, `contents: write` 토큰을 받는 `softprops/action-gh-release@v2`도 SHA 핀이 없어 액션 저장소가 침해되면 릴리스 자산을 임의 교체할 수 있다. 권장: 전 액션 커밋 SHA 핀 + Dependabot.

Major | .github/workflows/release.yml:54,114,201 | 릴리스 러너에서 `pnpm install --frozen-lockfile`을 lifecycle script 허용 상태로 실행한다(.npmrc 없음, `onlyBuiltDependencies` 없음, pnpm 9 고정이라 postinstall 차단이 기본 아님). 436개 잠금 패키지 중 하나만 침해돼도 설치 단계에서 인스톨러를 조작할 수 있다. 권장: `--ignore-scripts` 또는 pnpm 10 + 빌드 화이트리스트.

Major | scripts/install_release_app.sh:104-113 | `validate_bundle`이 `CFBundleIdentifier == com.zenith.desktop`와 버전 문자열 존재만 확인한다. `codesign --verify`·`spctl --assess`·체크섬 대조가 없어, 빌드 트리에 쓸 수 있는 주체(악성 crate build script, 오염된 캐시)가 plist 두 키만 맞춘 위조 번들을 두면 `just release`가 그대로 `/Applications`에 설치한다. (긍정 확인: sudo 없음, `mktemp -d` 사용, 기존 앱을 rm하지 않는 스테이징→롤백 트랜잭션 정상 구현, curl|sh 패턴 없음.) 권장: 설치 전 codesign 검증을 필수 게이트로.

Minor | src-tauri/src/ai_usage/mod.rs:502-518,526 | 토큰 교환 POST에 리다이렉트 정책이 없어 기본 10회 follow다. 307/308이면 `code`+`code_verifier` 본문이 새 호스트로 재전송된다. 응답 본문 크기 상한도 없고 `json::<Value>()`로 스키마 없이 역직렬화한다. 권장: `redirect::Policy::none()` + 본문 상한 + 전용 struct.

Minor | src-tauri/src/ai_usage/mod.rs:490-493 | accept 루프가 첫 연결 하나만 소비하고, `code` 없는 요청이면 즉시 `Err`로 흐름을 끝낸다. 브라우저 프리커넥트나 로컬 포트 스캐너 한 방으로 로그인이 DoS된다. 권장: code 없는 연결은 400 응답 후 루프를 계속하고 타임아웃까지 대기.

Minor | src-tauri/src/ai_control_center/safety.rs:156-182 | `is_scannable_file`이 확장자 매칭이라 `.env`(`Path::extension()`이 None)와 `.env.local`(확장자 "local")이 시크릿 스캔에서 제외된다. 시크릿이 가장 많이 사는 파일이 탐지 사각지대다(누출이 아니라 탐지 실패). 권장: `file_name()`이 `.env`로 시작하는 경우를 별도 허용.

Minor | src-tauri/src/safety/planner.rs:95-99 | `min_age_days` 시그니처에서 `path == *root`도 허용한다. 워커(walker.rs:39-47)가 루트 항목을 만들지 않아 현재는 도달 불가하지만, 플래너 단독으로는 `~/Library/Caches` 전체를 `delete_directory` 대상으로 승인한다 — SAFETY.md:63 "the root itself is never returned as a cleanup item"의 강제 지점이 워커 한 곳뿐이다. 권장: 플래너에서 직계 자식임을 명시적으로 요구.

Minor | src-tauri/src/safety/blacklist.rs:143-177 | 시스템 프리픽스·`C:/Users`·홈 비교가 전부 대소문자 구분이다. macOS APFS 기본값과 Windows는 대소문자 무시 파일시스템이라 `c:/windows/...`·`/USERS/...` 형태는 어떤 규칙에도 걸리지 않는다 (추정 — 현재 경로는 전부 `$HOME`·시그니처 TOML·`read_dir` 실명에서 오므로 실제 진입 경로는 확인되지 않음. 검증: `Blacklist::is_blacklisted(Path::new("c:/windows/system32"))` 단위 테스트). 권장: 플랫폼별 대소문자 무시 비교.

Minor | src-tauri/src/cleaner/executor.rs:52-56 | 정리 실패 시 삭제 대상 절대경로 전체를 디스크 로그에 남긴다. 경로 문자열은 어떤 리댁션 패턴에도 걸리지 않아 사용자명·프로젝트명이 평문으로 남고 진단 스냅샷으로 화면에도 노출된다. 권장: `diagnostics::normalized_log_path`처럼 `~/`로 축약해 기록.

Minor | src-tauri/src/platform/system_actions.rs:77,86,93 | `wt.exe`·`powershell.exe`·`cmd.exe`를 절대경로 없이 spawn하며 `current_dir`을 FE가 준 경로로 설정한다. HKCU PATH는 사용자 쓰기 가능이라 바이너리 플랜팅 여지가 있다 (추정 — Rust std의 Windows 프로그램 탐색이 자식 cwd를 포함하는지는 버전 의존. 쉘 경유·인자 보간은 없음). 권장: `%SystemRoot%\System32\` 절대경로 사용.

### 문서 주장 검증표

| 주장 | 코드 위치 | 결과 |
|---|---|---|
| "Generic cleanup does not call `remove_dir_all`" (SAFETY.md:37-38) | 저장소 전체 grep 0건; tree_deleter.rs:159,210만 `remove_file`/`remove_dir` | 지킴 |
| "The tree deleter walks bottom-up without following symlinks … unlinks a symlink itself" (SAFETY.md:35-37) | tree_deleter.rs:153-169,197-219 + `O_NOFOLLOW` 231-241 | 지킴 |
| "Safety checks fail closed" (SAFETY.md:298) | symlink.rs:24 · symlink.rs:163-168 fail-open 2곳 | 미지킴 |
| "System paths, home roots, credentials … are blocked" (README:113-114) | blacklist.rs:85-121 — `.netrc`/`.git-credentials`/`.docker`/`.npmrc` 미포함, 119행 early-return | 부분 |
| "Planned files are checked again immediately before deletion using filesystem identity" (README:115-116) | executor.rs:184-197, tree_deleter.rs:359-405 (dev/ino) | 지킴. 단 `identity`가 None(플랜 시점 부재)이면 건너뜀 |
| "the root itself is never returned as a cleanup item" (SAFETY.md:63) | walker.rs:39-47은 지킴, planner.rs:95-99는 `path == root` 허용 | 부분 |
| "only direct children … symlink children and protected Apple/system prefixes are skipped" (SAFETY.md:60-63) | walker.rs:120-157, signatures/system.toml:48-56 | 지킴 |
| "The frontend may not provide an executable deletion path … arbitrary PID" (SAFETY.md:18-21) | 삭제 경로·PID는 지킴(planner.rs:26-44, lease 기반). `open_in_terminal`/`show_in_file_manager`는 임의 경로 수용 | 부분 |
| "Memory Inspector … does not accept a PID from the WebView" (SAFETY.md:263-264) | commands/system.rs:29 — `name: String`만 수신 | 지킴(문자로만) |
| "Normal application termination is offered before a confirmed force termination" (SAFETY.md:265-266) | memory.rs:356이 FE 불리언 직결, 백엔드 게이트 없음 | 미지킴 |
| "System processes, terminals, and Zenith are protected" (SAFETY.md:264-265) | memory.rs:231-237,277-304 — 리터럴 이름 목록, uid 검사 없음 | 부분 |
| "A normal release sends `SIGTERM` only to the exact listener PID" (SAFETY.md:284) | Unix 지킴(dev_ports/termination.rs:587-591), Windows는 graceful도 `TerminateProcess`(189-215) | 부분 |
| "Leases expire after 30 seconds, are capped … consumed before any mutation" (SAFETY.md:276-278) | store.rs:7,8,91-97,105-115 + termination.rs:492-497 | 지킴 |
| "PID reuse … fail closed" (SAFETY.md:282-284) | dev_ports 553-562 / agent_activity 201-216 지킴, memory.rs:355→369 미지킴 | 부분 |
| "Public snapshots contain opaque hashes rather than PID" (SAFETY.md:257) | AgentSession 지킴, `DevelopmentListener.pid`(models/dev_ports.rs:42)는 직렬화됨 | 부분 |
| "Project Cockpit is observation-only … No termination lease or mutable command" (SAFETY.md:249,254-255) | `request_stop_agent_session`(commands/ai.rs:102)이 SIGTERM 발송, `stop_lease_id` 공개 | 미지킴(문서가 기능을 부정) |
| "Secret redaction guarantees … raw secret bytes never returned" (SAFETY.md:358-362) | safety.rs:107-113,294-343 — 라인번호·상대경로·카테고리만 반환 | 지킴 |
| "Subprocess errors and diagnostic messages automatically redact … tokens, passwords" (README:101) | diagnostics/mod.rs:43-59 — Basic/PAT/AKIA/AIza/URL 자격증명 미탐지 | 미지킴 |
| "Zero telemetry, zero cloud tracking" (README:100) | 외부 HTTP는 openrouter.ai 2개 엔드포인트뿐 | 지킴 |
| "Zenith never exposes an arbitrary PID-kill command" (README:137) | 임의 PID는 불가하나 임의 앱 이름 kill은 가능 | 부분 |
| CSP / 원격 IPC | tauri.conf.json:13,47 — `withGlobalTauri:false`, `script-src 'self'`, `dangerousRemoteDomainIpcAccess` 없음, updater 없음 | 지킴(`style-src 'unsafe-inline'`만 잔존) |
| 코드서명 정책 "현재 미서명" (CODE_SIGNING_POLICY.md:48-66) | release.yml:152-154,244,308-312이 미서명임을 고지 | 지킴(정책-CI 일치) |

### IPC 표면 요약

- `#[tauri::command]` 총 **59개**, 전부 `lib.rs:325` `collect_commands!`에 등록. `permissions/autogenerated/*.toml` 59개 ↔ `main.json` 59개가 **1:1 정확히 일치** — 고아 권한 0건, 누락 커맨드 0건(ACL 위생 양호).
- main capability 노출 59개 / quick capability 노출 **15개**.
- **quick(투명·always-on-top·decorations 없음) 노출 파괴적 커맨드 3종**: `execute_clean`(실삭제 + `docker prune` + `npm/pnpm/uv cache prune`), `create_delete_plan`(삭제 대상 임의 선택), `start_scan`. 나머지 12개는 읽기 전용.
- FE가 임의 문자열/경로를 넣는 지점: `terminate_process_group(name)`, `open_in_terminal(path)`, `show_in_file_manager(path)`, `post_agent_event(cwd)`. 그 외 삭제 계열은 전부 백엔드 소유 인벤토리의 opaque ID(`scan_id`/`item_id`/`plan_id`/`lease_id`)만 받는다.
- 런타임 검증은 specta 타입 외에 별도로 존재: `LargeFileScanRequest.roots`는 4개 토큰 화이트리스트(large_files/mod.rs:17), `selected_item_ids`는 직전 스캔과 대조(planner.rs:26-44), tool_id/signature_id는 고정 match.

### 총평

삭제 경계는 이 리뷰에서 가장 잘 만들어진 부분이다 — `remove_dir_all` 0건, `O_NOFOLLOW` dirfd, uid·dev/ino 검증, 1회성 플랜, 트러스트 인벤토리가 실제로 코드에 있고 문서 주장의 다수가 지켜진다.
결함은 경계 바깥에 몰려 있다: 프로세스 종료(`terminate_process_group`)만 lease 체계에서 통째로 빠져 있고, OAuth 콜백은 state·경로 검증이 없으며, 외부 바이너리는 PATH 우선 탐색이라 실행 파일 신뢰 정책이 모듈마다 다르다.
문서 대비 실제로 틀린 주장은 3건이 두드러진다 — "fail closed"(symlink 2곳 fail-open), "force 전 normal 우선"(강제 장치 없음), "리댁션이 토큰·비밀번호를 가린다"(Basic·신형 PAT·URL 자격증명 미탐지).
공급망 쪽은 서명·provenance 부재 자체보다, README:72가 자기가 만든 체크섬을 변조 방지 수단인 것처럼 안내하는 점이 더 문제다.
우선순위: ① `terminate_process_group` lease 전환 ② OAuth state+콜백 검증 ③ symlink fail-open 2곳 ④ `tooling::command` 실행파일 검증 ⑤ quick capability 축소.


---

# 부록 — 데이터 차원 원본 (data.md)

## Zenith 데이터 무결성·영속성·데이터 모델 리뷰

Critical | src-tauri/src/safety/toctou.rs:36 | Windows `capture()`가 `std::fs::File::open(path)`로 핸들을 여는데, Windows에서 디렉터리는 `FILE_FLAG_BACKUP_SEMANTICS` 없이 열리지 않아 항상 실패 → `(device, inode) = (0, 0)`이 되고, verify()의 `expected.device != 0 || expected.inode != 0` 가드(toctou.rs:108) 때문에 **모든 디렉터리 삭제 대상의 TOCTOU identity 검증이 무력화**된다(디렉터리는 mtime/size 비교도 하지 않음, toctou.rs:133). large_files/mod.rs:60처럼 `BACKUP_SEMANTICS|OPEN_REPARSE_POINT`로 열고, (0,0) identity는 검증 통과가 아니라 실패로 처리하라.

Critical | signatures/system.toml:9 | `system.developer_temp`의 `paths = ["$TMPDIR", "${TEMP}"]`가 platform/paths.rs:27에서 **둘 다 `std::env::temp_dir()`로 동일 확장**된다. 같은 stale 디렉터리가 idx 0/1로 두 번 스캔되어 ScanItem이 2개 생성되고(walker.rs:164) safe_bytes·total_bytes가 2배로 계상된다(두 번째 삭제는 "이미 없음" Success 0바이트). 두 토큰 중 하나만 남기고, `resolve_paths`에서 확장 결과를 dedup하라.

Major | signatures/ai.toml:50 | macOS에서 `${ROAMING_APP_DATA}`는 `~/Library/Application Support`로 확장되므로(paths.rs:243) `ai.cursor.cache`의 Cache/Code Cache/GPUCache와 `ai.cursor.logs`의 logs가 리터럴 경로와 **완전히 같은 경로로 중복 등록**되어 macOS에서 4개 경로가 이중 계상된다. Windows 전용 행에는 `platforms = ["windows"]`를 명시하라.

Major | src-tauri/src/scanner/engine.rs:66 | 시그니처 간·시그니처 내 확장 경로에 대한 정규화·중복 제거가 전혀 없고 total_bytes는 단순 합산이다. 스캔 시작 시 canonical 경로 집합으로 dedup하고, 이미 계상된 상위 경로의 하위 경로도 배제하라.

Major | src-tauri/src/scanner/size.rs:48 | `blocks() * 512`를 파일마다 그대로 더하고 `nlink`·clone 판정이 코드 전체에 존재하지 않는다. 하드링크 스토어(pnpm/bun/cargo)와 APFS clone은 링크 수만큼 중복 계상되어 "reclaimable"이 실제 회수량보다 크게 나온다. 측정 패스에서 (dev, ino) 집합으로 중복 제거하거나, 해당 시그니처에 `reclaimable_is_lower_bound`/`Informational` 시맨틱을 강제하라.

Major | src-tauri/src/scanner/size.rs:194 | 측정 쪽 `is_excluded`는 `child_str.contains(exclusion)` 부분문자열 매칭까지 허용하는데, 삭제 쪽 `is_excluded`(safety/tree_deleter.rs:450)는 확장된 절대경로 prefix와 파일명 완전일치만 본다. 즉 **스캔에서 크기에서 뺀 항목을 삭제 단계는 지울 수 있다**. 공용 exclusion matcher 하나로 통일하라.

Major | src-tauri/src/scanner/walker.rs:287 | aged-children 경로의 `measure_tree_stats` exclusion 검사는 `~`/절대경로 확장을 하지 않아(size.rs:183과 다름) 삭제 단계에서 skip될 파일까지 크기에 포함시킨다. 같은 matcher로 통일하고 예상 회수량을 삭제 규칙과 일치시켜라.

Major | src-tauri/src/docker/adapter.rs:283 | `docker system df`의 사람이 읽는 크기는 SI(GB=10^9)인데 `parse_docker_size`가 `GB`를 1024^3으로 환산한다. Docker 용량·reclaimable이 약 7.4%(TB는 10%) 과대 표시된다. `GB/MB/kB`는 1000배, `GiB/MiB/KiB`만 1024배로 분리하고, 알 수 없는 단위는 0이 아니라 오류로 처리하라.

Major | src-tauri/src/models_inventory/scanner.rs:108 | Ollama 모델 크기를 태그별 manifest의 layer size 합으로 계산한다. blobs는 content-addressed라 태그·모델 간 공유되므로 **공유 레이어가 모델 수만큼 중복 합산**된다. 전체 manifest를 훑어 layer digest 기준으로 dedup한 뒤 배분하라.

Major | src-tauri/src/ai_control_center/budgets.rs:59 | `spent_micros`가 기간 필터 없이 매칭 observation을 전부 합산하고 `budget.period`는 결과 행에 그대로 복사만 된다(budgets.rs:80). 일/주/월 예산이 동일한 spent·used_basis_points를 갖는다(테스트도 period 라벨만 검증). observation의 period 창으로 필터링하거나 기간별 정규화를 넣어라.

Major | src-tauri/src/ai_control_center/budgets.rs:59 | 동일 provider의 live observation과 사용자 수기 입력(manual)이 같이 합산되어 지출이 이중 계상된다(`mixed_sources`는 플래그일 뿐 보정 없음). provider별로 소스 우선순위를 정해 하나만 합산하라.

Major | src-tauri/src/ai_control_center/audit.rs:20 | 감사 파일이 512KB를 넘거나(line 20) 파싱에 실패하면(line 27 `unwrap_or_default`) **감사 이력 전체를 백업 없이 조용히 폐기**하고 다음 save에서 덮어쓴다. settings_store.rs:36처럼 `.corrupt.<ts>`로 보존하고 진단에 노출하라.

Major | src-tauri/src/commands/ai.rs:504 | `let _ = audit_store.save(&config_dir);` 로 쓰기 오류(용량 초과 audit.rs:78 포함)를 버린다. 메모리에는 남고 디스크에는 없는 상태가 무음으로 발생한다. 실패를 로깅하고 스냅샷 품질에 반영하라.

Major | src-tauri/src/cleaner/executor.rs:81 | `CleanResult`는 메모리·IPC에만 존재하고 실패만 zenith.log로 나간다(line 52). **무엇을 지웠는지에 대한 영속 기록이 없어 사후 추적이 불가능**하다. plan_id·경로·바이트·시그니처를 담는 append-only 삭제 저널을 남겨라.

Major | src-tauri/src/settings_store.rs:98 | temp 파일 `fsync`도, rename 후 디렉터리 `fsync`도 없어 크래시 시 0바이트 settings.json이 남을 수 있고, temp 이름이 고정(`settings.json.tmp`)이라 동시 쓰기 시 서로를 침범한다. 유니크 temp 이름 + `sync_all()` + 디렉터리 fsync를 적용하라(ai_control_center/audit.rs:75도 동일).

Major | src/lib/stores/settings.svelte.ts:162 | 저장이 **문서 전체 스냅샷 전송**이고, 각 WebView(메인/퀵패널)가 기동 시 1회 로드한 자기 사본에서 병합한다. 변경 브로드캐스트가 없어 뒤에 저장한 창이 앞선 창의 변경을 되돌린다(commands/system.rs:214는 검증 없이 그대로 기록). revision/If-Match 검증이나 필드 단위 patch 커맨드로 바꿔라.

Major | src/lib/stores/settings.svelte.ts:122 | FE 폴백이 `dashboard_tabs_revision ?? 3`(백엔드 현재 5, models/settings.rs:226)이고 기본 탭 목록에 폐기된 `ai_control`·`usage`가 남아 있다. 폴백 경로로 저장되면 마이그레이션 revision이 되감겨 4·5가 재실행되고 사용자가 숨긴 탭이 되살아난다. FE 기본값을 제거하고 백엔드 응답만 신뢰하라.

Major | src-tauri/src/models/settings.rs:100 | 구조체 레벨 `#[serde(default)]`뿐이고 top-level schema_version도 unknown-field 보존도 없다. 신버전이 쓴 필드는 구버전 로드 시 무시되고 다음 save에서 **영구 소실**된다(다운그레이드 데이터 유실). unknown 필드를 `serde_json::Value`로 보존하거나 schema_version 가드를 두라.

Minor | src-tauri/src/settings_store.rs:18 | JSON 파싱 오류 하나로 설정 전체가 기본값으로 교체된다(백업은 남지만 사용자 인지 경로는 진단 화면 한 곳뿐, diagnostics/mod.rs:211). 필드 단위 관용 파싱과 명시적 인앱 알림을 추가하라.

Minor | src-tauri/src/storage_commands.rs:155 | `now.saturating_sub(created_at) < ttl`은 시계가 뒤로 점프하면 0을 반환해 **영구 fresh**가 된다(fail-open). models/scan.rs:148의 `checked_sub` fail-closed와 불일치하고, trash_manager/mod.rs:70의 플랜 만료도 같은 문제다. `Instant` 기반 TTL 또는 `checked_sub`로 통일하라.

Minor | src-tauri/src/agent_activity/hooks.rs:172 | 사용자 소유 3rd-party 설정(`~/.claude/settings.json` 등)을 `serde_json::Value` 왕복으로 재작성한다. `preserve_order` 피처가 없어(src-tauri/Cargo.toml:24) 키 순서가 알파벳 정렬로 바뀌고, 새 파일이 umask 기본 권한을 받아 원본의 제한 권한(0600 등)을 잃는다. preserve_order + 원본 권한 복사 + fsync를 적용하라.

Minor | src-tauri/src/ai_usage/mod.rs:594 | `parse_stat_u64`/`parse_stat_f64`가 양끝 비숫자만 trim하므로 `1,234`, `12 (3 active)`, 로케일 소수점 콤마는 파싱 실패 → `None`으로 조용히 사라진다. 숫자 런 정규식으로 추출하고 파싱 실패를 로깅하라.

Minor | src-tauri/src/ai_usage/mod.rs:191 | `last_7d_tokens`가 `dailyUsageBuckets`를 `.rev().take(7)`로 자르며 오름차순·일 단위 버킷을 가정하고, `tokens` 누락 버킷은 filter_map으로 무음 제외한다. CLI 포맷 변경 시 집계 창이 조용히 어긋난다. 버킷의 날짜 필드로 필터링하고 누락은 품질 플래그로 보고하라.

Minor | src-tauri/src/agent_activity/events.rs:82 | `timestamp` 단위 검증이 없어 훅이 JS 밀리초를 보내면 전부 "in the future"로 거절되고, 사용자에게는 이벤트 0건으로만 보인다. 자릿수 기반 ms→s 정규화 또는 명시적 단위 오류 메시지를 넣어라.

Minor | src-tauri/src/agent_activity/projects.rs:188 | `opaque_id`가 salt 없는 SHA-256(절대경로)의 상위 64비트라 "경로를 감춘다"는 목적이 흔한 경로 사전 대입으로 깨지고, 프로젝트를 옮기면 id가 바뀌어 audit `project_ref` 이력이 끊긴다. 설치별 랜덤 salt를 섞어라.

Minor | src-tauri/src/cache_providers/mod.rs:143 | 같은 시그니처(dev.uv.cache 등)가 어댑터 경로에서는 `size_semantics = Informational`, models/signature.rs:54의 `cache_metadata()`에서는 `ConservativeLowerBound`로 서로 다른 의미를 보고한다. 메타데이터 산출을 한 곳으로 모아라.

Minor | src-tauri/src/scanner/engine.rs:87 | 어댑터 아이템은 `!item.exists || bytes == 0` 필터(line 69) 없이 합산되고 risk와 무관하게 `cat_rebuild`로 하드코딩된다. 시그니처 아이템과 동일한 집계 규칙을 적용하라.

Minor | src/lib/utils/format.ts:5 | 1024 진수로 나누면서 라벨은 SI(KB/MB/GB)다. OS·Finder의 1000진수 GB 표기와 같은 볼륨이 다른 숫자로 보인다. KiB/MiB/GiB 라벨을 쓰거나 디스크 지표는 1000진수로 맞춰라.

Minor | src/lib/api/mock.ts:535 | mock의 크기가 `2.1 * 1024**3 = 2254857830.4`처럼 소수다. 실제 계약은 u64 정수(models/scan.rs:63)라 정수 전용 반올림·포맷 버그를 mock이 가린다. `Math.round`로 정수화하라.

Minor | src-tauri/src/signatures/registry.rs:62 | `load_from_dir`가 파일별 파싱 오류를 `if let Ok(...)`로 무음 처리한다(현재 호출부 없음, lib.rs:127은 embedded만 로드). 외부 시그니처 드롭인을 지원할 계획이면 부분 로드 대신 오류를 반환하고, 아니면 API를 제거하라.

### 영속 데이터 인벤토리

| 데이터 | 위치 | 포맷 | 원자쓰기 | 버전 | 비고 |
|---|---|---|---|---|---|
| 시그니처 규칙 | 바이너리 임베드 (`signatures/*.toml`, registry.rs:6-10) | TOML | 해당없음 | 없음 | 로드 실패 시 빈 레지스트리로 fail-closed(registry.rs:19). `load_from_dir`는 미사용 |
| 앱 설정 | `app_config_dir()/settings.json` | JSON | temp+rename, fsync 없음, temp명 고정 | `dashboard_tabs_revision`(u8, 현재 5) — 탭에만 적용 | 파싱 실패 시 `settings.corrupt.<ts>.json`로 백업 후 기본값 복구 |
| 설정 손상 백업 | `app_config_dir()/settings.corrupt.*.json` | JSON | rename(실패 시 copy) | 없음 | 무한 누적, 정리 로직 없음. 개수만 진단에 노출 |
| AI Control 감사로그 | `app_config_dir()/ai-control-audit.json` | JSON 배열(VecDeque) | temp+rename, fsync 없음 | 없음 | 1024건·512KB 캡, append-only 아닌 전체 재작성. 초과/파싱실패 시 전량 폐기, save 오류 무시 |
| 진단 로그 | macOS `~/Library/Logs/Zenith/zenith.log`, Win `%LOCALAPPDATA%/Zenith/Logs`, 기타 `~/.local/share/zenith/logs` | 텍스트 라인 | append(O_APPEND) | 없음 | 1MB 초과 시 `zenith.log.1` 1세대만 회전, 시크릿 정규식 마스킹 |
| 3rd-party 훅 설정 | `~/.claude/settings.json`, `~/.cursor/hooks.json` 등 | JSON | temp(uuid)+rename, fsync 없음 | 없음 | 제거만 지원(설치는 거부). 키 순서 재정렬·권한 소실 |
| 스캔 결과 / DeletePlan | 메모리(AppState) | Rust 구조체 | 해당없음 | scan_id + 300초 TTL(checked_sub, fail-closed) | 디스크 미영속 |
| Trash 플랜 / 인벤토리 | 메모리(StorageWorkflowState) | Rust 구조체 | 해당없음 | PLAN_TTL 300초·INVENTORY_TTL 900초(saturating_sub, fail-open) | 최대 64건 LRU |
| 사용량/활동 스냅샷 캐시 | 메모리(`Mutex<Option<T>>` + generation) | Rust 구조체 | 해당없음 | generation 카운터 | usage TTL 60초, 구세대 완료본은 캐시 갱신 불가 |
| 개발 포트 리스 | 메모리(dev_ports/store.rs) | Rust 구조체 | 해당없음 | uuid v4 + Instant TTL 30초 | 단조시계 사용(정상), 용량 128 |
| OpenRouter OAuth 키 | 메모리(`Arc<Mutex<Option<String>>>`, lib.rs:132) | 문자열 | 해당없음 | 없음 | 디스크 비영속(재기동 시 재인증) — 시크릿 at-rest 없음 |
| 삭제 실행 결과 | 없음(IPC 전송 후 소멸) | — | — | — | 실패만 zenith.log에 남고 성공 삭제 내역은 기록 없음 |

### 시그니처 위험 항목 (risk = safe 인데 재검토 필요)

- `system.developer_temp` — `$TMPDIR`/`${TEMP}` 하위 디렉터리를 prefix 매칭으로 **트리째 삭제**(delete_directory)하면서 자동 선택 대상이다. `claude-`, `codex-`, `pytest-of-`, `com.google.Chrome.`처럼 3일 미사용이어도 재현 자료·테스트 아티팩트일 수 있는 prefix가 포함된다. 게다가 경로 중복으로 두 번 대상에 오른다.
- `system.intensive.user_app_caches` — `~/Library/Caches/<bundle>` 전체 트리를 7일 미사용 기준으로 삭제하는데, 캐시 디렉터리에 라이선스·세션 상태를 두는 앱이 흔하다. `safe`는 자동 선택(risk.rs:31)을 의미하므로 `rebuild` 이상이 적절하다.
- `system.intensive.application_logs` — `~/Library/Logs` 하위 트리 통삭제. Diagnostic/Crash만 제외되어 앱 자체 진단 이력이 사라진다. 위험은 낮으나 자동 선택은 과하다.
- `ai.gemini.cache` — 설명이 "cached search/API tokens"인데 risk=safe다. exclusions 대부분이 삭제 대상 디렉터리 밖 경로라 실제 보호 효과가 없고, `~/.gemini/cache` 통삭제로 재인증이 필요해질 수 있다.
- `ai.claude.cache` / `ai.opencode.cache` — 각각 `~/.claude/tmp`, `~/.opencode/logs`를 포함. exclusions에 적힌 `~/.claude/prompts`·`~/.claude/skills`는 삭제 대상 경로의 하위가 아니어서 의미 없는 선언이다(의도 오해 소지).
- `dev.xcode.caches` — `~/Library/Developer/CoreSimulator/Caches` 포함. 시뮬레이터 런타임 자산 재다운로드가 필요할 수 있어 `rebuild`가 맞다.
- `container.docker.builder` / `container.docker.dangling_images` — risk=safe로 자동 선택되지만 실제 회수량은 Docker CLI 파싱값(1024/1000 진수 오류 포함)에 의존한다.

### 총평

- 안전장치(블랙리스트·심볼릭링크·app 번들 fail-closed·TOCTOU·플랜 TTL) 설계 자체는 촘촘하나, **Windows 디렉터리 identity가 항상 (0,0)이 되어 TOCTOU 검증이 통째로 비는** 결함이 가장 치명적이다.
- 두 번째 축은 "수치 신뢰성"이다. `$TMPDIR`/`${TEMP}` 및 macOS `${ROAMING_APP_DATA}` 중복, 하드링크·APFS clone 미고려, Docker SI/IEC 혼동, Ollama 공유 레이어 중복으로 reclaimable이 구조적으로 과대 표시된다.
- 측정 경로와 삭제 경로의 exclusion 매칭 규칙이 서로 달라, "크기에서 제외했다"가 "삭제하지 않는다"를 보장하지 않는다 — 스캔 화면의 약속과 실제 동작이 어긋나는 지점이다.
- 영속 계층은 temp+rename까지는 갖췄으나 fsync·유니크 temp명·스키마 버전·다중 창 병합이 모두 빠져 있고, 감사로그는 손상·초과 시 무음 전량 폐기라 감사 목적을 달성하지 못한다.
- IPC 수치 안전성(`ipc_numeric` + CI의 bindings diff 검증)은 이 코드베이스에서 가장 잘 통제된 부분이며, 남은 위험은 mock의 소수 바이트와 FE 설정 기본값 드리프트 정도다.


---

# 부록 — 운영체제 차원 원본 (os.md)

## Zenith — 운영체제·시스템 프로그래밍 리뷰

대상: Tauri 2 + Rust (`src-tauri/src`, ~30k줄) / macOS ARM64 · Windows x64
관점: 파일시스템 삭제 정확성 · 프로세스 관리 · 리스너 탐지 · 전원관리 · 메트릭 · 외부 CLI · 앱 생명주기

## 발견 사항

Critical | src-tauri/src/safety/blacklist.rs:114 | [공통] `is_blacklisted`의 모든 비교(:48 AppData, :114 sensitive_relative, :143 users root, :170 system_prefixes)가 대소문자 구분 문자열 비교다. APFS는 기본 대소문자 비구분, NTFS도 비구분이므로 `~/documents/...`, `~/.SSH`, `C:/windows/...`는 보호 목록을 통과해 삭제 대상이 된다. macOS/Windows 경로는 정규화 단계에서 케이스 폴딩한 뒤 비교하고, 유닛 테스트에 대소문자 변형 케이스를 추가하라.

Critical | src-tauri/src/dev_ports/termination.rs:148 | [Windows] `get_process_info`가 Windows에서 uid를 상수 `Some(1000)`으로 채우고(:288, :345도 동일), `current_uid()`도 1000을 반환한다(:74). 그 결과 :375의 "현재 사용자 소유 리스너만 노출" 필터가 완전히 무력화되어 SYSTEM/타 계정 서비스의 리스너가 목록에 오르고 종료 후보가 된다. 같은 저장소의 `agent_activity/mod.rs:406 verified_process_uid`가 이미 SID 동등성 비교로 올바르게 구현되어 있으니 그 방식으로 통일하라.

Critical | src-tauri/src/tooling.rs:222 | [macOS] `try_wait`가 `Some(status)`를 돌려준(=이미 reap된) 뒤에 `cleanup_and_drain(true)`가 `libc::kill(-pid, SIGKILL)`을 호출한다(:195). reap된 PID/PGID는 즉시 재사용될 수 있어 무관한 프로세스 그룹을 통째로 SIGKILL할 수 있고, 이 경로는 정상 종료 시마다 실행된다(:233, :243도 wait 이후 호출). 정상 경로는 `kill_tree=false`로 호출하고, 타임아웃 경로에서는 `wait()` 이전에 그룹 kill을 끝내라.

Critical | src-tauri/src/safety/tree_deleter.rs:159 | [공통] `prepare_directory`가 `O_DIRECTORY|O_NOFOLLOW`로 부모 디렉터리 fd를 이미 보유하고 있는데도(:231) 실제 삭제는 `fs::remove_file(path)` / `fs::remove_dir(path)`(:210)로 절대경로를 재해석한다. `verify_entry_identity`(:154)와 unlink 사이에 경로 구성요소를 교체하는 TOCTOU 창이 남는다. 보유 중인 dirfd로 `libc::unlinkat(dirfd, name, 0 | AT_REMOVEDIR)`을 호출해 경로 재탐색 자체를 없애라.

Major | src-tauri/src/safety/tree_deleter.rs:294 | [Windows] `prepare_directory`의 `#[cfg(not(unix))]` 분기가 완전한 no-op이다. `FILE_ATTRIBUTE_READONLY` 해제, 디렉터리 dev/ino(볼륨시리얼+FileId) 재검증, 소유자 확인이 모두 빠져 읽기 전용 캐시 트리 삭제가 실패하고 identity 검증 방어도 사라진다. `GetFileInformationByHandle` 기반 검증(`toctou.rs:261`에 이미 구현됨)을 재사용하고 읽기 전용 속성을 해제/복원하라.

Major | src-tauri/src/safety/tree_deleter.rs:341 | [Windows] `validate_entry_owner`가 `#[cfg(unix)]` 전용이라 Windows에서는 삭제 대상의 소유권을 전혀 확인하지 않는다(`_ = (path, metadata)` 후 무조건 Ok). 최소한 파일 소유자 SID와 현재 토큰 SID를 `GetSecurityInfo`로 비교하거나, Windows에서는 사용자 프로필 하위로 대상 범위를 강제하라.

Major | src-tauri/src/safety/tree_deleter.rs:159 | [Windows] 삭제 실패 원인을 구분하지 않는다. 열린 핸들이 있으면 `ERROR_SHARING_VIOLATION(32)`, 이미 delete-pending이면 `ERROR_ACCESS_DENIED`가 뜨는데 둘 다 그냥 `errors`에 문자열로 쌓이고 재시도가 없다. 또 `\\?\` 접두어를 붙이지 않아 260자를 넘는 `node_modules` 심층 경로 삭제가 실패한다. 경로를 verbatim으로 변환해 삭제하고, sharing violation은 짧은 백오프 재시도 대상으로 분리하라.

Major | src-tauri/src/safety/tree_deleter.rs:468 | [macOS] 회수 바이트를 `blocks() * 512`로 보고한다. APFS 클론(cp -c, 앱 설치본)과 Time Machine 로컬 스냅샷이 같은 블록을 참조 중이면 파일을 지워도 실제 여유 공간은 늘지 않는다. `cleaner/executor.rs:76`이 이미 실측 `actual_disk_free_delta`를 계산하므로, UI에는 실측치를 우선 노출하고 blocks 기반 값은 "예상"으로 명시 구분하라.

Major | src-tauri/src/safety/blacklist.rs:78 | [macOS] ADS 방어용 `path_str.rfind(':')`가 인덱스 1이 아니면 무조건 블랙리스트 처리한다. macOS POSIX 계층에서 ':'는 합법적인 파일명 문자라 정상 캐시 경로가 통째로 차단된다(Finder에서 '/'로 보이는 이름). 이 검사는 `#[cfg(target_os = "windows")]` 안으로 옮겨라.

Major | src-tauri/src/agent_activity/termination.rs:229 | [공통] 8단계 주석과 에러 문구(:231)가 "target process **or its parent**"라고 말하지만, `is_terminal_or_protected`(:130)는 대상 자신의 `name`/`executable`만 본다. `ProcessCheckInfo.parent_pid`를 :125에서 수집해 놓고 한 번도 쓰지 않는다. 터미널에서 기동한 에이전트의 부모 보호가 실제로는 동작하지 않으니 parent_pid를 따라 올라가며 검사하거나 문구를 실제 동작에 맞게 고쳐라.

Major | src-tauri/src/agent_activity/termination.rs:178 | [Windows] `send_sigterm`이 `#[cfg(unix)]` 전용이고 그 외 플랫폼에서는 `Err("Graceful stop is only supported on Unix systems.")`를 반환한다. Windows를 정식 배포 타깃으로 두면서 에이전트 정지 기능이 통째로 공백이다. `GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT)` 또는 대상 창에 `WM_CLOSE` 전송 경로를 추가하거나, `platform/capabilities.rs`에서 명시적으로 Unavailable로 노출하라.

Major | src-tauri/src/dev_ports/termination.rs:207 | [Windows] Graceful 모드(signal 15)와 Force 모드(signal 9)가 모두 `TerminateProcess`로 귀결된다(:189~:207). dev 서버가 정리 훅 없이 즉사해 빌드 산출물·SQLite/락 파일이 손상될 수 있다. Job Object 또는 `CTRL_BREAK_EVENT`로 graceful 단계를 실제로 구현하고, 실패 시에만 `TerminateProcess`로 승격하라.

Major | src-tauri/src/metrics/memory.rs:362 | [공통] `terminate_group`이 정규화된 "이름"으로 매칭해 일괄 시그널을 보낸다. `normalize_process_name`이 부분문자열 기반(:217 `contains("node")`, :200 `contains("docker")`, :231 `contains("terminal")`)이라 무관한 프로세스가 같은 그룹으로 묶여 함께 종료된다. 또 uid 필터가 없고, SIGTERM 후 대기·SIGKILL 승격·실제 종료 확인이 없어 반환값 `signaled`가 "종료됨"을 보장하지 않는다. 리스너 종료 경로처럼 PID+start_time+exe 리스 방식으로 바꾸고 종료 확인 폴링을 넣어라.

Major | src-tauri/src/metrics/memory.rs:354 | [공통] `terminate_group`과 `sample`(:47), `applications/mod.rs:67`이 매 호출마다 `System::new_all()` + `refresh_processes(All, true)`로 전체 프로세스를 스캔한다. macOS에서는 `proc_listallpids` + 프로세스당 `proc_pidinfo`·`KERN_PROCARGS2` 조회라 비용이 크고, 상주 트레이 앱의 폴링 경로라 누적된다. 재사용 `System` 인스턴스 + 축소된 `ProcessRefreshKind`(메모리/이름만)로 바꿔라.

Major | src-tauri/src/power/watcher.rs:261 | [macOS] 규칙이 하나라도 활성이면 최대 5초 주기(:450)로 `System::new()` + 전 프로세스 스캔을 반복한다. Keep Awake 기능이 오히려 타이머 병합(timer coalescing)과 App Nap을 깨고 배터리를 소모하는 자기모순이다. `NSWorkspace`의 `didLaunchApplicationNotification`/`didTerminateApplicationNotification`을 구독하거나, 최소 30초 주기 + 프로세스 이름 집합 캐시로 낮춰라.

Major | src-tauri/src/power/assertion.rs:59 | [macOS] `PreventUserIdleSystemSleep`만 사용한다. 이 어서션은 "유휴 슬립"만 막으므로 노트북 뚜껑을 닫거나(clamshell) 사용자가 명시적으로 슬립을 걸면 그대로 잠든다. 사용자는 "Keep Awake = 안 잠듦"으로 이해하므로 UI에 한계를 명시하거나, 필요 시 `PreventSystemSleep`(전원 연결 시)까지 선택지로 제공하라.

Major | src-tauri/src/metrics/memory.rs:122 | [macOS] 메모리 압력을 `used/total` 비율과 스왑 휴리스틱으로 산출한다. macOS는 파일 캐시·비활성 페이지가 used로 잡히는 특성 때문에 정상 상태에서도 0.75를 쉽게 넘겨 상시 Warning 오탐이 난다. 실제 지표인 `kern.memorystatus_vm_pressure_level` sysctl 또는 `host_statistics64(VM_STATISTICS64)`의 compressor/purgeable 페이지 기반으로 교체하라.

Major | src-tauri/src/metrics/memory.rs:388 | [macOS] compressed 메모리를 `vm_stat` 텍스트 파싱으로 얻는다(8초 캐시). 출력 포맷·로케일 변화에 취약하고 8초마다 fork/exec 비용이 든다. `host_statistics64`의 `compressor_page_count`를 직접 읽고 페이지 크기도 `vm_page_size`(또는 `_SC_PAGESIZE`)로 일관되게 쓰면 서브프로세스가 사라진다.

Major | src-tauri/src/metrics/disk.rs:36 | [macOS] APFS 컨테이너 공유를 고려하지 않고 sysinfo 볼륨을 그대로 나열한다. 같은 컨테이너에 속한 Data/System/VM/Preboot/Recovery 볼륨이 각각 동일한 여유 공간을 보고하므로 목록 표시·합산이 심하게 과대해진다. 컨테이너(BSD 디스크 그룹) 단위로 묶어 한 번만 집계하고 synthetic 볼륨을 필터링하라. purgeable(정리 가능) 공간도 미반영이라 Finder 수치와 어긋난다.

Major | src-tauri/src/dev_ports/discovery.rs:116 | [공통] `classify_address`가 IPv4-mapped IPv6(`::ffff:127.0.0.1`)와 link-local(`fe80::`)을 모두 `Network`로 분류해 실제 루프백 바인딩을 "외부 노출"로 오탐한다. Windows 경로(termination.rs:332)는 이미 `Ipv6Addr::is_loopback()`을 쓰므로, macOS/lsof 경로도 문자열 비교 대신 `Ipv6Addr` 파싱 + `is_loopback()`/`to_ipv4_mapped()`로 통일하라.

Major | src-tauri/src/tooling.rs:601 | [공통] `resolve()`에 캐시가 없어 `command()` 호출마다 PATH 전체 + homebrew·cargo·volta·asdf 후보를 stat하고, nvm 후보는 `read_dir`까지 수행한다(:263, :298). GUI PATH 축소 문제를 후보 목록으로 잘 보완했지만 고빈도 수집기 경로에서는 syscall이 폭증한다. `OnceLock<Mutex<HashMap<String, PathBuf>>>` 캐시 + 실패 시 짧은 negative TTL을 두어라.

Major | src-tauri/src/diagnostics/mod.rs:219 | [Windows] `open_logs_folder`가 cfg 분기 없이 `Command::new("open")`을 실행한다. 같은 파일 :13~:19가 Windows 로그 경로(`%LOCALAPPDATA%\Zenith\Logs`)는 제대로 분기해 놓았는데 여는 쪽만 macOS 전용이라 Windows에서는 항상 실패한다. `explorer.exe <dir>` 분기를 추가하라.

Major | src-tauri/src/platform/system_actions.rs:31 | [macOS] `open`, `explorer.exe`, `wt.exe`, `powershell.exe`를 `spawn()`만 하고 `wait()`하지 않는다(:31, :46, :69, :79, :86, :95, :112). 상주 트레이 프로세스라 자식이 좀비로 계속 누적되고, 반환된 `Ok(())`는 "명령이 성공했다"가 아니라 "fork에 성공했다"만 뜻한다. `tooling::run_with_timeout`을 쓰거나 최소한 별도 스레드에서 `wait()`로 회수하라.

Major | src-tauri/tauri.conf.json (bundle 섹션) | [macOS] `bundle.macOS` 항목 자체가 없어 entitlements, hardened runtime, `minimumSystemVersion`이 모두 미지정이다. 이 앱은 다른 앱 번들의 Info.plist를 읽고 Desktop/Documents 등 TCC 보호 폴더를 스캔하므로, 서명·공증이 없으면 사용자에게 프롬프트조차 뜨지 않고 조용히 EPERM으로 실패한다. hardened runtime + 필요한 usage description(AppleEvents 사용 시 `NSAppleEventsUsageDescription`)을 명시하라.

Minor | src-tauri/src/safety/blacklist.rs:100 | [공통] `sensitive_relative` 목록에 Desktop/Documents/Pictures/Movies는 있는데 `Downloads`가 없다. macOS 14+ 및 Windows 모두에서 Downloads는 사용자 콘텐츠·TCC 대상 폴더이므로 동일하게 보호 목록에 추가하라.

Minor | src-tauri/src/dev_ports/termination.rs:600 | [macOS] 종료 유예 폴링이 100ms × 15회 동안 매 반복 `discover_listeners()`를 호출해 `/usr/sbin/lsof`를 최대 15회 fork/exec 한다(각 호출 타임아웃 2초). 프로세스 소멸 확인만 필요하므로 `kill(pid, 0)` 또는 sysinfo 단건 조회로 바꾸고, 리스너 재확인은 루프 종료 후 1회만 수행하라.

Minor | src-tauri/src/dev_ports/termination.rs:197 | [Windows] `OpenProcess` 실패 시 `ERROR_ACCESS_DENIED(5)`만 에러로 처리하고 나머지는 전부 "이미 종료됨"으로 간주해 `Ok(())`를 반환한다(:200~:203). 핸들 고갈이나 잘못된 인자 같은 실제 실패가 성공으로 보고된다. `ERROR_INVALID_PARAMETER(87)`만 종료로 간주하고 나머지는 오류로 올려라.

Minor | src-tauri/src/dev_ports/termination.rs:288 | [Windows] `GetExtendedTcpTable`을 크기 조회 → 실제 조회 2단계로 부르면서, 두 호출 사이에 테이블이 커져 `ERROR_INSUFFICIENT_BUFFER`가 나면 `== 0` 비교에 걸려 조용히 빈 목록을 반환한다(IPv6 경로 :345도 동일). 재시도 루프(최대 3회, 크기 재조회)로 감싸라.

Minor | src-tauri/src/metrics/memory.rs:369 | [macOS] launchd `KeepAlive` 관리 프로세스나 Docker Desktop 헬퍼는 종료 직후 자동 재기동된다. 종료 후 재확인 단계가 없어 UI는 "종료됨"으로 표기하지만 실제로는 몇 초 뒤 되살아난다. 종료 후 짧은 재확인을 넣고 재기동이 감지되면 "시스템이 다시 시작했습니다"로 보고하라.

Minor | src-tauri/src/dev_ports/termination.rs:86 | [macOS] `lsof -F0pcuLn`에서 `c`(command) 필드는 기본 9자로 잘린다(`+c 0` 미지정). `classifier.rs`가 `Electron Helper` 같은 긴 커맨드명을 매칭하지 못한다. `+c 0`을 인자에 추가하거나, sysinfo 조회 실패 시의 폴백 분류(:368~:380)가 잘린 이름에 의존한다는 점을 명시하라.

Minor | src-tauri/src/platform/paths.rs:303 | [macOS] 홈 디렉터리를 `HOME` 환경변수에만 의존해 해석하고, 없으면 `None`을 반환해 로그·설정·스캔이 전부 비활성화된다(`diagnostics::log_dir`은 `/tmp`로 폴백). launchd 잡이나 환경이 비워진 컨텍스트를 대비해 `getpwuid_r` 폴백을 추가하라.

### 플랫폼별 구현 매트릭스

| 기능 | macOS 방식 | Windows 방식 | 위험 |
|------|-----------|-------------|------|
| 트리 삭제 | `symlink_metadata` + `O_DIRECTORY\|O_NOFOLLOW` dirfd + dev/ino 재검증 + uid 검사 → `remove_file`(경로) | 전부 no-op(`cfg(not(unix))`), `remove_file`만 | Windows에 소유자·identity·읽기전용 방어 부재, 양쪽 모두 unlinkat 미사용 TOCTOU |
| 심볼릭 링크 방어 | `file_type().is_symlink()` | + reparse point 비트(0x400), 8.3 별칭 폴백 + junction 테스트 | macOS firmlink·`/System/Volumes/Data` 이중 경로 미고려 |
| 리스너 탐지 | `/usr/sbin/lsof -F0pcuLn` (fork/exec, 2초 타임아웃) | `GetExtendedTcpTable` v4/v6 직접 호출 | macOS는 폴링마다 프로세스 생성·command 9자 절단, Windows는 버퍼 경쟁 재시도 없음 |
| 리스너 소유자 판정 | `lsof` u 필드 + `geteuid()` | **상수 1000 하드코딩** | Windows에서 소유자 필터 완전 무력화 (Critical) |
| 프로세스 종료(포트) | SIGTERM → 1.5초 폴링 → SIGKILL | `TerminateProcess`만 (graceful 없음) | Windows는 정리 훅 없이 즉사 |
| 프로세스 종료(에이전트) | SIGTERM 단독 + uid/start_time/exe/cwd/allowlist 검증 | **미구현(Err 반환)** | Windows 기능 공백 |
| 서브프로세스 격리 | `setpgid(0)` + `kill(-pid)` | Job Object + `KILL_ON_JOB_CLOSE` + `TerminateJobObject` | reap 이후 `kill(-pid)` 호출로 PID 재사용 오살상 위험 |
| Keep Awake | IOKit `IOPMAssertionCreateWithName` (`PreventUserIdleSystemSleep` / `PreventUserIdleDisplaySleep`) | `PowerCreateRequest` + `PowerSetRequest`(System/Display) + RAII `PowerClearRequest` | macOS 클램셸/명시적 슬립 미방어, Windows Modern Standby(S0ix) 미고려 |
| 전원 소스 | `IOPSCopyPowerSourcesInfo` → 실패 시 `pmset -g batt` 파싱 | `GetSystemPowerStatus` | macOS 폴백이 5초 주기로 fork 가능, `pmset` 상대경로 |
| 메모리 | sysinfo + `vm_stat` 파싱(compressed) + used/total 휴리스틱 | sysinfo만 | 압력 지표 오탐, macOS 실제 API(host_statistics64) 미사용 |
| 디스크 | sysinfo `Disks`, primary = `/` | sysinfo `Disks`, primary = `%SystemDrive%` | APFS 컨테이너 공유·purgeable 미반영, `/`는 봉인된 System 볼륨 |
| 앱 인벤토리 | `/Applications` + `~/Applications` 1단계 + plist 파싱 | Program Files / LocalAppData\Programs 디렉터리 열거 | macOS 하위 폴더 앱 누락, quarantine/코드서명 미확인 |
| CLI 탐색 | PATH + homebrew·cargo·volta·asdf·nvm 후보 | PATH + ProgramFiles·LOCALAPPDATA·nvm-windows + .exe/.cmd/.bat | 캐시 없음, Windows는 확장자 없는 파일을 실행 가능으로 오판 |
| 로그 폴더 열기 | `open <dir>` | **동일 코드(`open`) → 항상 실패** | Windows 기능 파손 |
| 시스템 액션 | `open -R` / `open -a Terminal` (wait 없음) | explorer / wt / powershell / cmd (wait 없음) | 좀비 누적, 성공 여부 미확인 |

### 시스템콜/API 선택 평가

**잘한 선택 3**

1. `power/windows_request.rs:265` — `SetThreadExecutionState` 대신 `PowerCreateRequest`/`PowerSetRequest`를 골랐다. 전자는 호출 스레드가 끝나면 어서션이 사라지는 함정이 있는데, 이 구현은 프로세스 스코프 핸들 + `OwnedHandle` RAII + 부분 획득 시 `active_count`만큼만 역순 해제(:293)라 크래시/조기 리턴/스레드 이동 어느 쪽에도 새지 않는다. 스레드 간 이동 테스트(:183)까지 있다.
2. `safety/tree_deleter.rs:231` — 디렉터리 권한을 임시로 올릴 때 경로 chmod가 아니라 `O_DIRECTORY|O_NOFOLLOW|O_CLOEXEC` fd를 열고 `fchmod`(`File::set_permissions`)한다. 심링크 교체로 남의 디렉터리 권한을 바꾸게 만드는 고전적 공격을 원천 차단하고, 소유자 비트만 OR로 추가해 setgid/sticky를 보존한다.
3. `scanner/walker.rs:222` + `large_files/mod.rs:227` — `.app` 번들을 만나면 하위 탐색을 중단하고 `complete=false`로 표시한다(macOS App Management/TCC 하에서는 소유자여도 EPERM). 그리고 `meta.dev() != root_device`로 크로스 디바이스 탐색을 차단해 firmlink/외장 볼륨으로의 무한 확장을 막는다. OS 제약을 알고 fail-closed로 설계한 흔적이다.

**아쉬운 선택 3**

1. `dev_ports/termination.rs:85` — 리스너 탐지에 `lsof` 서브프로세스를 골랐다. macOS에는 `libproc`의 `proc_pidfdinfo(PROC_PIDFDSOCKETINFO)`나 `sysctl net.inet.tcp.pcblist`가 있어 fork/exec·파싱·타임아웃·9자 절단 문제 없이 같은 정보를 얻는다. 특히 종료 유예 폴링(:600)에서 이 선택이 15배로 증폭된다. Windows 쪽이 `GetExtendedTcpTable`을 직접 부르는 것과 대비된다.
2. `metrics/memory.rs:388` + `power/source.rs:413` — compressed 메모리는 `vm_stat` 파싱, 전원 폴백은 `pmset -g batt` 파싱이다. 둘 다 Mach/IOKit 네이티브 API(`host_statistics64`, 이미 쓰고 있는 `IOPSCopyPowerSourcesInfo`)로 대체 가능하며, 텍스트 파싱은 로케일·포맷 변경에 깨진다. 같은 파일에서 `sysconf(_SC_PAGESIZE)`는 libc로 직접 부르면서 통계만 파싱하는 것도 일관성이 없다.
3. `safety/blacklist.rs:13` — 삭제 안전성의 최종 관문을 문자열 접두어 비교로 구현했다. 대소문자(Critical), macOS의 ':' 합법성, firmlink 이중 표기, Windows 8.3 별칭이 전부 이 한 함수의 문자열 비교에 걸린다. `symlink.rs`/`toctou.rs`가 dev/ino·FileId 같은 커널 식별자를 쓰는 수준에 비해 여기만 한 단계 낮다. 보호 대상을 경로 문자열이 아니라 dev/ino(또는 volume+FileId) 집합으로 캡처해 비교하면 세 문제가 동시에 사라진다.

### 총평

삭제 경로의 안전 설계는 이 규모의 데스크톱 유틸리티치고 이례적으로 견고하다 — dirfd 기반 fchmod, dev/ino 재검증, 일회성 플랜, 실측 여유공간 델타까지 갖췄다.
문제는 그 견고함이 Unix 분기에만 존재한다는 점이다. Windows는 소유자 검사·읽기전용 해제·identity 재검증·graceful 종료·에이전트 정지·로그 폴더 열기가 통째로 비어 있거나 파손돼 있고, 리스너 uid를 상수 1000으로 위조해 소유자 필터를 무력화한 것이 가장 심각하다.
macOS 쪽 결함은 성격이 다르다. 정확성보다는 "OS가 실제로 제공하는 지표 대신 텍스트 파싱과 휴리스틱을 쓴다"는 문제(vm_stat, pmset, used/total 압력, APFS 컨테이너 무시)와, 상주 트레이 앱이 5초마다 전체 프로세스를 스캔하는 전력 비용이다.
Critical 4건(블랙리스트 대소문자 우회, Windows uid 위조, reap 후 `kill(-pid)`, unlinkat 미사용)은 RC 전에 반드시 닫아야 한다. 특히 대소문자 우회는 삭제 대상 선정 전 구간에 영향을 준다.
권장 순서: 대소문자 정규화 → Windows uid/소유자 경로 정리 → `kill(-pid)` 호출 시점 수정 → unlinkat 전환 → macOS 네이티브 지표 이관.
