# Zenith 잔여 이슈 정리 — Delivery plan (로컬 전용, 커밋하지 않음)

기준: `develop` @ #186 머지 이후 (#185, #186 포함) · 작성 2026-09-12 · 근거: 코드/CI/문서 실측 감사

## 현재 open 이슈

- **#136** runtime efficiency (coalesce/lock/async/budget)
- **#166** failures and partial results observable
- **#168** Windows environmental verification

닫은 이슈: #163(superseded by #168), #169(completed), #170(completed).

## 이미 끝난 부분 (다시 만들지 말 것)

| 이슈 | 이미 landed |
| --- | --- |
| #136 | §2 coalesce(`collection.rs` SingleFlight, `ai_snapshots.rs` 서비스, generation 검증, progress buffer), §3 lock 축소(`ai_control_center/git.rs` baseline/collect/commit 분리 + 테스트), §5 대부분(공유 Rayon 풀 + storage/subprocess 세마포어) |
| #166 | incompleteness 표현(`ScanItem.quality`/`incomplete_reason` → `CategoryResult` → `ScanResult`), `TreeStats.complete` UI 도달, `ScanEvent::Error` 생성, capability error vs unsupported, I/O 오류≠safety violation, Keep Awake 실제 상태 |
| #168 | flavor 순수 path algebra + 불변식, injectable `PlatformEnvironment`/`SimulatedPaths`, fixture 5종 + table-driven 테스트, `--doctor`(+UI+이슈 템플릿), signature `platforms` lint, `test_hygiene.rs`, Windows Job Object 테스트, 설치→doctor→uninstall 스모크(per-user), Justfile 플랫폼 스코프 |

## PR 목록과 순서

```
PR-0  fix(verification): close the gate gaps that need no design      ✅ merged (#188)
A     fix(diagnostics): report startup and environment failures        ✅ PR #189 (green)
B     fix(core): stop presenting unknown state as success              (#166 1,4,6,7,8,11,12,18)  ← 다음
C2    test(windows): verify classifier/size and command-level env      (#168 part2, B 이후)
D     perf(core): measure the runtime and finish budget leftovers      (#136 §1 + 잔여)
E     refactor(tooling): migrate subprocess callers to async runner    (#136 §4)
E2    refactor(ai): provider collection off blocking HTTP/threads      (#136 §4 provider)
```

- `PR-0 ∥ A → B → C2`, 그리고 `B → D → E → E2`.
- B와 C2는 `models/`·SettingsView·스캔 렌더링을 공유하므로 병렬 금지.
- D/E/E2 규칙: 측정 없이 수치 주장 금지, `StorageOperationGate` 대체 금지, 숨은 quick panel은 여전히 무동작.

## PR-0 상세 (이번 작업 범위)

1. **golden drift gate 부재** — CI는 `tests::export_typescript_bindings`만 돌리고 `src/lib/bindings/tauri.ts`+`Cargo.lock`만 diff → `platform-capabilities.golden.json`은 조용히 stale 가능. export 테스트 실행 + diff 추가.
2. **`lib.rs` 거짓 주석** — "binding drift gate in CI covers this file at no additional cost" → 사실이 아님. 게이트 추가 후 문장 교정.
3. **`test_hygiene.rs` 사각지대** — `#[cfg(unix)] mod …`, `let Ok(x) = env::var(…) else {…}`, 한 줄 `} else { return; }` 감지 추가 + allowlist에 사유 기록 지원.
4. **dev-ports integration suite** — `tests/dev_ports_tests.rs:6`의 whole-module `#[cfg(unix)]`를 allowlist에 사유와 함께 명시(또는 Windows 대응).
5. **Justfile ↔ CI 통일** — `lint`/`test-rust` 레시피와 CI 명령이 다름 → CI가 레시피를 호출하도록 변경.
6. **machine-wide installer 스모크** — per-user만 설치 검증, machine-wide는 빌드/스테이징만 → 설치+doctor+제거 검증 추가.
7. **잔여 `cfg!(windows)` 경로 헬퍼** — `power/watcher.rs paths_equal`, `safety/tree_deleter.rs path_starts_with`, `platform/paths.rs`의 하드코딩 `C:\Program Files`/`C:\ProgramData` 폴백, `NativePlatformPaths::new()` 인라인 생성. (범위가 크면 승격 판단)

## 검증 명령 (PR-0)

```
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo clippy --manifest-path src-tauri/Cargo.toml --target x86_64-pc-windows-gnu --all-targets -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml --lib tests::export_platform_capability_golden -- --ignored --exact
pnpm check && pnpm test -- --run && pnpm build
just build-fast
```

## 후속 PR에서 반드시 지킬 것

- B는 IPC shape 변경 → `src/lib/bindings/tauri.ts` 재생성 + `src/lib/api/mocks` 키 패리티(프런트 계약 테스트).
- A는 `docs/WINDOWS.md`/`CODE_SIGNING_POLICY.md`가 CFA 처리를 문서로 주장 → 코드로 만들거나 문서에서 제거.
- C2는 B가 만든 상태를 assert → 반드시 B 이후.
