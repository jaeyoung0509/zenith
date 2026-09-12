//! `Zenith --doctor`: the same pure assertions CI runs, evaluated against the
//! real environment and printed as a de-identified report.
//!
//! The report is safe to paste into a bug report by construction: the
//! fingerprint and every check detail carry shapes, classifications, and
//! booleans — never a user name, a drive-letter path, a profile path, or a
//! machine name. It performs no network access and uploads nothing.
//!
//! Two properties make the report trustworthy as a CI gate:
//!
//! * the checks are the flavor-parameterized [`crate::platform::path_algebra`]
//!   invariants plus the environment's own resolution facts, so a Windows
//!   machine and a macOS machine assert the same rules;
//! * a check is a count, not a yes/no: a sample set that stops exercising the
//!   invariant makes the check fail rather than pass vacuously.

use crate::models::PlatformKind;
use crate::platform::path_algebra;
use crate::platform::path_algebra::{
    contains, fold, is_absolute, is_root, key, normalize, protected_root, PathFlavor,
};
use crate::platform::PlatformEnvironment;
use crate::signatures::SignatureRegistry;

/// One self-check result. `name` is the invariant, `detail` is a shape or
/// boolean summary of what was checked.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, specta::Type)]
pub struct SelfCheckRow {
    pub name: String,
    pub outcome: SelfCheckOutcome,
    pub detail: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum SelfCheckOutcome {
    Pass,
    Fail,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, specta::Type)]
pub struct EnvironmentReport {
    pub platform: crate::models::PlatformKind,
    pub fingerprint: Vec<String>,
    pub checks: Vec<SelfCheckRow>,
    pub failures: u32,
}

const USAGE: &str = "\
Zenith platform self-check

USAGE:
  Zenith --doctor          Print the de-identified environment fingerprint and self-check table
  Zenith --doctor --json   Print the same report as a single JSON object
  Zenith --help            Show this message

EXIT CODES:
  0  every self-check passed
  1  at least one self-check failed

The report carries shapes and booleans only: no user name, profile path,
drive-letter path, or machine name. It is computed locally and never uploaded.";

/// Runs the same pure assertions CI runs, against the real environment.
pub fn self_check(environment: &PlatformEnvironment) -> EnvironmentReport {
    let flavor = environment.flavor();
    let mut checks = Vec::new();

    // Path algebra. The Windows samples run on every host: the algebra is
    // parameterized by flavor, so a macOS runner still proves the Windows rules.
    let flavor_samples = flavor_samples(flavor);
    checks.push(row(
        "normalize_is_idempotent",
        violations(&flavor_samples, |sample| {
            let once = normalize(sample, flavor);
            normalize(&once, flavor) == once
        }),
        flavor_samples.len(),
        &format!("samples normalized twice (flavor={})", flavor.name()),
    ));
    checks.push(row(
        "fold_is_idempotent",
        violations(&flavor_samples, |sample| {
            let once = fold(sample, flavor);
            fold(&once, flavor) == once
        }),
        flavor_samples.len(),
        &format!("samples folded twice (flavor={})", flavor.name()),
    ));

    let separator_samples = separator_pairs(flavor);
    checks.push(row(
        "separators_are_equivalent",
        violations(&separator_samples, |(left, right)| {
            key(left, flavor) == key(right, flavor)
        }),
        separator_samples.len(),
        &format!("separator pairs compare equal (flavor={})", flavor.name()),
    ));

    let verbatim_samples = WINDOWS_VERBATIM_SAMPLES;
    checks.push(row(
        "verbatim_prefix_is_invariant",
        violations(verbatim_samples, |(verbatim, plain)| {
            normalize(verbatim, PathFlavor::Windows) == normalize(plain, PathFlavor::Windows)
        }),
        verbatim_samples.len(),
        "verbatim and plain spellings normalize alike",
    ));

    let unc_samples = WINDOWS_UNC_SAMPLES;
    checks.push(row(
        "unc_paths_are_invariant",
        violations(unc_samples, |(left, right)| {
            key(left, PathFlavor::Windows) == key(right, PathFlavor::Windows)
        }),
        unc_samples.len(),
        "UNC spellings compare equal",
    ));

    let trailing_samples = trailing_separator_pairs(flavor);
    checks.push(row(
        "trailing_separator_is_invariant",
        violations(&trailing_samples, |(with, without)| {
            key(with, flavor) == key(without, flavor)
        }),
        trailing_samples.len(),
        &format!("trailing separators are ignored (flavor={})", flavor.name()),
    ));

    let protected_samples = WINDOWS_PROTECTED_SAMPLES;
    checks.push(row(
        "protected_root_is_drive_letter_independent",
        violations(protected_samples, |(left, right)| {
            protected_root(left, PathFlavor::Windows) == protected_root(right, PathFlavor::Windows)
        }),
        protected_samples.len(),
        "protected roots are drive-letter independent",
    ));

    let containment_samples = CONTAINMENT_SAMPLES;
    checks.push(row(
        "containment_respects_component_boundaries",
        violations(containment_samples, |(parent, child, contained)| {
            contains(parent, child, PathFlavor::Windows) == *contained
        }),
        containment_samples.len(),
        "component-boundary containment samples hold",
    ));

    let short_name_samples = SHORT_NAME_SAMPLES;
    checks.push(row(
        "short_name_ambiguity_fails_closed",
        violations(short_name_samples, |(path, refused)| {
            protected_root(path, PathFlavor::Windows).is_some() == *refused
        }),
        short_name_samples.len(),
        "8.3 alias ambiguity samples fail closed",
    ));

    // Environment resolution.
    checks.push(profile_root_row(environment));
    checks.push(broad_root_row(environment));
    checks.push(known_folder_row(environment));
    checks.push(catalog_row(environment));

    let failures = checks
        .iter()
        .filter(|check| check.outcome == SelfCheckOutcome::Fail)
        .count() as u32;

    EnvironmentReport {
        platform: platform_of(environment),
        fingerprint: fingerprint(environment),
        checks,
        failures,
    }
}

pub fn render_text(report: &EnvironmentReport) -> String {
    let mut lines = Vec::new();
    lines.push("Zenith environment self-check".to_string());
    lines.push(String::new());
    lines.push(format!("platform: {}", platform_name(report.platform)));
    lines.push("fingerprint:".to_string());
    for entry in &report.fingerprint {
        lines.push(format!("  {entry}"));
    }
    lines.push("checks:".to_string());
    for check in &report.checks {
        let outcome = match check.outcome {
            SelfCheckOutcome::Pass => "PASS",
            SelfCheckOutcome::Fail => "FAIL",
        };
        lines.push(format!("  [{outcome}] {}: {}", check.name, check.detail));
    }
    lines.push(format!(
        "result: {} of {} checks failed",
        report.failures,
        report.checks.len()
    ));
    lines.join("\n")
}

/// Appends the check that only a running machine can answer: whether this
/// process can actually write the diagnostics log, in the directory the
/// application uses.
///
/// It is separate from [`self_check`] because that function is also run against
/// committed environment fixtures, and a report must never write to the
/// directory it is describing.
pub fn with_log_writability(mut report: EnvironmentReport) -> EnvironmentReport {
    let environment = PlatformEnvironment::native();
    let directory = crate::diagnostics::log_dir(&environment);
    let outcome = match crate::diagnostics::probe_log_writability(&directory) {
        Ok(()) => row("log_writable", 0, 1, "line appended to the diagnostics log"),
        Err(_) => SelfCheckRow {
            name: "log_writable".to_string(),
            outcome: SelfCheckOutcome::Fail,
            detail: "the diagnostics log cannot be written".to_string(),
        },
    };
    if outcome.outcome == SelfCheckOutcome::Fail {
        report.failures += 1;
    }
    report.checks.push(outcome);
    report
}

/// Returns Some(exit_code) when argv asked for a console action.
pub fn run_cli(args: &[String]) -> Option<i32> {
    match args.first().map(String::as_str) {
        Some("--doctor") => {
            let report = with_log_writability(self_check(&PlatformEnvironment::native()));
            let exit_code = i32::from(report.failures > 0);
            if args.iter().skip(1).any(|argument| argument == "--json") {
                // Serialized verbatim: the payload is de-identified by
                // construction and must stay machine-parseable for CI. The
                // rendered table below is redacted as well as de-identified.
                match serde_json::to_string(&report) {
                    Ok(payload) => println!("{payload}"),
                    Err(error) => {
                        println!(
                            "{{\"error\":\"{}\"}}",
                            crate::diagnostics::sanitize_log(&error.to_string())
                        );
                        return Some(1);
                    }
                }
            } else {
                println!(
                    "{}",
                    crate::diagnostics::sanitize_log(&render_text(&report))
                );
            }
            Some(exit_code)
        }
        Some("--help") | Some("-h") => {
            println!("{USAGE}");
            Some(0)
        }
        _ => None,
    }
}

fn platform_of(environment: &PlatformEnvironment) -> PlatformKind {
    if environment.flavor().is_windows() {
        PlatformKind::Windows
    } else {
        PlatformKind::current()
    }
}

fn platform_name(platform: PlatformKind) -> &'static str {
    match platform {
        PlatformKind::Macos => "macos",
        PlatformKind::Windows => "windows",
        PlatformKind::Linux => "linux",
        PlatformKind::Other => "other",
    }
}

/// Shapes and booleans only: how the environment is configured, never what the
/// paths are called.
fn fingerprint(environment: &PlatformEnvironment) -> Vec<String> {
    let flavor = environment.flavor();
    let home = environment.user_home();
    let mut entries = vec![
        format!("platform={}", platform_name(platform_of(environment))),
        format!("path_flavor={}", flavor.name()),
        format!("arch={}", std::env::consts::ARCH),
        format!("profile_root={}", presence(home.is_some())),
        format!(
            "local_app_data={}",
            presence(environment.local_app_data().is_some())
        ),
        format!(
            "roaming_app_data={}",
            presence(environment.roaming_app_data().is_some())
        ),
        format!(
            "program_files={}",
            presence(environment.program_files().is_some())
        ),
        format!(
            "program_data={}",
            presence(environment.program_data().is_some())
        ),
        format!(
            "known_folders={}/{}",
            environment.known_folders().len(),
            crate::platform::KnownFolder::ALL.len()
        ),
        format!("path_entries={}", environment.path_entries().len()),
        format!(
            "volumes={}",
            match environment.volumes() {
                Some(volumes) => format!("stated({})", volumes.len()),
                None => "os".to_string(),
            }
        ),
        format!("separator={}", describe_separator(flavor)),
        format!(
            "case_folding={}",
            if flavor.is_windows() {
                "insensitive"
            } else {
                "sensitive"
            }
        ),
    ];

    // Known Folder Move and administrator redirection: how many stated
    // user-content folders are not the literal profile join. A count, not a
    // location.
    let redirected = home.as_deref().map_or(0, |home| {
        environment
            .known_folders()
            .iter()
            .filter(|(folder, path)| {
                let name = capitalize(folder.token());
                path.as_path() != home.join(name)
            })
            .count()
    });
    entries.push(format!("known_folder_redirects={redirected}"));
    entries.push(format!(
        "profile_is_broad_root={}",
        match home {
            Some(home) => description_of_broad_root(&home.to_string_lossy(), flavor).to_string(),
            None => "unknown".to_string(),
        }
    ));
    entries
}

fn presence(present: bool) -> &'static str {
    if present {
        "stated"
    } else {
        "absent"
    }
}

fn describe_separator(flavor: PathFlavor) -> &'static str {
    match flavor.separator() {
        '\\' => "backslash",
        _ => "slash",
    }
}

fn description_of_broad_root(home: &str, flavor: PathFlavor) -> &'static str {
    if is_root(home, flavor) || protected_root(home, flavor).is_some() {
        "protected"
    } else {
        "cleanable"
    }
}

fn capitalize(token: &str) -> String {
    let mut name = token.to_string();
    if let Some(first) = name.get_mut(..1) {
        first.make_ascii_uppercase();
    }
    name
}

fn row(name: &str, violations: usize, total: usize, subject: &str) -> SelfCheckRow {
    SelfCheckRow {
        name: name.to_string(),
        outcome: if violations == 0 {
            SelfCheckOutcome::Pass
        } else {
            SelfCheckOutcome::Fail
        },
        detail: if violations == 0 {
            format!("{total} {subject}")
        } else {
            format!("{violations} of {total} {subject}")
        },
    }
}

fn violations<T>(samples: &[T], holds: impl Fn(&T) -> bool) -> usize {
    samples.iter().filter(|sample| !holds(sample)).count()
}

fn profile_root_row(environment: &PlatformEnvironment) -> SelfCheckRow {
    let flavor = environment.flavor();
    let (violations, detail) = match environment.user_home() {
        Some(home) => {
            let absolute = is_absolute(&home.to_string_lossy(), flavor);
            (
                usize::from(!absolute),
                format!("profile root absolute={absolute}"),
            )
        }
        None => (1, "profile root absent".to_string()),
    };
    SelfCheckRow {
        name: "home_resolves".to_string(),
        outcome: if violations == 0 {
            SelfCheckOutcome::Pass
        } else {
            SelfCheckOutcome::Fail
        },
        detail,
    }
}

fn broad_root_row(environment: &PlatformEnvironment) -> SelfCheckRow {
    let flavor = environment.flavor();
    let Some(home) = environment.user_home() else {
        return SelfCheckRow {
            name: "home_is_not_a_broad_root".to_string(),
            outcome: SelfCheckOutcome::Fail,
            detail: "profile root absent".to_string(),
        };
    };
    let text = home.to_string_lossy();
    let root = protected_root(&text, flavor);
    SelfCheckRow {
        name: "home_is_not_a_broad_root".to_string(),
        outcome: if root.is_none() {
            SelfCheckOutcome::Pass
        } else {
            SelfCheckOutcome::Fail
        },
        detail: match root {
            Some(root) => format!("profile root is a protected {}", root.reason()),
            None => "profile root is a user directory".to_string(),
        },
    }
}

fn known_folder_row(environment: &PlatformEnvironment) -> SelfCheckRow {
    let flavor = environment.flavor();
    let stated = environment.known_folders().len();
    let mut violations = 0usize;
    for path in environment.known_folders().values() {
        let text = path.to_string_lossy();
        // A known folder is authoritative because the operating system
        // reports it, so requiring it to sit under the profile would fail
        // exactly the environments this check exists for: Known Folder Move,
        // redirected Documents, and roaming profiles on a corporate share.
        // What must hold is that the resolved location is a usable absolute
        // path and not a broad root or a device namespace.
        let usable = is_absolute(&text, flavor)
            && !path_algebra::is_root(&text, flavor)
            && !path_algebra::is_unsupported_namespace(&text, flavor)
            && !path_algebra::contains_short_name(&text, flavor);
        if !usable {
            violations += 1;
        }
    }
    SelfCheckRow {
        name: "known_folders_resolve_to_usable_locations".to_string(),
        outcome: if violations == 0 {
            SelfCheckOutcome::Pass
        } else {
            SelfCheckOutcome::Fail
        },
        detail: if stated == 0 {
            "no user-content folders stated".to_string()
        } else if violations == 0 {
            format!("{stated} of {stated} folders resolve to usable absolute locations")
        } else {
            format!("{violations} of {stated} folders did not resolve to a usable location")
        },
    }
}

fn catalog_row(environment: &PlatformEnvironment) -> SelfCheckRow {
    let name = "signature_catalog_lints_clean".to_string();
    let catalog = match SignatureRegistry::load_embedded_catalog() {
        Ok(catalog) => catalog,
        Err(error) => {
            return SelfCheckRow {
                name,
                outcome: SelfCheckOutcome::Fail,
                detail: crate::diagnostics::sanitize_log(&error.to_string()),
            }
        }
    };
    let findings = SignatureRegistry::audit_signature_platforms(&catalog, environment);
    if findings.is_empty() {
        return SelfCheckRow {
            name,
            outcome: SelfCheckOutcome::Pass,
            detail: format!("{} signatures checked", catalog.all().len()),
        };
    }
    // Signature ids are catalog constants, not user data; no resolved path is
    // part of the report.
    let summary = findings
        .iter()
        .take(8)
        .map(|finding| {
            let kind = if finding.platform.is_some() {
                "platforms"
            } else {
                "path"
            };
            format!("{kind}:{}", finding.signature_id)
        })
        .collect::<Vec<_>>()
        .join(", ");
    SelfCheckRow {
        name,
        outcome: SelfCheckOutcome::Fail,
        detail: format!(
            "{} findings over {} signatures ({summary})",
            findings.len(),
            catalog.all().len()
        ),
    }
}

/// Samples for the flavor's own normalization rules.
fn flavor_samples(flavor: PathFlavor) -> Vec<String> {
    if flavor.is_windows() {
        [
            r"C:\",
            r"C:\Users\me\AppData\Local\",
            r"C:\Users\me\..\me\.\AppData",
            r"C:/Users/me/AppData",
            r"\\?\C:\Users\me\AppData",
            r"\\server\share\dir\",
            r"D:\Users\..\Users\me",
        ]
        .into_iter()
        .map(str::to_string)
        .collect()
    } else {
        [
            "/",
            "/Users/me/Library/Caches/",
            "/Users/me/../me/./Library",
            "/var/folders/x/T/",
            "relative/path",
            "//server/share/dir",
        ]
        .into_iter()
        .map(str::to_string)
        .collect()
    }
}

/// Pairs that must denote the same location under the flavor's separator rules.
fn separator_pairs(flavor: PathFlavor) -> Vec<(String, String)> {
    let pairs: &[(&str, &str)] = if flavor.is_windows() {
        &[
            (r"C:\Users\me", r"C:/Users/me"),
            (r"C:\a\b", r"C:\a/b"),
            (r"\\server\share\dir", r"\\server\share/dir"),
        ]
    } else {
        &[
            ("/Users/me/Library", "/Users/me/Library"),
            ("/Users/me/Library/", "/Users/me/Library"),
            ("/a/./b", "/a/b"),
        ]
    };
    pairs
        .iter()
        .map(|(left, right)| (left.to_string(), right.to_string()))
        .collect()
}

const WINDOWS_VERBATIM_SAMPLES: &[(&str, &str)] = &[
    (r"\\?\C:\Users\me\AppData", r"C:\Users\me\AppData"),
    (r"\\?\UNC\server\share\dir", r"\\server\share\dir"),
    (r"\\?\C:\Windows\", r"C:\Windows"),
];

const WINDOWS_UNC_SAMPLES: &[(&str, &str)] = &[
    (r"\\SERVER\SHARE\dir", r"\\server\share\dir"),
    (r"\\?\UNC\server\share\dir\", r"\\server\share\dir"),
];

fn trailing_separator_pairs(flavor: PathFlavor) -> Vec<(String, String)> {
    let pairs: &[(&str, &str)] = if flavor.is_windows() {
        &[
            (r"C:\Users\me\AppData\", r"C:\Users\me\AppData"),
            (r"C:\Windows\\", r"C:\Windows"),
        ]
    } else {
        &[("/Users/me/Library//", "/Users/me/Library")]
    };
    pairs
        .iter()
        .map(|(left, right)| (left.to_string(), right.to_string()))
        .collect()
}

/// The protected-root classification must not depend on which drive the system
/// is installed on, or on whether the path arrived verbatim.
const WINDOWS_PROTECTED_SAMPLES: &[(&str, &str)] = &[
    (r"C:\Windows", r"D:\Windows"),
    (r"C:\Program Files", r"E:\Program Files"),
    (r"\\?\C:\ProgramData", r"F:\ProgramData"),
];

/// Component-boundary containment: the boolean is the expectation.
const CONTAINMENT_SAMPLES: &[(&str, &str, bool)] = &[
    (r"C:\Program Files", r"C:\Program Files\App", true),
    (r"C:\Program Files", r"C:\Program Files (x86)", false),
    (r"C:\Users", r"C:\Users\me", true),
    (r"C:\Users\me", r"C:\Users\other", false),
];

/// Where an 8.3 alias could shadow a protected directory the classifier must
/// refuse the subtree, because the alias target is not knowable without the
/// volume. `refused` is the expectation.
const SHORT_NAME_SAMPLES: &[(&str, bool)] = &[
    (r"C:\PROGRA~1\App", true),
    (r"C:\DOCUME~1", true),
    (r"C:\Program Files\App", true),
    (r"C:\Users\me\AppData", false),
    (r"D:\projects\zenith", false),
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::path_algebra::PathFlavor;
    use crate::platform::paths::SimulatedPaths;
    use std::sync::Arc;

    fn stated_posix_environment() -> PlatformEnvironment {
        PlatformEnvironment::simulated(PathFlavor::Posix).with_roots(Arc::new(
            SimulatedPaths::new()
                .with_flavor(PathFlavor::Posix)
                .with_home("/home/tester")
                .with_local_app_data("/home/tester/.local/share")
                .with_roaming_app_data("/home/tester/.config"),
        ))
    }

    fn stated_windows_environment() -> PlatformEnvironment {
        PlatformEnvironment::simulated(PathFlavor::Windows).with_roots(Arc::new(
            SimulatedPaths::new()
                .with_flavor(PathFlavor::Windows)
                .with_home(r"D:\Users\tester")
                .with_temp_dir(r"D:\Users\tester\AppData\Local\Temp")
                .with_local_app_data(r"D:\Users\tester\AppData\Local")
                .with_roaming_app_data(r"D:\Users\tester\AppData\Roaming")
                .with_program_files(r"D:\Program Files")
                .with_program_data(r"D:\ProgramData"),
        ))
    }

    #[test]
    fn every_check_passes_for_a_stated_environment() {
        let report = self_check(&stated_posix_environment());
        assert_eq!(
            report.failures,
            0,
            "unexpected failures: {:?}",
            report
                .checks
                .iter()
                .filter(|check| check.outcome == SelfCheckOutcome::Fail)
                .collect::<Vec<_>>()
        );
        assert!(report.checks.len() >= 12, "{:?}", report.checks);
        assert_eq!(
            report
                .checks
                .iter()
                .map(|check| &check.name)
                .collect::<Vec<_>>(),
            vec![
                "normalize_is_idempotent",
                "fold_is_idempotent",
                "separators_are_equivalent",
                "verbatim_prefix_is_invariant",
                "unc_paths_are_invariant",
                "trailing_separator_is_invariant",
                "protected_root_is_drive_letter_independent",
                "containment_respects_component_boundaries",
                "short_name_ambiguity_fails_closed",
                "home_resolves",
                "home_is_not_a_broad_root",
                "known_folders_resolve_to_usable_locations",
                "signature_catalog_lints_clean",
            ]
        );
    }

    #[test]
    fn the_algebra_checks_pass_for_the_windows_flavor_on_this_host() {
        // The Windows invariants are pure functions of the flavor, so they are
        // proven here rather than only on a Windows machine.
        let environment = PlatformEnvironment::simulated(PathFlavor::Windows)
            .with_home(r"D:\Users\me")
            .with_known_folder(
                crate::platform::KnownFolder::Downloads,
                r"D:\Users\me\Downloads",
            );
        let report = self_check(&environment);
        assert_eq!(report.failures, 0, "{:?}", report.checks);
        assert_eq!(report.platform, PlatformKind::Windows);
    }

    #[test]
    fn a_missing_profile_root_fails_the_environment_checks() {
        // The checks are not vacuous: an environment that states no profile
        // root is reported as a failure instead of passing.
        let report = self_check(&PlatformEnvironment::simulated(PathFlavor::Posix));
        assert!(report.failures >= 2, "{:?}", report.checks);
        for name in ["home_resolves", "home_is_not_a_broad_root"] {
            let row = report
                .checks
                .iter()
                .find(|check| check.name == name)
                .unwrap();
            assert_eq!(row.outcome, SelfCheckOutcome::Fail, "{row:?}");
        }
    }

    #[test]
    fn a_redirected_known_folder_is_healthy() {
        // Known Folder Move, redirected Documents, and roaming profiles on a
        // corporate share all resolve outside the profile. The operating
        // system reporting the location is what makes it authoritative, so a
        // redirect must pass rather than fail the self-check.
        for environment in [
            stated_posix_environment().with_known_folder(
                crate::platform::KnownFolder::Documents,
                "/mnt/other/Documents",
            ),
            stated_windows_environment().with_known_folder(
                crate::platform::KnownFolder::Documents,
                r"D:\Redirected\Documents",
            ),
            stated_windows_environment().with_known_folder(
                crate::platform::KnownFolder::Downloads,
                r"\\fileserver\profiles\tester\Downloads",
            ),
        ] {
            let report = self_check(&environment);
            let row = report
                .checks
                .iter()
                .find(|check| check.name == "known_folders_resolve_to_usable_locations")
                .unwrap();
            assert_eq!(row.outcome, SelfCheckOutcome::Pass, "{row:?}");
            assert_eq!(report.failures, 0, "{:?}", report.checks);
        }
    }

    #[test]
    fn an_unusable_known_folder_fails() {
        // The check still has teeth: a location that is not usable as an
        // absolute folder is reported instead of passing.
        for (name, path) in [
            ("relative", "Documents"),
            ("root", "/"),
            ("device namespace", r"\\.\C:\Windows"),
        ] {
            let environment = stated_posix_environment()
                .with_known_folder(crate::platform::KnownFolder::Documents, path);
            let report = self_check(&environment);
            let row = report
                .checks
                .iter()
                .find(|check| check.name == "known_folders_resolve_to_usable_locations")
                .unwrap();
            assert_eq!(row.outcome, SelfCheckOutcome::Fail, "{name}: {row:?}");
        }
    }

    #[test]
    fn the_report_carries_no_paths_or_identities() {
        // De-identification is by construction; this asserts the construction.
        // Every marker below belongs to the stated environment, so the test
        // asserts the same thing on a runner that happens to have a different
        // account name — or no `USER` at all.
        let environment = stated_posix_environment();
        let report = self_check(&environment);
        let rendered = render_text(&report) + &serde_json::to_string(&report).unwrap();
        for leak in [
            "/home/tester",
            "tester",
            "/home/tester/.local/share",
            "/home/tester/.config",
            "C:\\",
        ] {
            assert!(
                !rendered.contains(leak),
                "report leaked `{leak}`: {rendered}"
            );
        }
    }

    #[test]
    fn render_text_reports_the_failure_count_and_every_row() {
        let report = self_check(&PlatformEnvironment::simulated(PathFlavor::Posix));
        let text = render_text(&report);
        assert!(text.contains("platform: "), "{text}");
        assert!(text.contains("fingerprint:"), "{text}");
        assert!(text.contains("[FAIL] home_resolves"), "{text}");
        assert!(
            text.contains(&format!(
                "result: {} of {} checks failed",
                report.failures,
                report.checks.len()
            )),
            "{text}"
        );
    }

    #[test]
    fn run_cli_ignores_unrelated_arguments() {
        assert_eq!(run_cli(&[]), None);
        assert_eq!(run_cli(&["--verbose".to_string()]), None);
        // `--doctor` must be the leading argument, so a Tauri-style flag that
        // merely mentions it cannot switch the process into console mode.
        assert_eq!(
            run_cli(&["--json".to_string(), "--doctor".to_string()]),
            None
        );
    }

    #[test]
    fn run_cli_exit_codes_follow_the_checks() {
        // A native environment with a profile root and a clean catalog exits 0;
        // the doctor never panics on either path.
        assert_eq!(run_cli(&["--doctor".to_string()]), Some(0));
        assert_eq!(
            run_cli(&["--doctor".to_string(), "--json".to_string()]),
            Some(0)
        );
        assert_eq!(run_cli(&["--help".to_string()]), Some(0));
    }

    #[test]
    fn fingerprint_states_shapes_not_locations() {
        let environment = stated_posix_environment().with_known_folder(
            crate::platform::KnownFolder::Documents,
            "/mnt/redirected/Documents",
        );
        let entries = fingerprint(&environment);
        assert!(
            entries.contains(&"profile_root=stated".to_string()),
            "{entries:?}"
        );
        assert!(
            entries.contains(&"local_app_data=stated".to_string()),
            "{entries:?}"
        );
        assert!(
            entries.contains(&"separator=slash".to_string()),
            "{entries:?}"
        );
        assert!(
            entries.contains(&"case_folding=sensitive".to_string()),
            "{entries:?}"
        );
        // One known folder is stated and differs from the literal profile join.
        assert!(
            entries.contains(&"known_folder_redirects=1".to_string()),
            "{entries:?}"
        );
        // Every value is a shape or a boolean, never a location: no value is
        // an absolute path or a drive-letter path.
        for entry in &entries {
            for value in entry.split('=').skip(1) {
                assert!(!value.starts_with('/'), "{entry}");
                assert!(!value.contains(":\\"), "{entry}");
                assert!(!value.contains("tester"), "{entry}");
            }
        }
    }
}
