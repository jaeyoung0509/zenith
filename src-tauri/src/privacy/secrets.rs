//! Single source of truth for credential-shaped strings.
//!
//! The diagnostics sanitizer and the AI Control Center secret scanner both read
//! this table so detection coverage cannot diverge between the two. Adding a
//! credential-bearing provider means adding a sample in
//! [`provider_credential_samples`]; the coverage test fails otherwise.

use regex::Regex;
use std::sync::LazyLock;

/// Where a scanned line came from.
///
/// An assignment is reported broadly in a credential-bearing file, where a
/// password may contain punctuation, and conservatively in source, where the
/// same text is usually an identifier or an expression.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanContext {
    /// Source, tests, and documentation.
    Source,
    /// `.env`, credential stores, and configuration files.
    CredentialFile,
}

/// A credential pattern. Detection and redaction are separate regexes because
/// their correct semantics differ: a private-key header is enough to detect a
/// secret but redaction must remove the whole key block; an assignment whose
/// value is an ordinary identifier is source code, while a sanitizer must still
/// redact it. `replacement` keeps any capture groups that must survive
/// redaction (for example the assignment prefix or an authorization scheme) and
/// replaces everything else with `[REDACTED]`.
pub struct SecretPattern {
    pub category: &'static str,
    pub detector: Regex,
    pub redactor: Regex,
    pub replacement: &'static str,
    /// Optional extra condition the detector's first capture group must
    /// satisfy. Detection is deliberately narrower than redaction: the scanner
    /// must report real exposures, not every line that mentions a password.
    /// A detector that carries a guard therefore captures the value first.
    pub detector_guard: Option<fn(&str) -> bool>,
    /// Detector for credential-bearing files, where an assignment is reported
    /// even when its value is not credential-shaped. `None` means the detector
    /// above applies in both contexts.
    pub credential_file_detector: Option<Regex>,
}

macro_rules! pattern {
    ($category:expr, $regex:expr, $replacement:expr) => {
        SecretPattern {
            category: $category,
            detector: Regex::new($regex).expect("valid secret detector"),
            redactor: Regex::new($regex).expect("valid secret redactor"),
            replacement: $replacement,
            detector_guard: None,
            credential_file_detector: None,
        }
    };
}

macro_rules! block_pattern {
    ($category:expr, $detector:expr, $redactor:expr, $replacement:expr) => {
        SecretPattern {
            category: $category,
            detector: Regex::new($detector).expect("valid secret detector"),
            redactor: Regex::new($redactor).expect("valid secret redactor"),
            replacement: $replacement,
            detector_guard: None,
            credential_file_detector: None,
        }
    };
}

/// A shape whose detector and redactor deliberately differ, with the shared
/// replacement expressed in terms of the redactor's capture groups.
macro_rules! split_pattern {
    ($category:expr, $redactor:expr, $detector:expr, $replacement:expr) => {
        split_pattern!($category, $redactor, $detector, $replacement, None, None)
    };
    ($category:expr, $redactor:expr, $detector:expr, $replacement:expr, $guard:expr) => {
        split_pattern!($category, $redactor, $detector, $replacement, $guard, None)
    };
    (
        $category:expr,
        $redactor:expr,
        $detector:expr,
        $replacement:expr,
        $guard:expr,
        $credential_file_detector:expr
    ) => {{
        let credential_file_detector: Option<String> = $credential_file_detector;
        SecretPattern {
            category: $category,
            detector: Regex::new(&$detector).expect("valid secret detector"),
            redactor: Regex::new(&$redactor).expect("valid secret redactor"),
            replacement: $replacement,
            detector_guard: $guard,
            credential_file_detector: credential_file_detector
                .map(|source| Regex::new(&source).expect("valid credential file detector")),
        }
    }};
}

/// Credential key names that introduce an assigned value, shared by every
/// assignment shape so detection and redaction cannot drift apart.
const ASSIGNMENT_KEYS: &str = r#"(?:api[_-]?key|access[_-]?key|secret[_-]?access[_-]?key|client[_-]?secret|private[_-]?key|auth[_-]?token|access[_-]?token|refresh[_-]?token|id[_-]?token|token|secret|password|passwd|pwd)"#;

/// Shortest quoted value the scanner reports as an assigned credential.
const MIN_DETECTED_QUOTED_VALUE: usize = 8;

/// Shortest unquoted credential-shaped token the scanner reports. Below this
/// the value is indistinguishable from an identifier, and the scanner must not
/// turn `password = some_identifier` into a critical finding.
const MIN_DETECTED_UNQUOTED_VALUE: usize = 12;

/// The `key =` prefix shared by every assignment shape. Its captured group is
/// what survives redaction, so the value is always the shape's second group.
fn assignment_prefix(quote: Option<char>) -> String {
    match quote {
        Some(quote) => format!(r#"(?i)(["']?{ASSIGNMENT_KEYS}["']?\s*[:=]\s*{quote})"#),
        None => format!(r#"(?i)(["']?{ASSIGNMENT_KEYS}["']?\s*[:=]\s*)"#),
    }
}

/// A quoted value body in which `\\.` consumes an escaped character, so an
/// escaped quote inside the value is not treated as its terminator.
fn quoted_value_body(quote: char) -> String {
    format!(r"(?:\\.|[^{quote}\\\r\n])")
}

/// A value that reached the end of its line, possibly cut off in the middle of
/// an escape sequence or a Windows line ending.
const LINE_END: &str = r"\\?\r?";

/// Assignment shapes for `quote`-delimited values.
///
/// Detection is stricter than redaction on purpose: a sanitizer must
/// over-redact anything it cannot fully parse, while the scanner must not
/// report ordinary source as an exposed credential. Every shape therefore
/// carries the same prefix and replacement, and only the value width differs.
fn quoted_assignment_patterns(quote: char, patterns: &mut Vec<SecretPattern>) {
    let value = quoted_value_body(quote);
    let prefix = assignment_prefix(Some(quote));
    // A closing quote followed by anything other than a delimiter does not
    // terminate the value: everything from that point to the end of the line is
    // the secret, because a sanitizer that stopped at the second quote would
    // expose whatever follows it.
    let invalid_close = format!(r#"{prefix}({value}*){quote}[^\s,;:)\]}}>][^\r\n]*"#);
    patterns.push(split_pattern!(
        "Credential assignment",
        invalid_close,
        invalid_close,
        "${1}[REDACTED]"
    ));
    let complete_detector = format!(r#"{prefix}({value}{{{MIN_DETECTED_QUOTED_VALUE},}}){quote}"#);
    let complete_redactor = format!(r#"{prefix}({value}+)({quote})"#);
    // Any non-empty quoted value is reported in a credential file: short
    // passwords are normal there and a missed one costs more than a finding.
    let complete_credential_file_detector = format!(r#"{prefix}({value}+)({quote})"#);
    patterns.push(split_pattern!(
        "Credential assignment",
        complete_redactor,
        complete_detector,
        "${1}[REDACTED]${3}",
        None,
        Some(complete_credential_file_detector)
    ));
    // An opening quote with no closing quote runs to the end of the line: the
    // closing quote may have been cut off by truncation and whitespace is part
    // of the value. The detector requires exactly that end-of-line shape, so a
    // closed quote further along the line is left to the shapes above.
    let open_detector =
        format!(r#"(?m){prefix}({value}{{{MIN_DETECTED_QUOTED_VALUE},}}{LINE_END})$"#);
    let open_redactor = format!(r#"(?m){prefix}({value}*{LINE_END})$"#);
    let open_credential_file_detector = format!(r#"(?m){prefix}({value}+{LINE_END})$"#);
    patterns.push(split_pattern!(
        "Credential assignment",
        open_redactor,
        open_detector,
        "${1}[REDACTED]",
        None,
        Some(open_credential_file_detector)
    ));
}

/// Longest unquoted value that still needs a mixed case to be reported in
/// source. A single-case run this long is a generated key (a hex digest, a
/// base64 blob) rather than an identifier.
const LONG_DETECTED_UNQUOTED_VALUE: usize = 32;

/// Whether an assigned value reads like a generated secret rather than an
/// identifier, a path, or a function call.
///
/// The scanner runs over source trees, where `let token = platform_token2`,
/// `secret: SecretString`, or `tokens.get('ring')` is ordinary code. A
/// generated key carries a digit together with a mixed case, or is long enough
/// that an identifier is implausible, so those are the shapes source reports.
/// A credential file is scanned broadly instead, because a password there may
/// be lower case, short, or full of punctuation.
fn looks_like_a_secret(value: &str) -> bool {
    let has_digit = value.chars().any(|c| c.is_ascii_digit());
    let has_lowercase = value.chars().any(|c| c.is_ascii_lowercase());
    let has_uppercase = value.chars().any(|c| c.is_ascii_uppercase());
    value.len() >= MIN_DETECTED_UNQUOTED_VALUE
        && has_digit
        && ((has_lowercase && has_uppercase) || value.len() >= LONG_DETECTED_UNQUOTED_VALUE)
}

/// Assignment shapes for unquoted values.
fn unquoted_assignment_patterns(patterns: &mut Vec<SecretPattern>) {
    let prefix = assignment_prefix(None);
    // Redaction runs to the next whitespace or newline: anything narrower
    // (`&`, `;`, `?`, `,`) leaves a credential suffix behind, and a sanitizer
    // must over-redact rather than under-redact.
    let redactor = format!(r#"{prefix}([^\s"'\r\n]{{2,}})"#);
    // Source detection only accepts credential-shaped tokens, so identifiers,
    // paths, and punctuation-heavy code are not reported as secrets. Its prefix
    // does not capture: the guard inspects the detector's first capture group,
    // which is the value.
    let detector_prefix = format!(r#"(?i)["']?{ASSIGNMENT_KEYS}["']?\s*[:=]\s*"#);
    let detector =
        format!(r#"{detector_prefix}([A-Za-z0-9+/=_.-]{{{MIN_DETECTED_UNQUOTED_VALUE},}})"#);
    // A credential file holds exactly this shape, so punctuation is part of the
    // value: `password=p@ssw0rd!very-secret` is a credential there while the
    // same text in source is an expression.
    let credential_file_detector = format!(r#"{prefix}([^\s"'\r\n]+)"#);
    patterns.push(split_pattern!(
        "Credential assignment",
        redactor,
        detector,
        "${1}[REDACTED]",
        Some(looks_like_a_secret),
        Some(credential_file_detector)
    ));
}

static PATTERNS: LazyLock<Vec<SecretPattern>> = LazyLock::new(|| {
    // Both PGP headers are assembled from parts so the source text does not
    // carry the complete signature the scanner looks for.
    let pgp_header = ["-----BEGIN PGP ", "PRIVATE KEY BLOCK-----"].concat();
    let pgp_end = ["-----END PGP ", "PRIVATE KEY BLOCK-----"].concat();
    let pgp_block = format!(r"(?s){pgp_header}.*?(?:{pgp_end}|$)");
    let assignment_patterns = {
        let mut patterns = Vec::new();
        quoted_assignment_patterns('"', &mut patterns);
        quoted_assignment_patterns('\'', &mut patterns);
        unquoted_assignment_patterns(&mut patterns);
        patterns
    };
    let mut patterns = vec![
        block_pattern!(
            "Private key material",
            r"-----BEGIN (?:RSA |EC |DSA |OPENSSH |ENCRYPTED )?PRIVATE KEY-----",
            r"(?s)-----BEGIN (?:RSA |EC |DSA |OPENSSH |ENCRYPTED )?PRIVATE KEY-----.*?(?:-----END (?:RSA |EC |DSA |OPENSSH |ENCRYPTED )?PRIVATE KEY-----|$)",
            "[REDACTED]"
        ),
        block_pattern!(
            "Private key material",
            &pgp_header,
            &pgp_block,
            "[REDACTED]"
        ),
        pattern!(
            "GitHub token",
            r"\bgithub_pat_[A-Za-z0-9_]{20,}\b",
            "[REDACTED]"
        ),
        pattern!(
            "GitHub token",
            r"\bgh[pousr]_[A-Za-z0-9]{20,}\b",
            "[REDACTED]"
        ),
        pattern!(
            "GitLab token",
            r"\bglpat-[A-Za-z0-9_\-]{20,}\b",
            "[REDACTED]"
        ),
        pattern!(
            "Slack token",
            r"\bxox[baprs]-[A-Za-z0-9-]{10,}\b",
            "[REDACTED]"
        ),
        pattern!(
            "Stripe key",
            r"\b(?:sk|rk)_live_[A-Za-z0-9]{16,}\b",
            "[REDACTED]"
        ),
        pattern!(
            "OpenAI-style API key",
            r"\bsk-[A-Za-z0-9_\-]{8,}\b",
            "[REDACTED]"
        ),
        pattern!("xAI API key", r"\bxai-[A-Za-z0-9_\-]{16,}\b", "[REDACTED]"),
        pattern!(
            "Fireworks API key",
            r"\bfw_[A-Za-z0-9]{16,}\b",
            "[REDACTED]"
        ),
        pattern!(
            "Hugging Face token",
            r"\bhf_[A-Za-z0-9]{20,}\b",
            "[REDACTED]"
        ),
        pattern!("npm token", r"\bnpm_[A-Za-z0-9]{20,}\b", "[REDACTED]"),
        pattern!(
            "SendGrid API key",
            r"\bSG\.[A-Za-z0-9_\-]{16,}\.[A-Za-z0-9_\-]{16,}\b",
            "[REDACTED]"
        ),
        pattern!(
            "Google API key",
            r"\bAIza[0-9A-Za-z_\-]{35}\b",
            "[REDACTED]"
        ),
        pattern!(
            "AWS access key id",
            r"\b(?:AKIA|ASIA)[0-9A-Z]{16}\b",
            "[REDACTED]"
        ),
        pattern!(
            "AWS secret access key",
            r#"(?i)(aws[_-]?secret[_-]?access[_-]?key\s*[:=]\s*["']?)[A-Za-z0-9/+=]{40}"#,
            "${1}[REDACTED]"
        ),
        pattern!(
            "JSON Web Token",
            r"\beyJ[A-Za-z0-9_\-]{10,}\.[A-Za-z0-9_\-]{10,}\.[A-Za-z0-9_\-]{10,}\b",
            "[REDACTED]"
        ),
        // Redaction keeps the scheme and removes whatever follows it, because a
        // credential can be any text. Detection additionally requires a
        // credential-shaped value, so `start_authorization: impl FnOnce…` in
        // source is not reported as an exposed header.
        split_pattern!(
            "Authorization header",
            r#"(?i)(authorization\s*[:=]\s*)([A-Za-z][A-Za-z0-9._-]*\s+)?([^\s,;"']{4,})"#,
            r#"(?i)(authorization\s*[:=]\s*)((?:basic|bearer|api[_-]?key|apikey|token)\s+)?([A-Za-z0-9_\-\.=+/]{8,})"#,
            "${1}${2}[REDACTED]"
        ),
        pattern!(
            "Bearer token",
            r"(?i)(bearer\s+)[A-Za-z0-9_\-\.=+/]{8,}",
            "${1}[REDACTED]"
        ),
        pattern!(
            "URL query credential",
            r#"(?i)([?&](?:access[_-]?token|api[_-]?key|apikey|token|secret|password|key)=)[^&\s"']+"#,
            "${1}[REDACTED]"
        ),
        pattern!(
            "URL userinfo credential",
            r"(?i)(://[^/\s:@]{1,256}:)[^@/\s]{1,256}@",
            "${1}[REDACTED]@"
        ),
    ];
    patterns.extend(assignment_patterns);
    patterns
});

pub fn patterns() -> &'static [SecretPattern] {
    &PATTERNS
}

/// Replaces every credential-shaped substring with `[REDACTED]`.
pub fn redact(text: &str) -> String {
    let mut result = text.to_string();
    for pattern in patterns() {
        result = pattern
            .redactor
            .replace_all(&result, pattern.replacement)
            .into_owned();
    }
    result
}

/// Whether the text contains any credential shape. Used by the scanner, which
/// intentionally never extracts or returns the matched value.
pub fn contains_secret(text: &str) -> bool {
    contains_secret_in(text, ScanContext::Source)
}

/// Context-aware [`contains_secret`].
pub fn contains_secret_in(text: &str, context: ScanContext) -> bool {
    patterns()
        .iter()
        .any(|pattern| detector_matches(pattern, text, context))
}

/// Returns the category of the first matching credential shape without
/// extracting the value.
pub fn match_category(text: &str) -> Option<&'static str> {
    match_category_in(text, ScanContext::Source)
}

/// Context-aware [`match_category`].
///
/// A credential-bearing file reports an assigned value that source would not:
/// `password=p@ssw0rd!very-secret` is a credential there, while the same text in
/// source is an expression.
pub fn match_category_in(text: &str, context: ScanContext) -> Option<&'static str> {
    patterns()
        .iter()
        .find(|pattern| detector_matches(pattern, text, context))
        .map(|pattern| pattern.category)
}

/// Whether a pattern's detector applies, including its optional value guard.
///
/// The guard only narrows the source context: it exists so identifiers are not
/// reported as credentials, and a credential file is scanned for values of any
/// shape instead.
fn detector_matches(pattern: &SecretPattern, text: &str, context: ScanContext) -> bool {
    if context == ScanContext::CredentialFile {
        if let Some(broad) = &pattern.credential_file_detector {
            return broad.is_match(text);
        }
    }
    match pattern.detector_guard {
        None => pattern.detector.is_match(text),
        Some(guard) => pattern
            .detector
            .captures_iter(text)
            .any(|captures| captures.get(1).is_some_and(|value| guard(value.as_str()))),
    }
}

/// Representative credential strings for every provider that stores a secret.
/// A provider added without a sample fails
/// `every_registry_credential_format_is_covered`.
///
/// Samples are assembled at runtime so the source tree does not carry
/// credential-shaped literals that trip secret scanners.
pub fn provider_credential_samples() -> Vec<(crate::models::ProviderId, String)> {
    use crate::models::ProviderId;
    let jwt =
        |header: &str, payload: &str, signature: &str| format!("{header}.{payload}.{signature}");
    vec![
        (
            ProviderId::Codex,
            jwt(
                "eyJhbGciOiJSUzI1NiIsImtpZCI6ImFiYyJ9",
                "eyJzdWIiOiIxMjM0NTY3ODkwIn0",
                "SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV_adQssw5c",
            ),
        ),
        (
            ProviderId::OpenRouter,
            format!(
                "sk-or-v1-{}",
                "abcdefghijklmnopqrstuvwxyz0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ01"
            ),
        ),
        (
            ProviderId::Antigravity,
            jwt(
                "eyJhbGciOiJFUzI1NiIsImtpZCI6InRlc3QifQ",
                "eyJzdWIiOiIxMjM0NTY3ODkwIn0",
                "c2lnbmF0dXJlLXNlZ21lbnQ",
            ),
        ),
        (
            ProviderId::XaiApi,
            format!("xai-{}", "abcdefghijklmnopqrstuvwx0123456789ABCD"),
        ),
        (
            ProviderId::OpenAiApi,
            format!("sk-proj-{}", "abcdefghijklmnopqrstuvwxyz0123456789ABCDEFGH"),
        ),
        (
            ProviderId::AnthropicApi,
            format!(
                "{}{}{}",
                "sk-ant-", "api03-", "abcdefghijklmnopqrstuvwxyz0123456789ABCDEFGH"
            ),
        ),
        (
            ProviderId::MetaModelApi,
            format!(
                "META_API_KEY={}",
                "abcdefghijklmnopqrstuvwxyz0123456789ABCD"
            ),
        ),
        (
            ProviderId::MistralApi,
            format!(
                "MISTRAL_API_KEY={}",
                "abcdefghijklmnopqrstuvwxyz0123456789ABCD"
            ),
        ),
        (
            ProviderId::FireworksApi,
            format!("fw_{}", "abcdefghijklmnopqrstuvwx0123"),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai_providers::registry::{CredentialKind, ProviderRegistry};

    /// Builds `<key>=<value>` at runtime. The safety scanner inspects Zenith's
    /// own repository, so a fixture line must not contain a complete
    /// `key=` / `key="` signature: an exposed-credential finding must always
    /// mean a real credential, never this file's own test data.
    fn assignment(key: &str, value: &str) -> String {
        format!("{key}={value}")
    }

    /// Builds the JSON `"key": "value"` pair at runtime, for the same reason.
    fn json_pair(key: &str, value: &str) -> String {
        format!("\"{key}\": \"{value}\"")
    }

    /// Joins credential parts at runtime so no source line carries the whole
    /// signature the scanner looks for.
    fn joined(parts: &[&str]) -> String {
        parts.concat()
    }

    fn sanitized(value: &str) -> String {
        redact(value)
    }

    #[test]
    fn redacts_arbitrary_password_punctuation_without_leaving_a_suffix() {
        let cases = vec![
            (
                assignment("password", "p@ssw0rd!very-secret"),
                assignment("password", "[REDACTED]"),
            ),
            (
                assignment("password", "abc&SUPERSECRET"),
                assignment("password", "[REDACTED]"),
            ),
            (
                assignment("password", "abc;SUPERSECRET"),
                assignment("password", "[REDACTED]"),
            ),
            (
                assignment("password", "abc?SUPERSECRET"),
                assignment("password", "[REDACTED]"),
            ),
            (
                assignment("password", "abc,SUPERSECRET"),
                assignment("password", "[REDACTED]"),
            ),
            (
                assignment("client_secret", "\"abc:def!ghi@example\""),
                assignment("client_secret", "\"[REDACTED]\""),
            ),
            (
                assignment("client_secret", "\"abc\\\"defVERYSECRET\""),
                assignment("client_secret", "\"[REDACTED]\""),
            ),
            (
                assignment("token", "'abc%123#xyz'"),
                assignment("token", "'[REDACTED]'"),
            ),
            (
                assignment("password", "p@ssw0rd!very-secret\nnext=line"),
                assignment("password", "[REDACTED]\nnext=line"),
            ),
        ];
        for (input, expected) in &cases {
            assert_eq!(sanitized(input), *expected, "Failed on input: {input}");
        }
        for (input, _) in &cases {
            let output = sanitized(input);
            for fragment in [
                "p@ssw0rd",
                "very-secret",
                "SUPERSECRET",
                "abc:def!ghi@example",
                "defVERYSECRET",
            ] {
                assert!(
                    !output.contains(fragment),
                    "credential fragment `{fragment}` survived: {output}"
                );
            }
        }
    }

    /// A truncated subprocess log can lose the closing quote. Once the opening
    /// quote is recognized, whitespace is part of the value, so redaction must
    /// run to the end of the line instead of stopping at the first space.
    #[test]
    fn unterminated_quoted_credentials_are_redacted_through_end_of_line() {
        let cases = vec![
            assignment("password", "\"abc defVERYSECRET"),
            assignment("token", "'abc defVERYSECRET"),
            // An escaped quote does not terminate the value.
            assignment("client_secret", "\"abc\\\"defVERYSECRET"),
            // A closing quote directly followed by a non-delimiter is not a
            // terminator either: the remainder is still the secret.
            assignment("client_secret", "\"abc\"defVERYSECRET"),
        ];
        for input in &cases {
            let output = sanitized(input);
            assert!(
                output.contains("[REDACTED]"),
                "value was not redacted: {output}"
            );
            assert!(
                !output.contains("VERYSECRET"),
                "secret suffix survived: {output}"
            );
            assert!(
                !output.contains("[REDACTED]VERYSECRET"),
                "redaction stopped before the value ended: {output}"
            );
        }

        // The truncation boundary is the line end, never the next line.
        let multiline = assignment("password", "\"abc defVERYSECRET\nnext=line");
        let output = sanitized(&multiline);
        assert!(!output.contains("VERYSECRET"), "{output}");
        assert!(
            output.contains("next=line"),
            "redaction crossed a line: {output}"
        );
    }

    /// A later quote must not end the redaction. Once a closing quote is seen
    /// followed by a non-delimiter, the value is invalid and everything to the
    /// end of the line belongs to the secret, however many quotes follow.
    #[test]
    fn malformed_quoted_credentials_never_expose_a_suffix() {
        let cases = vec![
            assignment("client_secret", "\"abc\"def\"ghiVERYSECRET"),
            assignment("client_secret", "\"abc\"def\"ghi\"VERYSECRET"),
            assignment("token", "'abc'def'ghiVERYSECRET"),
            assignment("client_secret", "\"abc\\\"defVERYSECRET"),
            assignment("password", "\"abc defVERYSECRET"),
        ];
        for input in &cases {
            let output = sanitized(input);
            assert!(
                !output.contains("VERYSECRET"),
                "secret suffix survived on `{input}`: {output}"
            );
            assert!(
                !output.contains("[REDACTED]ghi"),
                "redaction stopped at a later quote: {output}"
            );
            assert_eq!(
                sanitized(&output),
                output,
                "redaction must be idempotent: {output}"
            );
        }

        // Everything after the invalid close is removed, not just the first
        // character that made the close invalid.
        assert_eq!(
            sanitized(&assignment("client_secret", "\"abc\"def\"ghiVERYSECRET")),
            assignment("client_secret", "\"[REDACTED]")
        );
    }

    #[test]
    fn private_key_material_is_redacted_as_a_whole_block() {
        let block = format!(
            "{}\nSUPER_SECRET_BASE64_MATERIAL_LINE_ONE\nSUPER_SECRET_BASE64_MATERIAL_LINE_TWO\n{}",
            joined(&["-----BEGIN OPENSSH ", "PRIVATE KEY-----"]),
            joined(&["-----END OPENSSH ", "PRIVATE KEY-----"]),
        );
        let redacted = sanitized(&block);
        assert_eq!(redacted, "[REDACTED]");
        assert!(!redacted.contains("SUPER_SECRET_BASE64_MATERIAL"));
        assert!(contains_secret(&block));

        let pgp = format!(
            "{}\nSeCrEtBlOb\n{}",
            joined(&["-----BEGIN PGP ", "PRIVATE KEY BLOCK-----"]),
            joined(&["-----END PGP ", "PRIVATE KEY BLOCK-----"]),
        );
        assert_eq!(sanitized(&pgp), "[REDACTED]");
    }

    #[test]
    fn a_truncated_private_key_is_redacted_to_end_of_input() {
        // stderr is often truncated before sanitization, so an END marker may
        // never arrive; the body must still not survive.
        let truncated = format!(
            "{}\nSECRET_SECRET_SECRET\nmore-lines-that-must-not-survive",
            joined(&["-----BEGIN OPENSSH ", "PRIVATE KEY-----"]),
        );
        let redacted = sanitized(&truncated);
        assert_eq!(redacted, "[REDACTED]");
        assert!(!redacted.contains("SECRET_SECRET_SECRET"));

        let with_trailing = format!(
            "before\n{}\nBODY_SECRET\nafter-key-line",
            joined(&["-----BEGIN RSA ", "PRIVATE KEY-----"]),
        );
        let redacted = sanitized(&with_trailing);
        assert!(redacted.starts_with("before\n[REDACTED]"));
        assert!(!redacted.contains("BODY_SECRET"));
        assert!(!redacted.contains("after-key-line"));
    }

    #[test]
    fn redacts_full_values_containing_slash_plus_and_equals() {
        let refresh = assignment("refresh_token", "1//0gABCDEFGHIJKLMNOPQRSTUVWXYZ1234567890");
        assert_eq!(
            sanitized(&refresh),
            assignment("refresh_token", "[REDACTED]")
        );

        let base64 = assignment(
            "client_secret",
            "YWJjZGVmZ2hpamtsbW5vcC+/cXJzdHV2d3h5ejAxMjM0NTY=",
        );
        assert_eq!(
            sanitized(&base64),
            assignment("client_secret", "[REDACTED]")
        );

        let aws = assignment(
            "AWS_SECRET_ACCESS_KEY",
            &joined(&["wJalrXUtnFEMI", "/K7MDENG/", "bPxRfiCYEXAMPLEKEY"]),
        );
        assert_eq!(
            sanitized(&aws),
            assignment("AWS_SECRET_ACCESS_KEY", "[REDACTED]")
        );
    }

    #[test]
    fn basic_authorization_redacts_the_credential_not_the_scheme() {
        assert_eq!(
            sanitized(&format!("Authorization: Basic {}", "dXNlcjpwYXNz")),
            format!("Authorization: Basic {}", "[REDACTED]")
        );
        assert_eq!(
            sanitized(&assignment(
                "authorization",
                &format!("bearer {}", "abcdefghijklmnop")
            )),
            assignment("authorization", "bearer [REDACTED]")
        );
    }

    #[test]
    fn recognizes_modern_github_google_jwt_and_url_credentials() {
        let jwt = format!(
            "{}.{}.{}",
            "eyJhbGciOiJIUzI1NiJ9", "eyJzdWIiOiIxMjM0NTY3ODkwIn0", "abcdefghijklmnop"
        );
        let cases = vec![
            joined(&["github_pat_", "abcdefghijklmnopqrstuvwxyz0123456789ABCDEF"]),
            joined(&["AIzaSy", "abcdefghijklmnopqrstuvwxyz0123456"]),
            jwt,
            joined(&["https://user:", "password-value", "@example.com/path"]),
            joined(&["sk_live_", "abcdefghijklmnopqrstuvwx"]),
            joined(&["xai-", "abcdefghijklmnopqrstuvwx0123456789ABCD"]),
            joined(&["fw_", "abcdefghijklmnopqrstuvwx0123"]),
        ];
        for case in &cases {
            assert_ne!(sanitized(case), *case, "not redacted: {case}");
        }
    }

    #[test]
    fn normal_quoted_credentials_are_still_redacted_exactly() {
        // Regression guard for the unterminated-quote fallback: a complete
        // quoted value keeps its closing quote and loses only the value.
        let complete = assignment("password", "\"supersecretvalue\"");
        assert_eq!(
            sanitized(&complete),
            assignment("password", "\"[REDACTED]\"")
        );

        let json = format!(
            "{{{}, {}}}",
            json_pair("token", "dummy-token"),
            json_pair("password", "dummy-password")
        );
        assert_eq!(
            sanitized(&json),
            format!(
                "{{{}, {}}}",
                json_pair("token", "[REDACTED]"),
                json_pair("password", "[REDACTED]")
            )
        );
    }

    /// The scanner's detection half is narrower than the sanitizer's: it must
    /// report real exposure shapes and stay quiet on ordinary source.
    #[test]
    fn the_scanner_reports_credentials_and_ignores_identifiers() {
        assert!(contains_secret(&assignment(
            "password",
            "\"abc defVERYSECRET"
        )));
        assert!(contains_secret(&assignment("token", "'abc defVERYSECRET")));
        assert!(contains_secret(&assignment(
            "password",
            "\"supersecretvalue\""
        )));
        // A generated value in source: mixed case plus a digit.
        assert!(contains_secret(&assignment("password", "Abc123XyZ456")));
        // `platform_token2` is an identifier, not a generated key.
        assert!(!contains_secret(&assignment("token", "platform_token2")));
        assert!(!contains_secret("let token = platform_token(platform);"));
        assert!(!contains_secret(&assignment("secret", "SecretString")));
        assert_eq!(
            match_category(&assignment("password", "\"abc defVERYSECRET")),
            Some("Credential assignment")
        );
    }

    /// A credential-bearing file is scanned for values of any shape, because a
    /// missed password costs more there than a finding for a literal one.
    #[test]
    fn credential_files_report_assigned_values_source_would_not() {
        let punctuation = assignment("password", "p@ssw0rd!very-secret");
        assert!(
            !contains_secret(&punctuation),
            "source must not read an expression as a credential"
        );
        assert!(contains_secret_in(
            &punctuation,
            ScanContext::CredentialFile
        ));

        let identifier = assignment("token", "platform_token2");
        assert!(!contains_secret(&identifier));
        assert!(contains_secret_in(&identifier, ScanContext::CredentialFile));

        let short_quoted = assignment("password", "\"short\"");
        assert!(!contains_secret(&short_quoted));
        assert!(contains_secret_in(
            &short_quoted,
            ScanContext::CredentialFile
        ));

        let lowercase = assignment("api_key", "abcdef123456");
        assert!(!contains_secret(&lowercase));
        assert!(contains_secret_in(&lowercase, ScanContext::CredentialFile));

        // Provider shapes are reported in either context.
        let provider = joined(&["github_pat_", "abcdefghijklmnopqrstuvwxyz0123456789ABCDEF"]);
        assert!(contains_secret(&provider));
        assert!(contains_secret_in(&provider, ScanContext::CredentialFile));
        assert_eq!(
            match_category_in(&punctuation, ScanContext::CredentialFile),
            Some("Credential assignment")
        );
    }

    #[test]
    fn every_registry_credential_format_is_covered() {
        for descriptor in ProviderRegistry::static_all() {
            if !matches!(
                descriptor.credential_kind,
                CredentialKind::ApiKey | CredentialKind::OAuth
            ) {
                continue;
            }
            let samples = provider_credential_samples();
            let sample = samples
                .iter()
                .find(|(provider, _)| *provider == descriptor.id)
                .map(|(_, sample)| sample.as_str())
                .unwrap_or_else(|| {
                    panic!(
                        "credential-bearing provider {} has no pattern sample",
                        descriptor.display_name
                    )
                });
            assert!(
                contains_secret(sample),
                "no shared pattern matches the {} credential format",
                descriptor.display_name
            );
        }
    }

    #[test]
    fn every_sample_is_redacted_and_never_partially_survives() {
        for (_, sample) in provider_credential_samples() {
            let redacted = sanitized(&sample);
            assert!(
                redacted.contains("[REDACTED]"),
                "sample was not redacted: {sample}"
            );
            assert!(
                !redacted.contains("abcdefghijklmnopqrstuvwxyz"),
                "redaction left a credential suffix: {redacted}"
            );
        }
    }
}
