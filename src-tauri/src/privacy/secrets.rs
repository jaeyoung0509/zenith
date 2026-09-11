//! Single source of truth for credential-shaped strings.
//!
//! The diagnostics sanitizer and the AI Control Center secret scanner both read
//! this table so detection coverage cannot diverge between the two. Adding a
//! credential-bearing provider means adding a sample in
//! [`provider_credential_samples`]; the coverage test fails otherwise.

use regex::Regex;
use std::sync::LazyLock;

/// A credential pattern. `replacement` keeps any capture groups that must
/// survive redaction (for example the assignment prefix or an authorization
/// scheme) and replaces everything else with `[REDACTED]`.
pub struct SecretPattern {
    pub category: &'static str,
    pub regex: Regex,
    pub replacement: &'static str,
}

macro_rules! pattern {
    ($category:expr, $regex:expr, $replacement:expr) => {
        SecretPattern {
            category: $category,
            regex: Regex::new($regex).expect("valid secret pattern"),
            replacement: $replacement,
        }
    };
}

static PATTERNS: LazyLock<Vec<SecretPattern>> = LazyLock::new(|| {
    vec![
        pattern!(
            "Private key material",
            r"-----BEGIN (?:RSA |EC |DSA |OPENSSH |PGP )?PRIVATE KEY-----",
            "[REDACTED]"
        ),
        pattern!(
            "GitHub token",
            r"\bgithub_pat_[A-Za-z0-9_]{20,}\b",
            "[REDACTED]"
        ),
        pattern!("GitHub token", r"\bgh[pousr]_[A-Za-z0-9]{20,}\b", "[REDACTED]"),
        pattern!(
            "GitLab token",
            r"\bglpat-[A-Za-z0-9_\-]{20,}\b",
            "[REDACTED]"
        ),
        pattern!("Slack token", r"\bxox[baprs]-[A-Za-z0-9-]{10,}\b", "[REDACTED]"),
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
        pattern!("Fireworks API key", r"\bfw_[A-Za-z0-9]{16,}\b", "[REDACTED]"),
        pattern!("Hugging Face token", r"\bhf_[A-Za-z0-9]{20,}\b", "[REDACTED]"),
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
        pattern!(
            "Authorization header",
            r#"(?i)(authorization\s*[:=]\s*)([A-Za-z][A-Za-z0-9._-]*\s+)?([^\s,;"']{4,})"#,
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
        pattern!(
            "Credential assignment",
            r#"(?i)(["']?(?:api[_-]?key|access[_-]?key|secret[_-]?access[_-]?key|client[_-]?secret|private[_-]?key|auth[_-]?token|access[_-]?token|refresh[_-]?token|id[_-]?token|token|secret|password|passwd|pwd)["']?\s*[:=]\s*["']?)([A-Za-z0-9_\-\.=+/]{6,})("?)"#,
            "${1}[REDACTED]${3}"
        ),
    ]
});

pub fn patterns() -> &'static [SecretPattern] {
    &PATTERNS
}

/// Replaces every credential-shaped substring with `[REDACTED]`.
pub fn redact(text: &str) -> String {
    let mut result = text.to_string();
    for pattern in patterns() {
        result = pattern
            .regex
            .replace_all(&result, pattern.replacement)
            .into_owned();
    }
    result
}

/// Whether the text contains any credential shape. Used by the scanner, which
/// intentionally never extracts or returns the matched value.
pub fn contains_secret(text: &str) -> bool {
    patterns().iter().any(|pattern| pattern.regex.is_match(text))
}

/// Returns the category of the first matching credential shape without
/// extracting the value.
pub fn match_category(text: &str) -> Option<&'static str> {
    patterns()
        .iter()
        .find(|pattern| pattern.regex.is_match(text))
        .map(|pattern| pattern.category)
}

/// Representative credential strings for every provider that stores a secret.
/// A provider added without a sample fails
/// `every_registry_credential_format_is_covered`.
pub fn provider_credential_samples() -> &'static [(crate::models::ProviderId, &'static str)] {
    use crate::models::ProviderId;
    &[
        (
            ProviderId::Codex,
            "eyJhbGciOiJSUzI1NiIsImtpZCI6ImFiYyJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV_adQssw5c",
        ),
        (
            ProviderId::OpenRouter,
            "sk-or-v1-abcdefghijklmnopqrstuvwxyz0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ01",
        ),
        (
            ProviderId::Antigravity,
            "eyJhbGciOiJFUzI1NiIsImtpZCI6InRlc3QifQ.eyJzdWIiOiIxMjM0NTY3ODkwIn0.c2lnbmF0dXJlLXNlZ21lbnQ",
        ),
        (ProviderId::XaiApi, "xai-abcdefghijklmnopqrstuvwx0123456789ABCD"),
        (
            ProviderId::OpenAiApi,
            "sk-proj-abcdefghijklmnopqrstuvwxyz0123456789ABCDEFGH",
        ),
        (
            ProviderId::AnthropicApi,
            "sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123456789ABCDEFGH",
        ),
        (ProviderId::MetaModelApi, "META_API_KEY=abcdefghijklmnopqrstuvwxyz0123456789ABCD"),
        (
            ProviderId::MistralApi,
            "MISTRAL_API_KEY=abcdefghijklmnopqrstuvwxyz0123456789ABCD",
        ),
        (ProviderId::FireworksApi, "fw_abcdefghijklmnopqrstuvwx0123"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai_providers::registry::{CredentialKind, ProviderRegistry};

    #[test]
    fn redacts_full_values_containing_slash_plus_and_equals() {
        let refresh = "refresh_token=1//0gABCDEFGHIJKLMNOPQRSTUVWXYZ1234567890";
        assert_eq!(sanitized(refresh), "refresh_token=[REDACTED]");

        let base64 = "client_secret=YWJjZGVmZ2hpamtsbW5vcC+/cXJzdHV2d3h5ejAxMjM0NTY=";
        assert_eq!(sanitized(base64), "client_secret=[REDACTED]");

        let aws = "AWS_SECRET_ACCESS_KEY=wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY";
        assert_eq!(sanitized(aws), "AWS_SECRET_ACCESS_KEY=[REDACTED]");
    }

    #[test]
    fn basic_authorization_redacts_the_credential_not_the_scheme() {
        assert_eq!(
            sanitized("Authorization: Basic dXNlcjpwYXNz"),
            "Authorization: Basic [REDACTED]"
        );
        assert_eq!(
            sanitized("authorization=bearer abcdefghijklmnop"),
            "authorization=bearer [REDACTED]"
        );
    }

    #[test]
    fn recognizes_modern_github_google_jwt_and_url_credentials() {
        let stripe_sample = format!("sk_live_{}", "abcdefghijklmnopqrstuvwx");
        let cases = [
            "github_pat_abcdefghijklmnopqrstuvwxyz0123456789ABCDEF",
            "AIzaSyabcdefghijklmnopqrstuvwxyz0123456",
            "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.abcdefghijklmnop",
            "https://user:password-value@example.com/path",
            stripe_sample.as_str(),
            "xai-abcdefghijklmnopqrstuvwx0123456789ABCD",
            "fw_abcdefghijklmnopqrstuvwx0123",
        ];
        for case in cases {
            assert_ne!(sanitized(case), case, "not redacted: {case}");
        }
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
            let sample = provider_credential_samples()
                .iter()
                .find(|(provider, _)| *provider == descriptor.id)
                .map(|(_, sample)| *sample)
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
            let redacted = sanitized(sample);
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

    fn sanitized(value: &str) -> String {
        redact(value)
    }
}
