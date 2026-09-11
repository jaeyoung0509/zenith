//! Fails on new silent test skips.
//!
//! A test that is green because its body never ran is worse than no test: it
//! reports coverage that does not exist, and it hides the defect the test was
//! written for. This guard reads the test sources and refuses four shapes that
//! have been found in this tree:
//!
//! 1. a test body gated on an environment variable, so it asserts nothing when
//!    the variable is absent (`HOME` is undefined on Windows);
//! 2. a bare `return` before any assertion, so an unmeetable precondition turns
//!    the test into a no-op instead of a failure;
//! 3. a comparison against a `cfg!` that the test's own `#[cfg]` already fixed,
//!    which the compiler resolves to a constant;
//! 4. a `matches!` assertion that accepts every variant of the value it is
//!    handed, which cannot fail.
//!
//! An intentional exception must be named in `tests/hygiene_allowlist.txt` with
//! a reason, which makes the exception reviewable instead of invisible.

use regex::Regex;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

const RULE_ENV_GATE: &str = "env_gate";
const RULE_EARLY_RETURN: &str = "early_return";
const RULE_IDENTITY_CFG: &str = "identity_cfg";
const RULE_EXHAUSTIVE_MATCHES: &str = "exhaustive_matches";

struct Violation {
    location: String,
    rule: &'static str,
    test: String,
}

impl Violation {
    fn id(&self) -> String {
        format!("{}::{}", self.location, self.test)
    }
}

fn source_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for entry in walkdir::WalkDir::new(root)
        .into_iter()
        .filter_map(Result::ok)
    {
        let path = entry.path();
        if path.extension().is_some_and(|extension| extension == "rs") {
            files.push(path.to_path_buf());
        }
    }
    files.sort();
    files
}

/// Removes string literals and comments so brace counting is not confused by
/// braces that appear inside text.
fn code_only(line: &str) -> String {
    let mut result = String::with_capacity(line.len());
    let mut in_string = false;
    let mut in_char = false;
    let mut escaped = false;
    let mut chars = line.chars().peekable();
    while let Some(character) = chars.next() {
        if in_string {
            if escaped {
                escaped = false;
                continue;
            }
            match character {
                '\\' => escaped = true,
                '"' => in_string = false,
                _ => {}
            }
            continue;
        }
        if in_char {
            if escaped {
                escaped = false;
                continue;
            }
            match character {
                '\\' => escaped = true,
                '\'' => in_char = false,
                _ => {}
            }
            continue;
        }
        match character {
            '"' => in_string = true,
            '\'' => {
                // A lifetime (`'a`) is not a character literal.
                if chars.peek().is_some_and(|next| next.is_alphanumeric()) {
                    result.push(character);
                } else {
                    in_char = true;
                }
            }
            '/' if chars.peek() == Some(&'/') => break,
            _ => result.push(character),
        }
    }
    result
}

struct TestBody {
    name: String,
    cfg: String,
    start_line: usize,
    text: String,
}

/// Extracts every `#[test]`/`#[tokio::test]` function body with the `cfg`
/// attributes that guard it.
fn test_bodies(source: &str) -> Vec<TestBody> {
    let lines: Vec<&str> = source.lines().collect();
    let mut bodies = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        let line = lines[index].trim();
        if !(line == "#[test]"
            || line.starts_with("#[test ")
            || line == "#[tokio::test]"
            || line.starts_with("#[tokio::test")
            || line == "#[test]")
        {
            index += 1;
            continue;
        }

        // Collect the attribute block that carries this test, including any
        // #[cfg(...)] that follows it before the signature.
        let mut attributes = Vec::new();
        let mut cursor = index;
        let mut body_start = None;
        let mut signature = String::new();
        while cursor < lines.len() {
            let current = lines[cursor].trim();
            if current.starts_with("#[") {
                attributes.push(current.to_string());
                cursor += 1;
                continue;
            }
            if current.is_empty() {
                cursor += 1;
                continue;
            }
            signature.push_str(lines[cursor]);
            if code_only(lines[cursor]).contains('{') {
                body_start = Some(cursor);
                break;
            }
            signature.push(' ');
            cursor += 1;
        }
        let Some(body_start) = body_start else {
            index += 1;
            continue;
        };

        let name = Regex::new(r"fn\s+([A-Za-z0-9_]+)")
            .expect("static regex")
            .captures(&signature)
            .map(|captures| captures[1].to_string())
            .unwrap_or_else(|| "<anonymous>".to_string());
        let cfg = attributes
            .iter()
            .filter(|attribute| attribute.contains("cfg("))
            .cloned()
            .collect::<Vec<_>>()
            .join(" ");

        let mut depth = 0i32;
        let mut text = String::new();
        let mut cursor = body_start;
        let mut end_line = None;
        while cursor < lines.len() {
            let code = code_only(lines[cursor]);
            depth += code.matches('{').count() as i32;
            depth -= code.matches('}').count() as i32;
            text.push_str(lines[cursor]);
            text.push('\n');
            if depth <= 0 && text.contains('{') {
                end_line = Some(cursor);
                break;
            }
            cursor += 1;
        }
        bodies.push(TestBody {
            name,
            cfg,
            start_line: index + 1,
            text,
        });
        index = end_line.map(|line| line + 1).unwrap_or(index + 1);
    }
    bodies
}

fn assertion_marker(text: &str) -> Option<usize> {
    let regex = Regex::new(r"assert(_eq|_ne)?!|panic!|unreachable!").expect("static regex");
    regex.find(text).map(|found| found.start())
}

fn inspect_test(location: &str, body: &TestBody) -> Vec<Violation> {
    let mut violations = Vec::new();
    let mut record = |rule: &'static str| {
        violations.push(Violation {
            location: location.to_string(),
            rule,
            test: format!("{} (line {})", body.name, body.start_line),
        });
    };

    let env_gate = Regex::new(r"(?m)^\s*(?:\}\s*)?if\s+(?:let\s+)?[^\n]*env::var")
        .expect("static regex");
    if env_gate.is_match(&body.text) {
        record(RULE_ENV_GATE);
    }

    let return_regex = Regex::new(r"(?m)^\s*return;\s*$").expect("static regex");
    if let Some(early) = return_regex.find(&body.text) {
        let asserted = assertion_marker(&body.text);
        if asserted.is_none_or(|asserted| early.start() < asserted) {
            record(RULE_EARLY_RETURN);
        }
    }

    let cfg_identity = Regex::new(r"cfg!\((?:unix|windows|target_os)\s*[=),]").expect("static regex");
    if cfg_identity.is_match(&body.text)
        && (body.cfg.contains("unix")
            || body.cfg.contains("windows")
            || body.cfg.contains("target_os"))
    {
        record(RULE_IDENTITY_CFG);
    }

    let exhaustive = Regex::new(r"(?s)assert!\s*\(\s*matches!\s*\([^;]*?\|[^;]*?\)\s*\)")
        .expect("static regex");
    if exhaustive.is_match(&body.text) {
        record(RULE_EXHAUSTIVE_MATCHES);
    }

    violations
}

fn file_level_violations(location: &str, source: &str) -> Vec<Violation> {
    let mut violations = Vec::new();
    for (index, line) in source.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with("#![cfg(") || trimmed.starts_with("#[cfg(all(test,") {
            violations.push(Violation {
                location: location.to_string(),
                rule: RULE_ENV_GATE,
                test: format!("whole-file gate (line {})", index + 1),
            });
        }
    }
    violations
}

fn allowlist() -> BTreeSet<String> {
    let path = Path::new("tests/hygiene_allowlist.txt");
    let Ok(contents) = std::fs::read_to_string(path) else {
        return BTreeSet::new();
    };
    contents
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| line.split('#').next().unwrap_or(line).trim().to_string())
        .collect()
}

#[test]
fn no_test_skips_or_unfailable_assertions_are_added() {
    let mut violations = Vec::new();
    let mut scanned = 0usize;

    for file in source_files(Path::new("src")).into_iter().chain(
        source_files(Path::new("tests"))
            .into_iter()
            // This guard's own source contains the patterns it looks for.
            .filter(|path| path.file_name().is_some_and(|name| name != "test_hygiene.rs")),
    ) {
        let Ok(source) = std::fs::read_to_string(&file) else {
            continue;
        };
        scanned += 1;
        let location = file.to_string_lossy().to_string();
        violations.extend(file_level_violations(&location, &source));
        for body in test_bodies(&source) {
            violations.extend(inspect_test(&location, &body));
        }
    }

    assert!(
        scanned > 50,
        "expected to scan the test tree, scanned only {scanned} files"
    );

    let allowed = allowlist();
    let unexpected = violations
        .iter()
        .filter(|violation| !allowed.contains(&violation.id()))
        .collect::<Vec<_>>();

    if !unexpected.is_empty() {
        let report = unexpected
            .iter()
            .map(|violation| {
                format!(
                    "  {} [{}] {}",
                    violation.id(),
                    violation.rule,
                    violation.test
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        panic!(
            "{} test(s) can pass without asserting:\n{report}\n\n\
             Fix the test, or add `<path>::<test> # reason` to tests/hygiene_allowlist.txt.",
            unexpected.len()
        );
    }

    // The guard itself must be able to fail: a synthetic offender has to be
    // reported, otherwise this test would pass on a broken detector.
    let synthetic = "#[test]\nfn skipped() {\n    if std::env::var(\"HOME\").is_ok() {\n        assert!(true);\n    } else {\n        return;\n    }\n}\n";
    let detected = test_bodies(synthetic)
        .iter()
        .flat_map(|body| inspect_test("<synthetic>", body))
        .collect::<Vec<_>>();
    let rules = detected
        .iter()
        .map(|violation| violation.rule)
        .collect::<BTreeSet<_>>();
    assert!(
        rules.contains(RULE_ENV_GATE) && rules.contains(RULE_EARLY_RETURN),
        "the guard did not detect a synthetic skip: {rules:?}"
    );
}
