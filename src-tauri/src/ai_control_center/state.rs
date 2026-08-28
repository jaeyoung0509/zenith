use super::{
    audit::AuditStore, git::GitBaselineStore, policy::PolicyEngine, previews::PreviewStore,
};
use crate::models::{
    AiControlCenterSnapshot, ObservationQuality, ProviderObservation, Recommendation,
    SafetySnapshot,
};
use std::collections::HashMap;

pub struct AiControlCenterState {
    pub providers_last_success: HashMap<String, ProviderObservation>,
    pub git: GitBaselineStore,
    pub policy: PolicyEngine,
    pub safety: SafetySnapshot,
    pub recommendations: Vec<Recommendation>,
    pub audit: AuditStore,
    pub previews: PreviewStore,
    pub last_snapshot: Option<AiControlCenterSnapshot>,
}
impl Default for AiControlCenterState {
    fn default() -> Self {
        Self {
            providers_last_success: HashMap::new(),
            git: GitBaselineStore::default(),
            policy: PolicyEngine::default(),
            safety: SafetySnapshot {
                observed_at: 0,
                quality: ObservationQuality::Unavailable,
                findings: vec![],
                scanned_files: 0,
                skipped_files: 0,
                status_message:
                    "Run a bounded local safety scan to inspect registered active project roots."
                        .into(),
            },
            recommendations: vec![],
            audit: AuditStore::default(),
            previews: PreviewStore::default(),
            last_snapshot: None,
        }
    }
}
