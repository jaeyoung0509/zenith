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
use std::sync::LazyLock;

const RULE_ENV_GATE: &str = "env_gate";
const RULE_EARLY_RETURN: &str = "early_return";
const RULE_IDENTITY_CFG: &str = "identity_cfg";
const RULE_EXHAUSTIVE_MATCHES: &str = "exhaustive_matches";
const RULE_PLATFORM_GATED_MODULE: &str = "platform_gated_module";

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

static TEST_ATTRIBUTE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^#\[(?:tokio::)?test(?:\([^)]*\))?\]$").expect("static regex"));
static FUNCTION_NAME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"fn\s+([A-Za-z0-9_]+)").expect("static regex"));
static ASSERTION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"assert(_eq|_ne)?!|panic!|unreachable!").expect("static regex"));
/// Any environment lookup inside a test body, whatever the shape: `if let
/// Ok(..) = env::var(..)`, `let Ok(..) = env::var(..) else { .. }`, or a
/// helper that reads the variable. A test whose behaviour depends on the
/// runner's environment asserts nothing on a machine that does not set it.
static ENV_GATE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"env::var(?:_os)?\s*\(").expect("static regex"));
/// A `return` with no value: the shape that turns an unmeetable precondition
/// into a no-op. The value-returning form is a normal early exit from a helper
/// closure, so only the bare one is a skip.
static BARE_RETURN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\breturn\s*;").expect("static regex"));
static CFG_IDENTITY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"cfg!\((?:unix|windows|target_os)\s*[=),]").expect("static regex")
});
static EXHAUSTIVE_MATCHES: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?s)assert!\s*\(\s*matches!\s*\([^;]*?\|[^;]*?\)\s*\)").expect("static regex")
});
static MODULE_DECLARATION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:pub\s+)?(?:\([^)]*\)\s*)?mod\s+[A-Za-z0-9_]").expect("static regex")
});

struct TestBody {
    name: String,
    cfg: String,
    start_line: usize,
    text: String,
}

struct AttributeBlock {
    text: String,
    start_line: usize,
    end_line: usize,
}

/// Reads one complete Rust attribute, including a rustfmt-expanded multiline
/// attribute. Bracket counting uses code-only text so brackets inside a string
/// literal cannot terminate the attribute early.
fn attribute_block(lines: &[&str], start: usize) -> Option<AttributeBlock> {
    let first = lines.get(start)?.trim();
    if !first.starts_with("#[") && !first.starts_with("#![") {
        return None;
    }

    let mut depth = 0i32;
    let mut opened = false;
    let mut text = String::new();
    for (offset, line) in lines.iter().enumerate().skip(start) {
        let code = code_only(line);
        depth += code.matches('[').count() as i32;
        depth -= code.matches(']').count() as i32;
        opened |= code.contains('[');
        text.push_str(line.trim());
        text.push(' ');
        if opened && depth <= 0 {
            return Some(AttributeBlock {
                text: text.trim().to_string(),
                start_line: start,
                end_line: offset,
            });
        }
    }
    None
}

fn compact_attribute(attribute: &str) -> String {
    attribute
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect()
}

/// Extracts every `#[test]`/`#[tokio::test]` function body with the `cfg`
/// attributes that guard it.
fn test_bodies(source: &str) -> Vec<TestBody> {
    let lines: Vec<&str> = source.lines().collect();
    let mut bodies = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        let Some(first_attribute) = attribute_block(&lines, index) else {
            index += 1;
            continue;
        };

        // Collect the entire attribute block that carries this test. A `cfg`
        // normally precedes `#[test]`, but starting at `#[test]` loses the
        // condition the identity-cfg rule is meant to inspect.
        let mut attributes = vec![first_attribute];
        let mut cursor = attributes[0].end_line + 1;
        loop {
            while cursor < lines.len()
                && (lines[cursor].trim().is_empty() || lines[cursor].trim_start().starts_with("//"))
            {
                cursor += 1;
            }
            let Some(attribute) = attribute_block(&lines, cursor) else {
                break;
            };
            cursor = attribute.end_line + 1;
            attributes.push(attribute);
        }

        let Some(test_line) = attributes.iter().find_map(|attribute| {
            TEST_ATTRIBUTE
                .is_match(&compact_attribute(&attribute.text))
                .then_some(attribute.start_line)
        }) else {
            index += 1;
            continue;
        };

        let mut body_start = None;
        let mut signature = String::new();
        while cursor < lines.len() {
            let current = lines[cursor].trim();
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

        let name = FUNCTION_NAME
            .captures(&signature)
            .map(|captures| captures[1].to_string())
            .unwrap_or_else(|| "<anonymous>".to_string());
        let cfg = attributes
            .iter()
            .filter(|attribute| cfg_predicate(&attribute.text).is_some())
            .map(|attribute| attribute.text.clone())
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
            start_line: test_line + 1,
            text,
        });
        index = end_line.map(|line| line + 1).unwrap_or(index + 1);
    }
    bodies
}

/// The stable id a violation and an allowlist entry are matched on. Separators
/// are normalized so the same exception text describes the same file on a
/// Windows runner, which reports paths with a backslash.
fn location_of(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn assertion_marker(text: &str) -> Option<usize> {
    ASSERTION.find(text).map(|found| found.start())
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

    // String literals and comments are removed first so an `assert!(x ==
    // "env::var(...)")` or a commented example cannot be mistaken for the real
    // thing, and so an inline `if x { return; }` is found wherever it sits.
    let code = body
        .text
        .lines()
        .map(code_only)
        .collect::<Vec<_>>()
        .join("\n");

    if ENV_GATE.is_match(&code) {
        record(RULE_ENV_GATE);
    }

    if let Some(early) = BARE_RETURN.find(&code) {
        let asserted = assertion_marker(&code);
        if asserted.is_none_or(|asserted| early.start() < asserted) {
            record(RULE_EARLY_RETURN);
        }
    }

    if CFG_IDENTITY.is_match(&body.text)
        && (body.cfg.contains("unix")
            || body.cfg.contains("windows")
            || body.cfg.contains("target_os"))
    {
        record(RULE_IDENTITY_CFG);
    }

    if EXHAUSTIVE_MATCHES.is_match(&body.text) {
        record(RULE_EXHAUSTIVE_MATCHES);
    }

    violations
}

/// The `cfg` predicate of an attribute, if it is a `cfg` attribute. Whitespace
/// is insignificant, so this also accepts rustfmt-expanded multiline forms.
fn cfg_predicate(attribute: &str) -> Option<String> {
    let attribute = compact_attribute(attribute);
    let attribute = attribute.strip_prefix('#')?;
    let attribute = attribute.strip_prefix('!').unwrap_or(attribute);
    attribute
        .strip_prefix("[cfg(")
        .and_then(|inner| inner.strip_suffix(")]"))
        .map(str::to_string)
}

/// A `cfg` predicate that pins a suite to one platform cannot run anywhere
/// else, which is how the size measurer and directory scanner suites lost their
/// Windows coverage. Gating on a *selection* of platforms (`any(...)`,
/// `not(...)`, `all(...)` around a selection) is a deliberate choice.
fn is_platform_specific_predicate(predicate: &str) -> bool {
    if predicate.contains("any(") || predicate.contains("not(") {
        return false;
    }
    predicate.contains("unix") || predicate.contains("windows") || predicate.contains("target_os")
}

/// The next item after an attribute, skipping comments and any additional
/// attributes attached to the same item.
fn next_significant_item<'a>(lines: &'a [&str], from: usize) -> Option<&'a str> {
    let mut cursor = from + 1;
    while cursor < lines.len() {
        let line = lines[cursor].trim();
        if line.is_empty() || line.starts_with("//") {
            cursor += 1;
            continue;
        }
        if let Some(attribute) = attribute_block(lines, cursor) {
            cursor = attribute.end_line + 1;
            continue;
        }
        return Some(line);
    }
    None
}

fn file_level_violations(location: &str, source: &str) -> Vec<Violation> {
    let mut violations = Vec::new();
    let lines = source.lines().collect::<Vec<_>>();
    // An integration target under `tests/` exists only to run tests, so a
    // platform-gated module there is a whole suite that other runners skip.
    let is_test_target = location.starts_with("tests/") || location.starts_with("tests\\");
    let mut index = 0;
    while index < lines.len() {
        let Some(attribute) = attribute_block(&lines, index) else {
            index += 1;
            continue;
        };
        index = attribute.end_line + 1;
        let Some(predicate) = cfg_predicate(&attribute.text) else {
            continue;
        };
        if !is_platform_specific_predicate(&predicate) {
            continue;
        }
        let file_level = compact_attribute(&attribute.text).starts_with("#![cfg(");
        let gates_module = next_significant_item(&lines, attribute.end_line)
            .is_some_and(|next| MODULE_DECLARATION.is_match(next));
        let gated_suite = gates_module && (is_test_target || predicate.contains("test"));
        if file_level || gated_suite {
            violations.push(Violation {
                location: location.to_string(),
                rule: RULE_PLATFORM_GATED_MODULE,
                test: format!("platform-gated module (line {})", attribute.start_line + 1),
            });
        }
    }
    violations
}

/// The `path::test (line N)` ids named in `tests/hygiene_allowlist.txt`,
/// together with the reason each one carries.
///
/// An exception without a reason is not reviewable, and an entry that no longer
/// matches a violation silently widens the guard, so both are refused rather
/// than accepted.
fn allowlist() -> Vec<(String, String)> {
    let Ok(contents) = std::fs::read_to_string("tests/hygiene_allowlist.txt") else {
        return Vec::new();
    };
    allowlist_entries(&contents)
}

fn allowlist_entries(contents: &str) -> Vec<(String, String)> {
    contents
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| {
            let (id, reason) = line.split_once('#').unwrap_or((line, ""));
            (id.trim().to_string(), reason.trim().to_string())
        })
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
            .filter(|path| {
                path.file_name()
                    .is_some_and(|name| name != "test_hygiene.rs")
            }),
    ) {
        let Ok(source) = std::fs::read_to_string(&file) else {
            continue;
        };
        scanned += 1;
        let location = location_of(&file);
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
    let without_reason = allowed
        .iter()
        .filter(|(_, reason)| reason.is_empty())
        .map(|(id, _)| id.clone())
        .collect::<Vec<_>>();
    assert!(
        without_reason.is_empty(),
        "every tests/hygiene_allowlist.txt entry needs a `# reason`: {}",
        without_reason.join(", ")
    );

    let allowed_ids = allowed
        .iter()
        .map(|(id, _)| id.clone())
        .collect::<BTreeSet<_>>();
    let unexpected = violations
        .iter()
        .filter(|violation| !allowed_ids.contains(&violation.id()))
        .collect::<Vec<_>>();
    let matched = violations
        .iter()
        .map(|violation| violation.id())
        .collect::<BTreeSet<_>>();
    let stale = allowed_ids
        .iter()
        .filter(|id| !matched.contains(*id))
        .collect::<Vec<_>>();

    assert!(
        stale.is_empty(),
        "tests/hygiene_allowlist.txt grants exceptions that no longer match a violation, \
         which silently widens this guard: {}",
        stale
            .iter()
            .map(|id| id.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );

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

    // The guard itself must be able to fail: otherwise a broken detector
    // would report a clean tree forever.
    let env_gated = "#[test]\nfn skipped() {\n    if std::env::var(\"HOME\").is_ok() {\n        assert!(true);\n    }\n}\n";
    let early_return = "#[test]\nfn skipped_early() {\n    let Some(value) = precondition() else {\n        return;\n    };\n    assert!(value);\n}\n";
    let exhaustive = "#[test]\nfn accepts_everything() {\n    assert!(matches!(source, Ac | Battery | Unknown));\n}\n";

    let rules_for = |source: &str| -> BTreeSet<&'static str> {
        test_bodies(source)
            .iter()
            .flat_map(|body| inspect_test("<synthetic>", body))
            .map(|violation| violation.rule)
            .collect()
    };

    let env_rules = rules_for(env_gated);
    assert!(
        env_rules.contains(RULE_ENV_GATE),
        "the guard did not detect an environment-gated test body: {env_rules:?}"
    );
    let return_rules = rules_for(early_return);
    assert!(
        return_rules.contains(RULE_EARLY_RETURN),
        "the guard did not detect an early return standing in for an assertion: {return_rules:?}"
    );
    let exhaustive_rules = rules_for(exhaustive);
    assert!(
        exhaustive_rules.contains(RULE_EXHAUSTIVE_MATCHES),
        "the guard did not detect a matches! assertion over every variant: {exhaustive_rules:?}"
    );
    let cfg_before_test =
        "#[cfg(unix)]\n#[test]\nfn fixed_platform() {\n    assert_eq!(cfg!(unix), true);\n}\n";
    assert!(
        rules_for(cfg_before_test).contains(RULE_IDENTITY_CFG),
        "the guard lost a cfg attribute that precedes the test attribute"
    );

    // And it must not fire on the shapes that are legitimate.
    let allowed_shapes = "#[test]\nfn legit() {\n    let value = compute();\n    assert_eq!(value, 4);\n    if value > 2 {\n        return;\n    }\n    assert!(value < 100);\n}\n";
    assert!(
        rules_for(allowed_shapes).is_empty(),
        "the guard flagged a test that asserts before returning: {:?}",
        rules_for(allowed_shapes)
    );

    // The inline skip shapes a line-anchored detector misses.
    let inline_return = "#[test]\nfn skipped_inline() {\n    if precondition() { return; }\n    assert!(value);\n}\n";
    assert!(
        rules_for(inline_return).contains(RULE_EARLY_RETURN),
        "the guard did not detect an inline return standing in for an assertion"
    );
    let let_else_env = "#[test]\nfn skipped_env() {\n    let Ok(home) = std::env::var(\"HOME\") else { return; };\n    assert!(!home.is_empty());\n}\n";
    let let_else_rules = rules_for(let_else_env);
    assert!(
        let_else_rules.contains(RULE_ENV_GATE) && let_else_rules.contains(RULE_EARLY_RETURN),
        "the guard did not detect a `let ... else` environment gate: {let_else_rules:?}"
    );
    // A test that reads the environment through a helper is the same gate.
    let helper_env = "#[test]\nfn skipped_helper() {\n    let shape = describe(std::env::var_os(\"LOCALAPPDATA\"));\n    assert!(shape.is_some());\n}\n";
    assert!(
        rules_for(helper_env).contains(RULE_ENV_GATE),
        "the guard did not detect an environment lookup inside a test body"
    );
    // The value-returning form is a normal early exit from a closure.
    let value_return = "#[test]\nfn maps_values() {\n    let mapped = (|| {\n        if broken() {\n            return None;\n        }\n        Some(1)\n    })();\n    assert_eq!(mapped, Some(1));\n}\n";
    assert!(
        rules_for(value_return).is_empty(),
        "the guard flagged a value-returning early exit: {:?}",
        rules_for(value_return)
    );

    // Platform gates: a whole suite pinned to one platform is refused, a
    // deliberate platform *selection* and a production platform module are not.
    let gated_suite =
        "#[cfg(unix)]\nmod release_integration {\n    #[test]\n    fn runs() {\n        assert!(true);\n    }\n}\n";
    assert_eq!(
        file_level_violations("tests/dev_ports_tests.rs", gated_suite).len(),
        1,
        "an integration target gated on one platform must be reported"
    );
    assert_eq!(
        file_level_violations(
            "src/scanner/size.rs",
            "#[cfg(all(test, unix))]\nmod tests {\n}\n"
        )
        .len(),
        1,
        "a test module gated on one platform must be reported"
    );
    assert_eq!(
        file_level_violations(
            "src/scanner/size.rs",
            "#[cfg(\n    all(test, target_os = \"windows\")\n)]\n#[allow(dead_code)]\nmod tests {\n}\n"
        )
        .len(),
        1,
        "a multiline cfg attribute must not hide a platform-gated test module"
    );
    assert_eq!(
        file_level_violations("src/lib.rs", "#![cfg(target_os = \"windows\")]\n").len(),
        1,
        "a file-level platform gate must be reported"
    );
    assert_eq!(
        file_level_violations("src/lib.rs", "#![cfg(\n    target_os = \"windows\"\n)]\n").len(),
        1,
        "a multiline file-level platform gate must be reported"
    );
    assert!(
        file_level_violations(
            "src/power/assertion.rs",
            "#[cfg(all(test, not(any(target_os = \"macos\", target_os = \"windows\"))))]\nmod unsupported_tests {\n}\n"
        )
        .is_empty(),
        "a platform selection gate is not a platform-specific suite"
    );
    assert!(
        file_level_violations(
            "src/power/assertion.rs",
            "#[cfg(target_os = \"macos\")]\nmod macos_iokit {\n}\n"
        )
        .is_empty(),
        "a production platform module is not a test suite"
    );
    assert!(
        file_level_violations(
            "src/metrics/memory.rs",
            "#[cfg(target_os = \"windows\")]\nfn native_handle() -> u32 {\n}\n"
        )
        .is_empty(),
        "a platform-gated function is not a suite"
    );

    // A violation id is platform-independent, so one allowlist entry describes
    // the same exception on every runner.
    assert_eq!(
        location_of(Path::new("tests\\dev_ports_tests.rs")),
        "tests/dev_ports_tests.rs"
    );

    // An exception carries its reason, and a bare path is refused.
    let parsed = allowlist_entries(
        "# a comment\n\n  tests/dev_ports_tests.rs::platform-gated module (line 6) # Unix-only helper process\n",
    );
    assert_eq!(
        parsed,
        vec![(
            "tests/dev_ports_tests.rs::platform-gated module (line 6)".to_string(),
            "Unix-only helper process".to_string()
        )]
    );
    assert!(!parsed.iter().any(|(_, reason)| reason.is_empty()));
}
