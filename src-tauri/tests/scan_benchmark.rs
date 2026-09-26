//! The scan benchmark: what one traversal costs, what it counts, and the
//! committed baseline those numbers are held to.
//!
//! Issue #227 makes a scan state its own bounds and its own measurements
//! (`ScanResult.metrics`, `ScanResult.cancelled`). A measurement nobody
//! compares against is a number, not a regression guard, so this harness does
//! three things:
//!
//! 1. It builds every fixture itself, in a `tempfile::TempDir`, and scans it
//!    through the real [`ScanEngine::scan`] with a registry that names only the
//!    fixture's own roots. Nothing here reaches the shipped catalog or the
//!    developer's home directory, so the numbers depend on the fixture and not
//!    on the machine's caches.
//! 2. `scan_metrics_match_the_committed_baseline` asserts the deterministic
//!    facts against `tests/fixtures/scan-baseline.json`, asserts the traversal
//!    honoured `max_concurrent_directory_reads`, and holds each fixture's
//!    duration under a generous ceiling. `export_scan_baseline` (ignored)
//!    regenerates that file; CI regenerates and diff-checks it exactly as it
//!    does the TypeScript bindings.
//! 3. It prints one JSON object per fixture, after a line naming the platform
//!    and the crate version, so a maintainer can read a real-machine baseline
//!    out of a CI log.
//!
//! # The fixtures
//!
//! | fixture | shape | what it asserts |
//! |---|---|---|
//! | `wide` | one root, 200 directories, three small files each | every directory is read once (`directories_read == 201`), nothing is skipped, the bytes are the tree's |
//! | `deep` | a chain 40 levels below the root, past `max_depth` | the depth limit is *reported*, and the file beyond it is not counted |
//! | `mixed_size` | files from 3 bytes to 2 MiB | logical and allocated populations are both tracked |
//! | `mixed_age` | a stale subtree and a fresh one, under an aged namespace signature | the stale subtree is reported stale and the fresh one is not |
//! | `overlapping_roots` | two signatures, one root inside the other | the nested unit is folded into the broader one and reported |
//! | `inaccessible` (unix) | a directory `chmod 000` | the refused read is attempted, counted, and reported |
//! | `symlink` (unix) | a link to a directory outside the root | the link is accounted for and never traversed |
//!
//! `inaccessible` and `symlink` cannot exist on a non-unix host (`chmod` and
//! `symlink` are the fixtures), so they are asserted by their own tests and are
//! not part of the committed baseline: that file is regenerated and diffed on
//! both the macOS and the Windows Rust job, and a row that only one of them can
//! produce would fail the other forever.
//!
//! # Why the numbers hold still
//!
//! The fixtures are written with fixed names and fixed sizes, and the walk's
//! counts are scheduling-independent: which directories are read and which
//! entries are visited is decided by the tree, not by which worker got there
//! first. Only `peak_outstanding_directory_tasks` moves between runs, and it is
//! therefore reported but never compared for equality — that is what the bound
//! assertion is for.
//!
//! The committed byte population is logical size, which is a property of the
//! fixture. Allocated bytes are measured and bounded separately: APFS and NTFS
//! can charge different amounts for the same small, sparse, or compressed file.
//! The portable baseline must never assume their allocation rules are equal.
//!
//! Traversal counts are scheduler-independent: the pooled and inline walks
//! count each filesystem entry once, so the committed baseline describes the
//! tree rather than the number of workers available on the host.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Instant, SystemTime};

use serde::{Deserialize, Serialize};
use tempfile::TempDir;
use zenith_lib::cleaner::{CleanExecutor, LifecycleProviderRegistry, OwnerProviderRegistry};
use zenith_lib::models::{
    CancellationProbe, Category, CleanStrategy, CleanupUnitKind, NeverCancelled, RiskTier,
    ScanEvent, ScanResult, Signature,
};
use zenith_lib::safety::SafetyPlanner;
use zenith_lib::scanner::{ScanEngine, ScanLimits};
use zenith_lib::signatures::SignatureRegistry;
use zenith_platform::path_algebra::PathFlavor;
use zenith_platform::PlatformEnvironment;

/// An end-to-end cleanup measurement on a directory created solely by this
/// harness. The elapsed time is evidence, not a CI threshold; the byte and
/// empty-root assertions ensure the measurement describes completed work.
#[test]
fn disposable_cleanup_fixture_reports_verified_reclaim() {
    const FILES: usize = 64;
    const FILE_BYTES: usize = 64 * 1024;

    let fixture = TempDir::new().expect("temporary fixture");
    let cache = fixture.path().join("generated-cache");
    std::fs::create_dir(&cache).expect("cache root");
    let payload = vec![0x5a; FILE_BYTES];
    for index in 0..FILES {
        std::fs::write(cache.join(format!("generated-{index:03}.bin")), &payload)
            .expect("generated file");
    }

    let mut registry = SignatureRegistry::new();
    registry.register(signature(
        "fixture.disposable.cache",
        "Disposable cache",
        Category::System,
        std::slice::from_ref(&cache),
        None,
        None,
        CleanStrategy::DeleteContents,
    ));
    let environment = PlatformEnvironment::native();
    let lifecycle = LifecycleProviderRegistry::new(Vec::new());
    let owners = OwnerProviderRegistry::new(Vec::new());
    let scan = ScanEngine::scan(
        &registry,
        &lifecycle,
        &owners,
        Some(&[Category::System]),
        &[],
        false,
        &environment,
        &NeverCancelled,
        |_| {},
    );
    let items: Vec<_> = scan
        .categories
        .into_iter()
        .flat_map(|category| category.items)
        .collect();
    assert_eq!(items.len(), 1);
    assert!(items[0].is_selected);
    let observed_before = items[0].size.observed_bytes();
    assert!(observed_before >= (FILES * FILE_BYTES) as u64);

    let plan = SafetyPlanner::create_plan(&items, &registry, &owners).expect("fixture plan");
    let started = Instant::now();
    let execution = CleanExecutor::execute(
        plan,
        &environment,
        &lifecycle,
        &owners,
        &zenith_platform::MockTrashBackend::new(),
        |_| {},
    );
    let elapsed_ms = started.elapsed().as_millis();
    let remaining = std::fs::read_dir(&cache)
        .expect("cache root retained")
        .count();
    assert_eq!(remaining, 0, "only the generated files were removed");
    assert_eq!(execution.items.len(), 1);
    assert!(execution.items[0].success);
    assert_eq!(execution.total_reclaimed_bytes, observed_before);
    eprintln!(
        "cleanup_fixture {}",
        serde_json::json!({
            "files": FILES,
            "logical_bytes": FILES * FILE_BYTES,
            "observed_allocated_bytes": observed_before,
            "reported_reclaimed_bytes": execution.total_reclaimed_bytes,
            "remaining_files": remaining,
            "execution_ms": elapsed_ms,
        })
    );
}

/// The committed baseline. Regenerated by [`export_scan_baseline`], diffed by
/// CI, and read by [`scan_metrics_match_the_committed_baseline`].
const BASELINE_PATH: &str = "tests/fixtures/scan-baseline.json";
/// The baseline's own format version: a change to the fields or to their
/// meaning is a new schema, not a silent edit.
const BASELINE_SCHEMA: u32 = 1;
/// The categories the fixtures declare their roots in. Both are exercised, and
/// `Container` and `Ai` are left out so no adapter or catalog provider can add
/// an item the fixture did not build.
const CATEGORIES: [Category; 2] = [Category::System, Category::Developer];
/// The generous ceiling on the cancellation latency, in milliseconds. The
/// measured value is the time from the probe tripping to `ScanEngine::scan`
/// returning; the bound is loose on purpose, because CI runners are shared and
/// a scheduler hiccup is not a regression.
const CANCELLATION_LATENCY_CEILING_MS: u64 = 1_000;
/// The wide fixture's directory count, named once so the cancellation assertion
/// can state what was *not* walked.
const WIDE_DIRECTORIES: usize = 200;
/// The allocated size of the fixture's anchor file (one 4096-byte block).
const ANCHOR_BYTES: u64 = 4_096;

/// A scan environment with no tools and no stated profile, so a fixture's scan
/// reports the fixture's own signatures and nothing the host happens to have.
fn environment() -> PlatformEnvironment {
    PlatformEnvironment::simulated(PathFlavor::current())
        .with_home(if cfg!(windows) {
            r"Z:\ZenithFixtureHome"
        } else {
            "/zenith-fixture-home"
        })
        .with_missing_tool("docker")
        .with_missing_tool("npm")
        .with_missing_tool("pnpm")
        .with_missing_tool("uv")
}

/// One fixture: the tree that must stay alive, the registry that names it, and
/// the ceiling this fixture's duration is held under.
struct Fixture {
    /// The fixture's name in the baseline and in the printed table.
    name: &'static str,
    /// The per-fixture duration ceiling in milliseconds: at least twenty times
    /// the local run's duration, rounded up to the next quarter second. CI
    /// runners are shared, and a Windows runner looks up each file's allocated
    /// size through the filesystem, so the ceiling exists to catch an
    /// order-of-magnitude regression rather than to pin a wall-clock time — the
    /// broad `wide` fixture carries the loosest ceiling in the set because it is
    /// the most metadata-bound walk and the one most likely to move with the
    /// machine.
    ceiling_ms: u64,
    /// The tree, kept alive for as long as the fixture is scanned. Never read:
    /// the roots are already named by the registry, and the value exists for
    /// its `Drop`.
    _directory: TempDir,
    /// The signatures that name the tree's roots.
    registry: SignatureRegistry,
}

impl Fixture {
    fn new(
        name: &'static str,
        ceiling_ms: u64,
        directory: TempDir,
        registry: SignatureRegistry,
    ) -> Self {
        Self {
            name,
            ceiling_ms,
            _directory: directory,
            registry,
        }
    }

    /// Scans this fixture with the real engine, the fixture's own registry, and
    /// no cancellation.
    fn scan(&self) -> ScanResult {
        self.scan_with(&NeverCancelled, |_| {})
    }

    /// [`Self::scan`] with the caller's cancellation contract and event sink.
    fn scan_with(
        &self,
        cancellation: &dyn CancellationProbe,
        on_event: impl FnMut(ScanEvent),
    ) -> ScanResult {
        ScanEngine::scan(
            &self.registry,
            &LifecycleProviderRegistry::new(Vec::new()),
            &OwnerProviderRegistry::new(Vec::new()),
            Some(&CATEGORIES),
            &[],
            false,
            &environment(),
            cancellation,
            on_event,
        )
    }
}

/// One fixture's row: the measurement for a single run.
///
/// Printed as JSON so a maintainer can read a baseline out of a CI log, and
/// compared field by field against the committed baseline except for the two
/// facts that are measurements of the machine rather than of the tree
/// (`duration_ms`, `peak_outstanding_directory_tasks`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct MetricsRow {
    fixture: String,
    duration_ms: u64,
    visited_entries: u64,
    directories_read: u64,
    peak_outstanding_directory_tasks: u64,
    candidate_count: u64,
    skipped_entries: u64,
    logical_bytes: u64,
    total_bytes: u64,
    cancelled: bool,
    /// Approximate resident-set growth across the scan, in KiB.
    ///
    /// Reported, never asserted against a baseline: the allocator decides when
    /// pages return to the operating system, so two identical runs disagree.
    /// It is here because "memory retained per scan" is one of the signals the
    /// contract names, and an approximate number that is printed honestly beats
    /// a precise one that is invented.
    rss_growth_kib: i64,
}

/// One fixture's committed facts. The byte fact is the logical population:
/// on-disk bytes are the platform's answer — a filesystem rounds them its own
/// way, and Windows reports what a file actually occupies — so they are bounded
/// per fixture rather than pinned in a file two platforms share.
///
/// Fields are declared in alphabetical order so the file reads the same way it
/// is written and a hand edit lands where the generator would put it.
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
struct Baseline {
    schema: u32,
    generator: String,
    note: String,
    bounds: BaselineBounds,
    fixtures: BTreeMap<String, BaselineFixture>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
struct BaselineBounds {
    max_concurrent_directory_reads: usize,
    max_depth: usize,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
struct BaselineFixture {
    candidate_count: u64,
    directories_read: u64,
    /// A bound, not a measurement: durations are not committed, so a slower
    /// machine cannot turn a green suite red by being slower.
    duration_ms_ceiling: u64,
    skipped_entries: u64,
    logical_bytes: u64,
    visited_entries: u64,
}

/// Every fixture the committed baseline describes.
fn committed_fixtures() -> Vec<Fixture> {
    vec![
        wide_fixture(),
        deep_fixture(),
        mixed_size_fixture(),
        mixed_age_fixture(),
        overlapping_roots_fixture(),
    ]
}

/// The number of retained candidates in a result, summed over its categories.
/// The logical population of the scanned items: a property of the tree, not of
/// the filesystem that holds it.
fn logical_bytes(result: &ScanResult) -> u64 {
    result
        .categories
        .iter()
        .map(|category| {
            category
                .items
                .iter()
                .map(|item| item.size.logical)
                .sum::<u64>()
        })
        .sum()
}

fn candidate_count(result: &ScanResult) -> u64 {
    result
        .categories
        .iter()
        .map(|category| category.items.len() as u64)
        .sum()
}

fn metrics_row(fixture: &str, result: &ScanResult, rss_growth_kib: i64) -> MetricsRow {
    MetricsRow {
        fixture: fixture.to_string(),
        duration_ms: result.metrics.duration_ms,
        visited_entries: result.metrics.visited_entries,
        directories_read: result.metrics.directories_read,
        peak_outstanding_directory_tasks: result.metrics.peak_outstanding_directory_tasks,
        candidate_count: candidate_count(result),
        skipped_entries: result.skipped_entry_count,
        logical_bytes: logical_bytes(result),
        total_bytes: result.total_bytes,
        cancelled: result.cancelled,
        rss_growth_kib,
    }
}

/// This process's resident set, in KiB, when the platform reports one.
///
/// A machine that cannot answer returns `None`, and the row states no memory
/// rather than a synthesized number.
fn resident_kib() -> Option<i64> {
    let mut system = sysinfo::System::new();
    system.refresh_processes(
        sysinfo::ProcessesToUpdate::Some(&[sysinfo::Pid::from_u32(std::process::id())]),
        true,
    );
    system
        .process(sysinfo::Pid::from_u32(std::process::id()))
        // `sysinfo` reports bytes; the row states KiB so it reads at the same
        // scale as the fixtures it describes.
        .map(|process| (process.memory() / 1024) as i64)
}

fn baseline_of(rows: &[(Fixture, ScanResult)]) -> Baseline {
    let limits = ScanLimits::default();
    let mut fixtures = BTreeMap::new();
    for (fixture, result) in rows {
        fixtures.insert(
            fixture.name.to_string(),
            BaselineFixture {
                candidate_count: candidate_count(result),
                directories_read: result.metrics.directories_read,
                duration_ms_ceiling: fixture.ceiling_ms,
                skipped_entries: result.skipped_entry_count,
                logical_bytes: logical_bytes(result),
                visited_entries: result.metrics.visited_entries,
            },
        );
    }
    Baseline {
        schema: BASELINE_SCHEMA,
        generator: "src-tauri/tests/scan_benchmark.rs".to_string(),
        note: "Deterministic scan facts per fixture. Regenerate with \
               `cargo test -p zenith-desktop --test scan_benchmark -- --ignored export_scan_baseline`. \
               Timing measurements are deliberately absent; `duration_ms_ceiling` is a bound."
            .to_string(),
        bounds: BaselineBounds {
            max_concurrent_directory_reads: limits.max_concurrent_directory_reads,
            max_depth: limits.max_depth,
        },
        fixtures,
    }
}

fn load_baseline() -> Baseline {
    let text = std::fs::read_to_string(BASELINE_PATH).unwrap_or_else(|error| {
        panic!("{BASELINE_PATH} must be committed (regenerate it with the ignored `export_scan_baseline` test): {error}")
    });
    serde_json::from_str(&text)
        .unwrap_or_else(|error| panic!("{BASELINE_PATH} must parse as the baseline: {error}"))
}

/// Writes `bytes` of deterministic content, creating the parent directories.
fn write_file(path: &Path, bytes: usize) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("fixture parent directory");
    }
    std::fs::write(path, vec![0xA5u8; bytes]).expect("fixture file");
}

/// Backdates an entry so the age policy sees it as inactive. The shape mirrors
/// the walker's `age_entry`: a POSIX directory can only be opened read-only,
/// while Windows needs `FILE_WRITE_ATTRIBUTES` plus `FILE_FLAG_BACKUP_SEMANTICS`
/// to open a directory at all.
fn age_entry(path: &Path, days: u64) {
    let when = SystemTime::now()
        .checked_sub(std::time::Duration::from_secs(days * 86_400))
        .expect("the fixture clock has a past");
    #[cfg(unix)]
    let entry = std::fs::File::open(path).expect("open fixture entry");
    #[cfg(windows)]
    let entry = {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_WRITE_ATTRIBUTES: u32 = 0x0000_0100;
        const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
        std::fs::OpenOptions::new()
            .access_mode(FILE_WRITE_ATTRIBUTES)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(path)
            .expect("open fixture entry")
    };
    #[cfg(not(any(unix, windows)))]
    let entry = std::fs::OpenOptions::new()
        .write(true)
        .open(path)
        .expect("open fixture entry");
    entry.set_modified(when).expect("backdate fixture entry");
}

fn signature(
    id: &str,
    name: &str,
    category: Category,
    paths: &[PathBuf],
    min_age_days: Option<u32>,
    unit: Option<CleanupUnitKind>,
    strategy: CleanStrategy,
) -> Signature {
    Signature {
        id: id.to_string(),
        name: name.to_string(),
        category,
        family: Default::default(),
        risk: RiskTier::Safe,
        strategy,
        paths: paths
            .iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect(),
        exclusions: Vec::new(),
        description: "benchmark fixture signature".into(),
        min_age_days,
        include_prefixes: vec![],
        exclude_prefixes: vec![],
        intensive_only: false,
        platforms: vec![],
        discovery: Default::default(),
        unit,
        owner: String::new(),
        priority: 0,
        fail_if_running: Vec::new(),
        provider: String::new(),
        provider_id: None,
        artifact_kind: Default::default(),
        consequence: String::new(),
    }
}

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

/// Wide: one root with 200 directories, three small files each.
///
/// The breadth fixture: it states that the walk reads every directory exactly
/// once (`directories_read == 1 root + 200 children`), skips nothing, and
/// accounts for the whole tree. Three files per directory (half a block, a
/// block, two blocks) make `total_bytes` an exact multiple of the 4096-byte
/// allocation unit every supported filesystem uses.
fn wide_fixture() -> Fixture {
    let directory = tempfile::tempdir().expect("fixture directory");
    let root = directory.path().join("wide");
    for index in 0..WIDE_DIRECTORIES {
        let child = root.join(format!("d{index:03}"));
        write_file(&child.join("a.bin"), 512);
        write_file(&child.join("b.bin"), 4_096);
        write_file(&child.join("c.bin"), 8_192);
    }
    let mut registry = SignatureRegistry::new();
    registry.register(signature(
        "benchmark.wide",
        "Wide cache",
        Category::Developer,
        &[root],
        None,
        None,
        CleanStrategy::DeleteContents,
    ));
    Fixture::new("wide", 2_500, directory, registry)
}

/// Deep: a chain 40 levels below the root, past the 32-level depth limit.
///
/// The depth fixture: the limit must be *reported* as the reason the
/// measurement is incomplete, not silently applied. A 4096-byte file sits at
/// the root and an 8192-byte file sits at the bottom of the chain, so the
/// assertion is that the file beyond the limit is not counted: the limits are
/// the tree's shape, and the totals are what the walk actually saw.
fn deep_fixture() -> Fixture {
    let directory = tempfile::tempdir().expect("fixture directory");
    let root = directory.path().join("deep");
    write_file(&root.join("root-file.bin"), 4_096);
    let mut level = root.clone();
    for index in 0..40 {
        level = level.join(format!("d{index:02}"));
    }
    write_file(&level.join("bottom.bin"), 8_192);
    let mut registry = SignatureRegistry::new();
    registry.register(signature(
        "benchmark.deep",
        "Deep cache",
        Category::System,
        &[root],
        None,
        None,
        CleanStrategy::DeleteContents,
    ));
    Fixture::new("deep", 1_250, directory, registry)
}

/// Mixed small/large: files spanning 3 bytes to 2 MiB, with one nested
/// directory so the walk still has a directory to descend into.
///
/// The size fixture: the measurement carries both populations — the logical
/// lengths the files state and the allocated blocks the filesystem charges —
/// and the assertion derives the allocated total from the logical one rather
/// than from a literal, so the two cannot drift apart unnoticed.
fn mixed_size_fixture() -> Fixture {
    let directory = tempfile::tempdir().expect("fixture directory");
    let root = directory.path().join("mixed-size");
    write_file(&root.join("tiny.bin"), 3);
    write_file(&root.join("small.bin"), 4_096);
    write_file(&root.join("odd.bin"), 5_000);
    write_file(&root.join("big.bin"), 1_048_576);
    write_file(&root.join("huge.bin"), 2_097_152);
    write_file(&root.join("nested/inner.bin"), 9_000);
    let mut registry = SignatureRegistry::new();
    registry.register(signature(
        "benchmark.mixed-size",
        "Mixed cache",
        Category::Developer,
        &[root],
        None,
        None,
        CleanStrategy::DeleteContents,
    ));
    Fixture::new("mixed_size", 1_250, directory, registry)
}

/// Mixed age: a stale subtree and a fresh one under a namespace signature that
/// removes stale contents older than 30 days.
///
/// The age fixture: it asserts that the verdict is per-entry — the backdated
/// child reports its stale files and the freshly written one reports none — so
/// a recent write in one child cannot make a stale sibling invisible or the
/// other way round. It is also the row that proves the aged walk reports its
/// traversal: `measure_tree_stats` feeds the same counters the size walks do,
/// so this fixture's row is populated rather than left at zero.
fn mixed_age_fixture() -> Fixture {
    let directory = tempfile::tempdir().expect("fixture directory");
    let root = directory.path().join("mixed-age");
    let old = root.join("old");
    write_file(&old.join("stale-a.bin"), 4_096);
    write_file(&old.join("stale-b.bin"), 4_096);
    age_entry(&old.join("stale-a.bin"), 40);
    age_entry(&old.join("stale-b.bin"), 40);
    write_file(&root.join("recent/fresh-a.bin"), 4_096);
    write_file(&root.join("recent/fresh-b.bin"), 4_096);
    let mut registry = SignatureRegistry::new();
    registry.register(signature(
        "benchmark.mixed-age",
        "Aged namespace",
        Category::System,
        &[root],
        Some(30),
        Some(CleanupUnitKind::ChildNamespace),
        CleanStrategy::DeleteStaleContents,
    ));
    Fixture::new("mixed_age", 3_500, directory, registry)
}

/// Overlapping roots: two signatures where one root contains the other.
///
/// The fold fixture: the nested unit is not a second total. It asserts that the
/// scan retains one unit, names the folded rule as provenance, and reports the
/// containment it resolved — the accounting decision the benchmark must not
/// change silently.
fn overlapping_roots_fixture() -> Fixture {
    let directory = tempfile::tempdir().expect("fixture directory");
    let root = directory.path().join("overlap");
    write_file(&root.join("outer.bin"), 4_096);
    write_file(&root.join("inner/inner.bin"), 8_192);
    let mut registry = SignatureRegistry::new();
    registry.register(signature(
        "benchmark.overlap.broad",
        "Broad cache",
        Category::Developer,
        std::slice::from_ref(&root),
        None,
        None,
        CleanStrategy::DeleteContents,
    ));
    registry.register(signature(
        "benchmark.overlap.narrow",
        "Nested cache",
        Category::Developer,
        &[root.join("inner")],
        None,
        None,
        CleanStrategy::DeleteContents,
    ));
    Fixture::new("overlapping_roots", 1_250, directory, registry)
}

// ---------------------------------------------------------------------------
// Assertions per fixture
// ---------------------------------------------------------------------------

/// The shape each fixture states about itself, asserted against the result the
/// fixture's own scan produced.
fn assert_fixture_shape(fixture: &Fixture, result: &ScanResult) {
    match fixture.name {
        "wide" => {
            assert_eq!(
                result.metrics.directories_read,
                WIDE_DIRECTORIES as u64 + 1,
                "the walk reads the root and each of its 200 children exactly once"
            );
            assert_eq!(
                result.skipped_entry_count, 0,
                "a readable fixture skips nothing"
            );
            assert_eq!(
                candidate_count(result),
                1,
                "one root of one signature is one candidate"
            );
            assert_eq!(
                logical_bytes(result),
                (WIDE_DIRECTORIES as u64) * (512 + 4_096 + 8_192),
                "the logical population is the whole tree, exactly, on every filesystem"
            );
            assert!(
                result.total_bytes >= logical_bytes(result),
                "on-disk accounting never reports less than the logical population: {result:?}"
            );
        }
        "deep" => {
            let limit = ScanLimits::default().max_depth as u64;
            assert_eq!(
                result.metrics.directories_read,
                limit + 1,
                "the walk reads the root plus max_depth levels and refuses to descend further"
            );
            assert!(
                result
                    .incomplete_reasons
                    .iter()
                    .any(|reason| reason.contains("depth limit")),
                "the depth limit is reported as the reason the measurement is incomplete, \
                 not applied silently: {:?}",
                result.incomplete_reasons
            );
            assert_eq!(
                result.skipped_entry_count, 1,
                "exactly the first directory beyond the limit is refused"
            );
            assert_eq!(
                logical_bytes(result),
                4_096,
                "only the root file is counted: the 8192-byte file below the limit is not"
            );
            assert!(result.total_bytes >= logical_bytes(result));
        }
        "mixed_size" => {
            let logical = 3 + 4_096 + 5_000 + 1_048_576 + 2_097_152 + 9_000;
            // The logical population is a property of the fixture and is exact
            // everywhere. The allocated one is the platform's answer — APFS
            // rounds each file up to a block, and `GetCompressedFileSizeW`
            // reports what a file actually occupies, which for a small resident
            // file is its logical size — so it is bounded rather than pinned.
            assert_eq!(
                logical_bytes(result),
                logical,
                "the logical population is the fixture's own bytes"
            );
            assert!(
                result.total_bytes >= logical,
                "on-disk accounting never reports less than the logical population: {result:?}"
            );
            let item = result
                .categories
                .iter()
                .flat_map(|category| category.items.iter())
                .find(|item| item.signature_id == "benchmark.mixed-size")
                .expect("the mixed-size cache is retained");
            assert_eq!(
                item.size.logical, logical,
                "the logical population is preserved beside the allocated one"
            );
        }
        "mixed_age" => {
            let system = result
                .categories
                .iter()
                .find(|category| category.category == Category::System)
                .expect("the aged namespace is scanned in its own category");
            assert_eq!(
                system.items.len(),
                2,
                "both children are inventoried; the age policy decides eligibility, not visibility"
            );
            let item = |name: &str| {
                system
                    .items
                    .iter()
                    .find(|item| item.name == name)
                    .unwrap_or_else(|| panic!("the {name} child is enumerated"))
            };
            let stale = item("old")
                .stale
                .as_ref()
                .expect("a namespace signature reports its stale entries");
            assert_eq!(stale.stale_file_count, 2, "both backdated files are stale");
            // The stale amount is the platform's on-disk accounting, so it is
            // bounded by the logical size of the two files rather than pinned.
            assert!(
                stale.stale_bytes >= 8_192,
                "both backdated files are counted as stale: {stale:?}"
            );
            let fresh = item("recent")
                .stale
                .as_ref()
                .expect("a namespace signature reports its stale entries");
            assert_eq!(
                fresh.stale_file_count, 0,
                "a freshly written child has nothing stale to report"
            );
            assert!(
                fresh.nothing_is_stale(),
                "a freshly written child has nothing stale to remove"
            );
        }
        "overlapping_roots" => {
            assert_eq!(
                candidate_count(result),
                1,
                "a unit inside a broader one is that unit's bytes, not a second total"
            );
            assert_eq!(
                result.suppressed_overlap_count, 1,
                "the folded unit is reported rather than dropped silently"
            );
            let items: Vec<_> = result
                .categories
                .iter()
                .flat_map(|category| category.items.iter())
                .collect();
            assert_eq!(items[0].signature_id, "benchmark.overlap.broad");
            assert_eq!(
                items[0]
                    .overlaps
                    .iter()
                    .map(|overlap| overlap.signature_id.as_str())
                    .collect::<Vec<_>>(),
                vec!["benchmark.overlap.narrow"],
                "the rule that did not count the bytes is named on the unit that did"
            );
            assert_eq!(
                logical_bytes(result),
                12_288,
                "the nested file is counted once, through the broader unit"
            );
            assert!(result.total_bytes >= logical_bytes(result));
        }
        other => panic!("fixture {other} has no stated shape"),
    }
}

/// Asserts the two facts every fixture must satisfy whatever its shape.
fn assert_common_invariants(row: &MetricsRow, max_concurrent_directory_reads: usize) {
    assert!(
        row.peak_outstanding_directory_tasks <= max_concurrent_directory_reads as u64,
        "{}: the traversal's stated bound must hold as a measurement: peak {} > bound {}",
        row.fixture,
        row.peak_outstanding_directory_tasks,
        max_concurrent_directory_reads
    );
    assert!(
        !row.cancelled,
        "{}: an uncancelled fixture must not report a cancelled scan",
        row.fixture
    );
}

/// Prints the platform line a CI log reader needs to interpret the table.
fn print_environment_line() {
    println!(
        "scan_benchmark environment os={} arch={} zenith-desktop={}",
        std::env::consts::OS,
        std::env::consts::ARCH,
        env!("CARGO_PKG_VERSION")
    );
}

/// Prints one fixture's row as a single JSON object.
fn print_row(row: &MetricsRow) {
    println!(
        "scan_benchmark fixture {}",
        serde_json::to_string(row).expect("a metrics row serializes")
    );
}

/// Compares one measured row against its committed facts, naming the field that
/// moved.
fn assert_row_matches_baseline(baseline: &Baseline, row: &MetricsRow, fixture: &Fixture) {
    let expected = baseline
        .fixtures
        .get(&row.fixture)
        .unwrap_or_else(|| panic!("{} must be in the committed baseline", row.fixture));
    assert_eq!(
        row.visited_entries, expected.visited_entries,
        "{}: visited_entries moved",
        row.fixture
    );
    assert_eq!(
        row.directories_read, expected.directories_read,
        "{}: directories_read moved",
        row.fixture
    );
    assert_eq!(
        row.candidate_count, expected.candidate_count,
        "{}: candidate_count moved",
        row.fixture
    );
    assert_eq!(
        row.skipped_entries, expected.skipped_entries,
        "{}: skipped_entries moved",
        row.fixture
    );
    assert_eq!(
        row.logical_bytes, expected.logical_bytes,
        "{}: logical_bytes moved",
        row.fixture
    );
    assert_eq!(
        fixture.ceiling_ms, expected.duration_ms_ceiling,
        "{}: the stated ceiling and the committed one disagree",
        row.fixture
    );
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Regenerates the committed baseline. Ignored so the suite never writes to the
/// tree; CI runs it and diff-checks the file, exactly as it does the generated
/// TypeScript bindings.
#[test]
#[ignore = "baseline regeneration; CI regenerates and diff-checks the committed file"]
fn export_scan_baseline() {
    let mut rows = Vec::new();
    for fixture in committed_fixtures() {
        let result = fixture.scan();
        assert_fixture_shape(&fixture, &result);
        rows.push((fixture, result));
    }
    let baseline = baseline_of(&rows);
    let mut text = serde_json::to_string_pretty(&baseline).expect("the baseline serializes");
    text.push('\n');
    std::fs::write(BASELINE_PATH, text).expect("the baseline file is writable");
}

/// The benchmark proper: every deterministic field equals the committed
/// baseline, the traversal honoured its stated concurrency bound, and each
/// fixture finished well inside its ceiling.
#[test]
fn scan_metrics_match_the_committed_baseline() {
    let baseline = load_baseline();
    assert_eq!(
        baseline.schema, BASELINE_SCHEMA,
        "the committed baseline and this harness must agree on the format"
    );
    let limits = ScanLimits::default();
    assert_eq!(
        baseline.bounds.max_depth, limits.max_depth,
        "the committed bounds must be the bounds a scan runs under"
    );
    assert_eq!(
        baseline.bounds.max_concurrent_directory_reads, limits.max_concurrent_directory_reads,
        "the committed bounds must be the bounds a scan runs under"
    );

    print_environment_line();
    let fixtures = committed_fixtures();
    let mut measured = Vec::new();
    for fixture in fixtures {
        let before = resident_kib();
        let result = fixture.scan();
        let rss_growth_kib = match (before, resident_kib()) {
            (Some(before), Some(after)) => after - before,
            _ => 0,
        };
        assert_fixture_shape(&fixture, &result);
        let row = metrics_row(fixture.name, &result, rss_growth_kib);
        print_row(&row);
        assert_common_invariants(&row, baseline.bounds.max_concurrent_directory_reads);
        assert_row_matches_baseline(&baseline, &row, &fixture);
        assert!(
            row.duration_ms <= fixture.ceiling_ms,
            "{}: {} ms is over the {} ms ceiling (a shared CI runner is slower than a \
             developer machine, which is why the ceiling is generous — this is an \
             order-of-magnitude regression, not a scheduling hiccup)",
            row.fixture,
            row.duration_ms,
            fixture.ceiling_ms
        );
        measured.push(row);
    }

    let expected: Vec<String> = baseline.fixtures.keys().cloned().collect();
    let actual: Vec<String> = measured
        .iter()
        .map(|row| row.fixture.clone())
        .collect::<Vec<_>>();
    let mut sorted = actual.clone();
    sorted.sort();
    assert_eq!(
        sorted, expected,
        "the fixture set must match the committed baseline exactly; regenerate it with the \
         ignored `export_scan_baseline` test when a fixture is added"
    );
}

/// Cancellation latency: how long a running scan takes to stop once the caller
/// says stop.
///
/// The scan holds two signatures in one category: a one-file anchor root that
/// answers first, and the wide fixture's shape behind it. The probe trips when
/// the anchor's candidate is streamed — the engine emits a signature's items
/// after that signature's walk, and checks the probe before the next signature
/// — so the wide tree is still unwalked when cancellation is requested. The
/// assertions are that the scan returns cancelled, that the wide tree was never
/// walked, and that it did so well inside a bound stated for a shared CI
/// runner.
#[test]
fn cancellation_latency_is_measured_from_the_first_candidate() {
    let directory = tempfile::tempdir().expect("fixture directory");
    let anchor = directory.path().join("cancel-anchor");
    write_file(&anchor.join("anchor.bin"), 4_096);
    let wide = directory.path().join("cancel-wide");
    for index in 0..WIDE_DIRECTORIES {
        let child = wide.join(format!("d{index:03}"));
        write_file(&child.join("a.bin"), 512);
        write_file(&child.join("b.bin"), 4_096);
        write_file(&child.join("c.bin"), 8_192);
    }
    let mut registry = SignatureRegistry::new();
    registry.register(signature(
        "benchmark.cancel.anchor",
        "Anchor cache",
        Category::Developer,
        &[anchor],
        None,
        None,
        CleanStrategy::DeleteContents,
    ));
    registry.register(signature(
        "benchmark.cancel.wide",
        "Wide cache behind the anchor",
        Category::Developer,
        &[wide],
        None,
        None,
        CleanStrategy::DeleteContents,
    ));
    let fixture = Fixture::new("cancellation", 0, directory, registry);

    let flag = Arc::new(AtomicBool::new(false));
    let flip_at = Arc::new(OnceLock::new());
    let scanned = {
        let cancellation_flag = Arc::clone(&flag);
        let flag = Arc::clone(&flag);
        let flip_at = Arc::clone(&flip_at);
        std::thread::spawn(move || {
            let probe = Probe {
                flag: cancellation_flag,
            };
            let result = fixture.scan_with(&probe, move |event| {
                if let ScanEvent::ItemFound { .. } = event {
                    flag.store(true, Ordering::SeqCst);
                    let _ = flip_at.set(Instant::now());
                }
            });
            (result, Instant::now())
        })
        .join()
        .expect("the scanning thread does not panic")
    };
    let (result, returned_at) = scanned;
    let flipped_at = flip_at
        .get()
        .copied()
        .expect("the first candidate must reach the event sink before the scan ends");
    let latency_ms = returned_at
        .duration_since(flipped_at)
        .as_millis()
        .min(u128::from(u64::MAX)) as u64;

    let row = metrics_row("cancellation", &result, 0);
    print_environment_line();
    print_row(&row);
    println!(
        "scan_benchmark cancellation_latency_ms {latency_ms} ceiling_ms {CANCELLATION_LATENCY_CEILING_MS}"
    );

    assert!(
        result.cancelled,
        "a probe that tripped must cancel the scan"
    );
    assert!(
        result
            .incomplete_reasons
            .iter()
            .any(|reason| reason.contains("cancelled")),
        "a cancelled scan states why it is incomplete: {:?}",
        result.incomplete_reasons
    );
    assert!(
        result.metrics.directories_read < WIDE_DIRECTORIES as u64,
        "the cancellation must stop work rather than arrive after it: {} directories were read \
         out of the wide tree's {WIDE_DIRECTORIES}",
        result.metrics.directories_read
    );
    assert_eq!(
        candidate_count(&result),
        1,
        "the signature behind the anchor was never scanned, so only the anchor reported a unit"
    );
    assert_eq!(
        logical_bytes(&result),
        ANCHOR_BYTES,
        "only the anchor's bytes were observed; the wide tree contributed nothing"
    );
    assert!(result.total_bytes >= ANCHOR_BYTES);
    assert!(
        latency_ms <= CANCELLATION_LATENCY_CEILING_MS,
        "{latency_ms} ms from the probe tripping to the scan returning is over the \
         {CANCELLATION_LATENCY_CEILING_MS} ms bound"
    );
}

/// A probe whose verdict the event sink decides.
struct Probe {
    flag: Arc<AtomicBool>,
}

impl CancellationProbe for Probe {
    fn is_cancelled(&self) -> bool {
        self.flag.load(Ordering::SeqCst)
    }
}

/// Inaccessible (unix): a directory the walk cannot read.
///
/// `chmod 000` is the fixture, so this cannot exist on a non-unix host. The
/// assertion is that the refusal is *attempted and counted* — the read is
/// attempted, the failure is reported, and the bytes behind the locked
/// directory are not claimed.
#[cfg(unix)]
#[test]
fn inaccessible_directory_is_attempted_counted_and_reported() {
    use std::os::unix::fs::PermissionsExt;

    let directory = tempfile::tempdir().expect("fixture directory");
    let root = directory.path().join("inaccessible");
    write_file(&root.join("readable.bin"), 4_096);
    let locked = root.join("locked");
    write_file(&locked.join("secret.bin"), 8_192);
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000))
        .expect("the fixture can lock its own directory");

    let mut registry = SignatureRegistry::new();
    registry.register(signature(
        "benchmark.inaccessible",
        "Locked cache",
        Category::System,
        &[root],
        None,
        None,
        CleanStrategy::DeleteContents,
    ));
    let fixture = Fixture::new("inaccessible", 1_250, directory, registry);
    let result = fixture.scan();

    // Restored before any assertion can panic, so a failure still leaves a
    // temporary tree the test runner can remove.
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755))
        .expect("the fixture can unlock its own directory");

    let row = metrics_row(fixture.name, &result, 0);
    print_environment_line();
    print_row(&row);
    assert_fixture_shape_inaccessible(&result);
    assert_common_invariants(&row, ScanLimits::default().max_concurrent_directory_reads);
    assert!(
        row.duration_ms <= fixture.ceiling_ms,
        "{}: {} ms is over the {} ms ceiling",
        row.fixture,
        row.duration_ms,
        fixture.ceiling_ms
    );
}

#[cfg(unix)]
fn assert_fixture_shape_inaccessible(result: &ScanResult) {
    assert_eq!(
        result.metrics.directories_read, 2,
        "the walk attempts both directories; a refused read is work that was attempted"
    );
    assert_eq!(
        result.skipped_entry_count, 1,
        "the locked directory is accounted for as one skipped entry"
    );
    assert!(
        result
            .incomplete_reasons
            .iter()
            .any(|reason| reason.contains("locked")),
        "the refusal names the directory it could not read: {:?}",
        result.incomplete_reasons
    );
    assert_eq!(
        logical_bytes(result),
        4_096,
        "the bytes behind the locked directory are not claimed"
    );
}

/// Symlink boundary (unix): a link to a directory outside the root.
///
/// The link's target is a real tree with a distinctive size, so the assertion
/// has two halves: the walk accounts for the link itself (it is visited) and it
/// never descends into it (the outside directory is not read and its bytes are
/// not claimed).
#[cfg(unix)]
#[test]
fn a_symlinked_directory_is_accounted_for_and_never_traversed() {
    let outside = tempfile::tempdir().expect("outside directory");
    write_file(&outside.path().join("outside.bin"), 16_384);
    let directory = tempfile::tempdir().expect("fixture directory");
    let root = directory.path().join("symlink");
    write_file(&root.join("real/payload.bin"), 4_096);
    std::os::unix::fs::symlink(outside.path(), root.join("escape"))
        .expect("the fixture can create a directory symlink");

    let mut registry = SignatureRegistry::new();
    registry.register(signature(
        "benchmark.symlink",
        "Linked cache",
        Category::Developer,
        &[root],
        None,
        None,
        CleanStrategy::DeleteContents,
    ));
    let fixture = Fixture::new("symlink", 1_250, directory, registry);
    let result = fixture.scan();

    let row = metrics_row(fixture.name, &result, 0);
    print_environment_line();
    print_row(&row);
    assert_fixture_shape_symlink(&result);
    assert_common_invariants(&row, ScanLimits::default().max_concurrent_directory_reads);
    assert!(
        row.duration_ms <= fixture.ceiling_ms,
        "{}: {} ms is over the {} ms ceiling",
        row.fixture,
        row.duration_ms,
        fixture.ceiling_ms
    );
}

#[cfg(unix)]
fn assert_fixture_shape_symlink(result: &ScanResult) {
    assert_eq!(
        result.metrics.directories_read, 2,
        "only the root and its real child are read; the link's target is not"
    );
    assert_eq!(
        result.metrics.visited_entries, 4,
        "the walk visits exactly root, real, its payload, and the link itself"
    );
    // The link contributes its own entry and no allocated bytes: APFS inlines a
    // link's target path and a Windows reparse point occupies nothing, so the
    // on-disk population is exactly the real file's, while the bytes the link
    // points at are never claimed.
    assert_eq!(
        result.total_bytes, 4_096,
        "the 16384 bytes behind the link are not claimed"
    );
    assert!(
        logical_bytes(result) >= 4_096,
        "the payload is counted beside whatever the link itself reports"
    );
}
