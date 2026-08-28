use crate::models::*;
use sha2::{Digest, Sha256};
use std::collections::HashMap;

#[derive(Debug, Default)]
pub struct PolicyEngine {
    previous_sessions: HashMap<String, Option<String>>,
    last_emitted: HashMap<String, u64>,
}

impl PolicyEngine {
    pub fn evaluate(
        &mut self,
        resources: &[ResourceAttribution],
        memory: Option<MemoryPressure>,
        power: PowerSourceType,
        preferences: &AutopilotPreferences,
        now: u64,
    ) -> Vec<Recommendation> {
        let current = resources
            .iter()
            .map(|resource| (resource.session_id.clone(), resource.project_id.clone()))
            .collect::<HashMap<_, _>>();
        let mut candidates = Vec::new();
        if preferences.notify_on_battery
            && power == PowerSourceType::Battery
            && !resources.is_empty()
        {
            candidates.push(candidate(
                RecommendationKind::Battery,
                "Agent moved to battery",
                "Review power-intensive sessions or connect AC power.",
                None,
                None,
                "Open Projects",
            ));
        }
        if preferences.notify_on_memory_pressure
            && matches!(
                memory,
                Some(MemoryPressure::Warning | MemoryPressure::Critical)
            )
        {
            candidates.push(candidate(RecommendationKind::Memory,"Memory pressure needs review","Review attributed sessions before choosing an existing process or cleanup workflow.",None,None,"Open Memory"));
        }
        if preferences.notify_on_session_completion {
            for (session_id, project_id) in self
                .previous_sessions
                .iter()
                .filter(|(id, _)| !current.contains_key(*id))
            {
                candidates.push(candidate(
                    RecommendationKind::SessionCompleted,
                    "Verified session exited",
                    "Review remaining listeners and developer artifacts before taking action.",
                    Some(session_id.clone()),
                    project_id.clone(),
                    "Review project",
                ));
                candidates.push(candidate(RecommendationKind::CleanupReview,"Generated artifacts may remain","Open the existing Developer Artifact Review workflow for an explicit scan and preview.",Some(session_id.clone()),project_id.clone(),"Open Developer Artifacts"));
            }
        }
        for resource in resources
            .iter()
            .filter(|resource| resource.project_id.is_none())
        {
            candidates.push(candidate(
                RecommendationKind::OrphanProcess,
                "Unassigned agent process",
                "The process is recognized, but Zenith cannot prove a project identity. Review it in Projects; no mutable action is authorized.",
                Some(resource.session_id.clone()),
                None,
                "Open Projects",
            ));
        }
        for resource in resources
            .iter()
            .filter(|resource| resource.open_dev_ports > 0)
        {
            candidates.push(candidate(RecommendationKind::DevelopmentPort,"Development listeners are still active",&format!("{} verified listener(s) are associated with this project. Review them before release.",resource.open_dev_ports),Some(resource.session_id.clone()),resource.project_id.clone(),"Open Development Servers"));
        }
        self.previous_sessions = current;
        let cooldown = preferences.recommendation_cooldown_seconds.max(60);
        let mut output = Vec::new();
        for item in candidates {
            let key = format!(
                "{:?}:{}",
                item.kind,
                item.project_id.as_deref().unwrap_or("unassigned")
            );
            if self
                .last_emitted
                .get(&key)
                .is_some_and(|last| now.saturating_sub(*last) < cooldown)
            {
                continue;
            }
            self.last_emitted.insert(key, now);
            output.push(materialize(item, now, cooldown));
        }
        output
    }
}
struct Candidate {
    kind: RecommendationKind,
    title: String,
    message: String,
    session_id: Option<String>,
    project_id: Option<String>,
    action_label: Option<String>,
}
fn candidate(
    kind: RecommendationKind,
    title: &str,
    message: &str,
    session_id: Option<String>,
    project_id: Option<String>,
    action: &str,
) -> Candidate {
    Candidate {
        kind,
        title: title.into(),
        message: message.into(),
        session_id,
        project_id,
        action_label: Some(action.into()),
    }
}
fn materialize(value: Candidate, now: u64, cooldown: u64) -> Recommendation {
    let mut hash = Sha256::new();
    hash.update(format!("{:?}", value.kind));
    hash.update(value.project_id.as_deref().unwrap_or("none"));
    hash.update(now.to_le_bytes());
    let id = format!("recommendation-{}", &format!("{:x}", hash.finalize())[..16]);
    Recommendation {
        id,
        kind: value.kind,
        title: value.title,
        message: value.message,
        created_at: now,
        cooldown_until: now.saturating_add(cooldown),
        session_id: value.session_id,
        project_id: value.project_id,
        action_label: value.action_label,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn resource() -> ResourceAttribution {
        ResourceAttribution {
            session_id: "s".into(),
            project_id: Some("p".into()),
            tool_name: "Codex".into(),
            cpu_percent: 1.0,
            memory_bytes: 1,
            process_count: 1,
            duration_seconds: 1,
            open_dev_ports: 0,
            power_eligible: true,
            confidence: "process_observed".into(),
            reason: "x".into(),
            mutable_actions_allowed: false,
        }
    }
    #[test]
    fn unassigned_process_is_advisory_and_never_authorizes_mutation() {
        let mut orphan = resource();
        orphan.project_id = None;
        orphan.mutable_actions_allowed = false;
        let rows = PolicyEngine::default().evaluate(
            &[orphan],
            None,
            PowerSourceType::Ac,
            &AutopilotPreferences::default(),
            10,
        );
        assert!(rows
            .iter()
            .any(|row| row.kind == RecommendationKind::OrphanProcess));
        assert!(rows.iter().all(|row| row.action_label.is_some()));
    }
    #[test]
    fn cooldown_deduplicates_battery_notifications() {
        let mut engine = PolicyEngine::default();
        let prefs = AutopilotPreferences {
            notify_on_battery: true,
            ..Default::default()
        };
        assert_eq!(
            engine
                .evaluate(&[resource()], None, PowerSourceType::Battery, &prefs, 100)
                .len(),
            1
        );
        assert!(engine
            .evaluate(&[resource()], None, PowerSourceType::Battery, &prefs, 101)
            .is_empty());
        assert_eq!(
            engine
                .evaluate(&[resource()], None, PowerSourceType::Battery, &prefs, 1000)
                .len(),
            1
        );
    }
    #[test]
    fn completion_is_advisory_and_requires_opt_in() {
        let mut engine = PolicyEngine::default();
        let mut prefs = AutopilotPreferences::default();
        engine.evaluate(&[resource()], None, PowerSourceType::Ac, &prefs, 1);
        assert!(engine
            .evaluate(&[], None, PowerSourceType::Ac, &prefs, 2)
            .is_empty());
        prefs.notify_on_session_completion = true;
        engine.evaluate(&[resource()], None, PowerSourceType::Ac, &prefs, 3);
        let result = engine.evaluate(&[], None, PowerSourceType::Ac, &prefs, 4);
        assert!(result.iter().all(|item| item.action_label.is_some()));
    }
    #[test]
    fn unknown_power_is_ineligible_for_ac_only() {
        assert!(!PowerSourceType::Unknown.is_ac());
    }
}
