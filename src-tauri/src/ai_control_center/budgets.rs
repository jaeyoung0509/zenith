use crate::models::*;
use std::collections::HashSet;

pub fn sanitize(mut preferences: AiControlPreferences) -> AiControlPreferences {
    let mut ids = HashSet::new();
    preferences.budgets.retain_mut(|budget| {
        budget.limit.micros = budget.limit.micros.clamp(0, 1_000_000_000_000_000);
        budget.limit.currency = "USD".into();
        budget
            .threshold_percents
            .retain(|value| (1..=100).contains(value));
        budget.threshold_percents.sort_unstable();
        budget.threshold_percents.dedup();
        if budget.threshold_percents.is_empty() {
            budget.threshold_percents = vec![50, 80, 100];
        }
        !budget.id.trim().is_empty() && ids.insert(budget.id.clone())
    });
    if preferences.budgets.is_empty() {
        preferences.budgets.push(LocalAlertBudget::default());
    }
    preferences.manual_usage.retain_mut(|manual| {
        manual.spent.micros = manual.spent.micros.max(0);
        manual.spent.currency = "USD".into();
        if let Some(limit) = &mut manual.limit {
            limit.micros = limit.micros.max(0);
            limit.currency = "USD".into();
        }
        !manual.provider_id.trim().is_empty()
    });
    preferences.autopilot.recommendation_cooldown_seconds = preferences
        .autopilot
        .recommendation_cooldown_seconds
        .clamp(60, 86_400);
    preferences.audit_retention_days = preferences.audit_retention_days.clamp(1, 365);
    preferences.dismissed_findings.sort();
    preferences.dismissed_findings.dedup();
    preferences.dismissed_findings.truncate(512);
    preferences
}

pub fn statuses(
    budgets: &[LocalAlertBudget],
    observations: &[ProviderObservation],
) -> Vec<BudgetStatus> {
    budgets
        .iter()
        .filter(|budget| budget.enabled)
        .map(|budget| {
            let matching = observations
                .iter()
                .filter(|observation| {
                    budget
                        .provider_id
                        .as_ref()
                        .is_none_or(|id| id == &observation.provider_id)
                })
                .collect::<Vec<_>>();
            let spent_micros = matching
                .iter()
                .flat_map(|observation| observation.metrics.iter())
                .filter_map(|metric| metric.cost.as_ref())
                .filter(|money| money.currency == "USD")
                .map(|money| money.micros)
                .fold(0i64, i64::saturating_add);
            let kinds = matching
                .iter()
                .filter(|observation| {
                    observation
                        .metrics
                        .iter()
                        .any(|metric| metric.cost.is_some())
                })
                .map(|observation| observation.source_kind)
                .collect::<HashSet<_>>();
            let spent = MoneyMicros::usd(spent_micros);
            let used_basis_points = spent.percent_of(&budget.limit).unwrap_or(0);
            BudgetStatus {
                budget_id: budget.id.clone(),
                spent,
                limit: budget.limit.clone(),
                used_basis_points,
                crossed_thresholds: budget
                    .threshold_percents
                    .iter()
                    .copied()
                    .filter(|threshold| used_basis_points >= u16::from(*threshold) * 100)
                    .collect(),
                source_label: "Zenith alert budget".into(),
                mixed_sources: kinds.len() > 1,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn threshold_boundaries_and_mixed_sources_are_exact() {
        let budget = LocalAlertBudget {
            enabled: true,
            ..LocalAlertBudget::default()
        };
        let make = |source_kind, micros| ProviderObservation {
            provider_id: "x".into(),
            display_name: "x".into(),
            source_kind,
            source_id: "x".into(),
            scope: ObservationScope::ApiKey,
            observed_at: 1,
            period: ObservationPeriod {
                starts_at: None,
                ends_at: None,
                resets_at: None,
                label: "x".into(),
            },
            fresh_for_seconds: 1,
            quality: ObservationQuality::Fresh,
            installed: true,
            connected: true,
            status_message: "x".into(),
            metrics: vec![ProviderMetric {
                label: "x".into(),
                tokens: None,
                cost: Some(MoneyMicros::usd(micros)),
                used_basis_points: None,
            }],
            action_url: None,
            partial_error: None,
        };
        let result = statuses(
            &[budget],
            &[
                make(ObservationSourceKind::LiveAuthoritative, 25_000_000),
                make(ObservationSourceKind::Manual, 15_000_000),
            ],
        );
        assert_eq!(result[0].used_basis_points, 8_000);
        assert_eq!(result[0].crossed_thresholds, vec![50, 80]);
        assert!(result[0].mixed_sources);
    }
}
