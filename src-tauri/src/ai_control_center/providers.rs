use crate::models::*;
use std::collections::HashMap;

pub trait ProviderAdapter: Send + Sync {
    fn collect(&self, now: u64) -> ProviderObservation;
}

pub fn collect_adapters(
    adapters: &[Box<dyn ProviderAdapter>],
    now: u64,
) -> Vec<ProviderObservation> {
    adapters
        .iter()
        .map(|adapter| adapter.collect(now))
        .collect()
}

pub fn retain_last_success(
    current: Vec<ProviderObservation>,
    last: &mut HashMap<String, ProviderObservation>,
) -> Vec<ProviderObservation> {
    current
        .into_iter()
        .map(|observation| {
            let successful = !matches!(observation.quality, ObservationQuality::Unavailable)
                && observation.partial_error.is_none();
            if successful {
                last.insert(observation.source_id.clone(), observation.clone());
                return observation;
            }
            if let Some(previous) = last.get(&observation.source_id) {
                let mut stale = previous.clone();
                stale.quality = ObservationQuality::Stale;
                stale.partial_error = observation
                    .partial_error
                    .clone()
                    .or_else(|| Some(observation.status_message.clone()));
                stale.status_message =
                    "Last successful observation retained after refresh failure.".into();
                stale
            } else {
                observation
            }
        })
        .collect()
}

pub fn normalize(
    existing: &AiUsageSnapshot,
    preferences: &AiControlPreferences,
) -> Vec<ProviderObservation> {
    let mut observations = existing
        .providers
        .iter()
        .map(|provider| from_legacy(provider, existing.fetched_at))
        .collect::<Vec<_>>();
    for manual in &preferences.manual_usage {
        let display_name = observations
            .iter()
            .find(|item| item.provider_id == manual.provider_id)
            .map(|item| item.display_name.clone())
            .unwrap_or_else(|| manual.provider_id.clone());
        observations.push(ProviderObservation {
            provider_id: manual.provider_id.clone(),
            display_name: format!("{display_name} · Manual"),
            source_kind: ObservationSourceKind::Manual,
            source_id: format!("manual.{}", manual.provider_id),
            scope: manual_scope(&manual.provider_id),
            observed_at: manual.entered_at,
            period: ObservationPeriod {
                starts_at: None,
                ends_at: None,
                resets_at: manual.resets_at,
                label: "User-entered period".into(),
            },
            fresh_for_seconds: u64::MAX,
            quality: ObservationQuality::Fresh,
            installed: true,
            connected: true,
            status_message: "Manual value entered in Zenith; not a provider-enforced limit.".into(),
            metrics: vec![ProviderMetric {
                label: "User-entered spend".into(),
                tokens: None,
                cost: Some(manual.spent.clone()),
                used_basis_points: manual
                    .limit
                    .as_ref()
                    .and_then(|limit| manual.spent.percent_of(limit)),
            }],
            action_url: None,
            partial_error: None,
        });
    }
    observations.extend(optional_organization_rows(existing.fetched_at));
    observations
}

fn from_legacy(provider: &AiProviderUsage, observed_at: u64) -> ProviderObservation {
    let (source_kind, scope) = match provider.id.as_str() {
        "codex" => (
            ObservationSourceKind::LiveQuota,
            ObservationScope::Subscription,
        ),
        "openrouter" => (
            ObservationSourceKind::LiveAuthoritative,
            ObservationScope::ApiKey,
        ),
        "opencode" => (
            ObservationSourceKind::LocalEstimate,
            ObservationScope::LocalSessions,
        ),
        _ => (
            ObservationSourceKind::Manual,
            ObservationScope::Subscription,
        ),
    };
    let mut metrics = provider
        .windows
        .iter()
        .map(|window| ProviderMetric {
            label: window.label.clone(),
            tokens: None,
            cost: None,
            used_basis_points: Some((window.used_percent.clamp(0.0, 100.0) * 100.0).round() as u16),
        })
        .collect::<Vec<_>>();
    if let Some(tokens) = provider.summary.last_7d_tokens {
        metrics.push(ProviderMetric {
            label: "Last 7 days".into(),
            tokens: Some(tokens),
            cost: None,
            used_basis_points: None,
        });
    }
    if let Some(cost) = provider
        .summary
        .local_cost_usd
        .and_then(usd_float_to_micros)
    {
        metrics.push(ProviderMetric {
            label: "Local estimated cost".into(),
            tokens: None,
            cost: Some(MoneyMicros::usd(cost)),
            used_basis_points: None,
        });
    }
    if let Some(cost) = provider.summary.usage_usd.and_then(usd_float_to_micros) {
        metrics.push(ProviderMetric {
            label: "Key usage".into(),
            tokens: None,
            cost: Some(MoneyMicros::usd(cost)),
            used_basis_points: None,
        });
    }
    let resets_at = provider
        .windows
        .iter()
        .filter_map(|window| window.resets_at)
        .min();
    let status_message = match provider.id.as_str() {
        "antigravity" => "Manual/external subscription usage. Antigravity is Google's primary individual coding CLI; no documented structured usage API was detected.".into(),
        "claude" => "Manual/external subscription usage. Use Claude Code /usage; Zenith does not scrape the TUI or credentials.".into(),
        _ => provider.status_message.clone(),
    };
    let has_measurement = !metrics.is_empty();
    let unavailable = match source_kind {
        ObservationSourceKind::LiveAuthoritative | ObservationSourceKind::LiveQuota => {
            !provider.connected
        }
        ObservationSourceKind::LocalEstimate => !has_measurement,
        // Capability/install detection is not a manual observation. Only an explicit
        // user entry added by `normalize` below receives a fresh manual timestamp.
        ObservationSourceKind::Manual => true,
    };
    ProviderObservation {
        provider_id: provider.id.clone(),
        display_name: provider.name.clone(),
        source_kind,
        source_id: format!(
            "{}.{}",
            provider.id,
            match source_kind {
                ObservationSourceKind::LiveQuota => "app_server",
                ObservationSourceKind::LiveAuthoritative => "official_api",
                ObservationSourceKind::LocalEstimate => "official_cli",
                ObservationSourceKind::Manual => "external",
            }
        ),
        scope,
        observed_at,
        period: ObservationPeriod {
            starts_at: None,
            ends_at: None,
            resets_at,
            label: if resets_at.is_some() {
                "Provider reset window".into()
            } else {
                "Current observation".into()
            },
        },
        fresh_for_seconds: 60,
        quality: if unavailable {
            ObservationQuality::Unavailable
        } else {
            ObservationQuality::Fresh
        },
        installed: provider.installed,
        connected: provider.connected,
        status_message,
        metrics,
        action_url: provider.action_url.clone(),
        partial_error: (unavailable && source_kind != ObservationSourceKind::Manual)
            .then(|| provider.status_message.clone()),
    }
}

fn optional_organization_rows(observed_at: u64) -> Vec<ProviderObservation> {
    [
        ("openai-api", "OpenAI API organization", ObservationScope::Organization, "Separate from Codex subscription; optional managed credentials must use Keychain."),
        ("anthropic-api", "Anthropic organization API", ObservationScope::Organization, "Optional organization adapter; individual Claude subscriptions remain manual."),
        ("cursor-org", "Cursor Teams / Enterprise", ObservationScope::Organization, "Optional admin adapter; individual Cursor usage remains external."),
        ("xai-api", "xAI API team", ObservationScope::Organization, "Separate from Grok Build subscription usage."),
        ("gemini-enterprise", "Gemini Code Assist Standard / Enterprise", ObservationScope::Organization, "Enterprise/API usage remains supported; consumer individual access moved to Antigravity."),
        ("grok-individual", "Grok Build subscription", ObservationScope::Subscription, "Manual/external; no documented subscription usage endpoint is available."),
        ("cursor-individual", "Cursor individual", ObservationScope::Subscription, "Manual/external; Zenith does not inspect private editor state."),
        ("claude-individual", "Claude individual", ObservationScope::Subscription, "Manual/external; use Claude Code /usage without scraping credentials or the TUI."),
    ].into_iter().map(|(id, name, scope, message)| ProviderObservation {
        provider_id: id.into(), display_name: name.into(), source_kind: ObservationSourceKind::Manual,
        source_id: format!("{id}.capability"), scope, observed_at,
        period: ObservationPeriod { starts_at: None, ends_at: None, resets_at: None, label: "Unavailable".into() },
        fresh_for_seconds: 300, quality: ObservationQuality::Unavailable, installed: false, connected: false,
        status_message: message.into(), metrics: vec![], action_url: None, partial_error: None,
    }).collect()
}

fn manual_scope(provider_id: &str) -> ObservationScope {
    if provider_id.ends_with("-api") {
        ObservationScope::ApiKey
    } else if provider_id.ends_with("-enterprise") || provider_id.ends_with("-org") {
        ObservationScope::Organization
    } else {
        ObservationScope::Subscription
    }
}

fn usd_float_to_micros(value: f64) -> Option<i64> {
    if !value.is_finite() || value < 0.0 || value > i64::MAX as f64 / 1_000_000.0 {
        return None;
    }
    Some((value * 1_000_000.0).round() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn separates_codex_subscription_from_openai_api() {
        let existing = AiUsageSnapshot {
            fetched_at: 10,
            providers: vec![AiProviderUsage {
                id: "codex".into(),
                name: "Codex".into(),
                installed: true,
                connected: true,
                auth_label: "OAuth".into(),
                status_message: "ok".into(),
                support: UsageSupport::Live,
                windows: vec![],
                summary: UsageSummary::default(),
                action_url: None,
            }],
        };
        let rows = normalize(&existing, &AiControlPreferences::default());
        let codex = rows.iter().find(|row| row.provider_id == "codex").unwrap();
        let api = rows
            .iter()
            .find(|row| row.provider_id == "openai-api")
            .unwrap();
        assert_eq!(codex.scope, ObservationScope::Subscription);
        assert_eq!(api.scope, ObservationScope::Organization);
        assert_eq!(api.quality, ObservationQuality::Unavailable);
    }

    #[test]
    fn separates_grok_build_and_xai_api() {
        let existing = AiUsageSnapshot {
            fetched_at: 10,
            providers: vec![],
        };
        let rows = normalize(&existing, &AiControlPreferences::default());
        let grok = rows
            .iter()
            .find(|row| row.provider_id == "grok-individual")
            .unwrap();
        let api = rows
            .iter()
            .find(|row| row.provider_id == "xai-api")
            .unwrap();
        assert_eq!(grok.scope, ObservationScope::Subscription);
        assert_eq!(api.scope, ObservationScope::Organization);
        assert_ne!(grok.source_id, api.source_id);
    }

    #[test]
    fn separates_antigravity_consumer_from_gemini_enterprise() {
        let existing = AiUsageSnapshot {
            fetched_at: 10,
            providers: vec![AiProviderUsage {
                id: "antigravity".into(),
                name: "Antigravity".into(),
                installed: true,
                connected: false,
                auth_label: "Google OAuth".into(),
                status_message: "Google".into(),
                support: UsageSupport::Manual,
                windows: vec![],
                summary: UsageSummary::default(),
                action_url: None,
            }],
        };
        let rows = normalize(&existing, &AiControlPreferences::default());
        let antigravity = rows
            .iter()
            .find(|row| row.provider_id == "antigravity")
            .unwrap();
        let gemini = rows
            .iter()
            .find(|row| row.provider_id == "gemini-enterprise")
            .unwrap();
        assert_eq!(antigravity.scope, ObservationScope::Subscription);
        assert_eq!(gemini.scope, ObservationScope::Organization);
        assert!(antigravity.status_message.contains("Antigravity"));
        assert!(gemini.status_message.contains("Enterprise"));
    }

    #[test]
    fn rejects_non_finite_money() {
        assert_eq!(usd_float_to_micros(f64::NAN), None);
        assert_eq!(usd_float_to_micros(1.234567), Some(1_234_567));
    }

    #[test]
    fn external_capabilities_are_unavailable_until_the_user_enters_a_value() {
        let existing = AiUsageSnapshot {
            fetched_at: 10,
            providers: vec![AiProviderUsage {
                id: "claude".into(),
                name: "Claude Code".into(),
                installed: true,
                connected: false,
                auth_label: "Claude.ai OAuth".into(),
                status_message: "Use /usage".into(),
                support: UsageSupport::Manual,
                windows: vec![],
                summary: UsageSummary::default(),
                action_url: None,
            }],
        };
        let rows = normalize(&existing, &AiControlPreferences::default());
        let claude = rows.iter().find(|row| row.provider_id == "claude").unwrap();
        assert_eq!(claude.source_kind, ObservationSourceKind::Manual);
        assert_eq!(claude.quality, ObservationQuality::Unavailable);
        assert_eq!(claude.partial_error, None);

        let preferences = AiControlPreferences {
            manual_usage: vec![ManualProviderUsage {
                provider_id: "claude".into(),
                spent: MoneyMicros::usd(1_000_000),
                entered_at: 9,
                ..Default::default()
            }],
            ..Default::default()
        };
        let rows = normalize(&existing, &preferences);
        let manual = rows
            .iter()
            .find(|row| row.source_id == "manual.claude")
            .unwrap();
        assert_eq!(manual.quality, ObservationQuality::Fresh);
        assert_eq!(manual.observed_at, 9);
    }

    struct FakeAdapter(ObservationQuality, ObservationSourceKind);
    impl ProviderAdapter for FakeAdapter {
        fn collect(&self, now: u64) -> ProviderObservation {
            ProviderObservation {
                provider_id: "fake".into(),
                display_name: "Fake".into(),
                source_kind: self.1,
                source_id: format!("fake.{:?}", self.1),
                scope: ObservationScope::Project,
                observed_at: now,
                period: ObservationPeriod {
                    starts_at: None,
                    ends_at: None,
                    resets_at: None,
                    label: "Test".into(),
                },
                fresh_for_seconds: 60,
                quality: self.0,
                installed: true,
                connected: true,
                status_message: "fake".into(),
                metrics: vec![],
                action_url: None,
                partial_error: (self.0 == ObservationQuality::Partial).then(|| "partial".into()),
            }
        }
    }

    #[test]
    fn adapter_contract_preserves_live_local_manual_unavailable_and_partial_provenance() {
        let adapters: Vec<Box<dyn ProviderAdapter>> = vec![
            Box::new(FakeAdapter(
                ObservationQuality::Fresh,
                ObservationSourceKind::LiveAuthoritative,
            )),
            Box::new(FakeAdapter(
                ObservationQuality::Fresh,
                ObservationSourceKind::LocalEstimate,
            )),
            Box::new(FakeAdapter(
                ObservationQuality::Fresh,
                ObservationSourceKind::Manual,
            )),
            Box::new(FakeAdapter(
                ObservationQuality::Unavailable,
                ObservationSourceKind::LiveQuota,
            )),
            Box::new(FakeAdapter(
                ObservationQuality::Partial,
                ObservationSourceKind::LiveAuthoritative,
            )),
        ];
        let rows = collect_adapters(&adapters, 42);
        assert_eq!(rows.len(), 5);
        assert!(rows.iter().all(|row| row.observed_at == 42));
        assert!(rows
            .iter()
            .any(|row| row.quality == ObservationQuality::Unavailable));
        assert!(rows
            .iter()
            .any(|row| row.quality == ObservationQuality::Partial));
    }

    #[test]
    fn last_success_becomes_stale_after_adapter_failure() {
        let mut cache = HashMap::new();
        let good = FakeAdapter(
            ObservationQuality::Fresh,
            ObservationSourceKind::LiveAuthoritative,
        )
        .collect(1);
        retain_last_success(vec![good], &mut cache);
        let failed = FakeAdapter(
            ObservationQuality::Unavailable,
            ObservationSourceKind::LiveAuthoritative,
        )
        .collect(2);
        let rows = retain_last_success(vec![failed], &mut cache);
        assert_eq!(rows[0].quality, ObservationQuality::Stale);
        assert!(rows[0].partial_error.is_some());
    }
}
