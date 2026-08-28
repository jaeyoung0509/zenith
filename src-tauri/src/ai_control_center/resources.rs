use crate::models::*;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub fn attribute(
    snapshot: &AgentActivitySnapshot,
    roots: &HashMap<String, PathBuf>,
    listeners: &[DevelopmentListener],
    power: PowerSourceType,
    ac_only: bool,
) -> Vec<ResourceAttribution> {
    let mut port_counts: HashMap<String, u32> = HashMap::new();
    for listener in listeners {
        let Some(directory) = listener.working_directory.as_deref() else {
            continue;
        };
        let Ok(directory) = Path::new(directory).canonicalize() else {
            continue;
        };
        for (project_id, root) in roots {
            if directory.starts_with(root) {
                *port_counts.entry(project_id.clone()).or_default() += 1;
                break;
            }
        }
    }
    let eligible = !ac_only || power.is_ac();
    let mut result = Vec::new();
    for project in &snapshot.projects {
        for session in &project.sessions {
            result.push(ResourceAttribution { session_id:session.id.clone(),project_id:Some(project.identity.id.clone()),tool_name:session.tool_name.clone(),cpu_percent:session.cpu_percent,memory_bytes:session.memory_bytes,process_count:1,duration_seconds:session.elapsed_seconds,open_dev_ports:*port_counts.get(&project.identity.id).unwrap_or(&0),power_eligible:eligible,confidence:"process_observed".into(),reason:"Attributed through the canonical project/session snapshot; downstream actions still require opaque previews and fresh validation.".into(),mutable_actions_allowed:true });
        }
    }
    for session in &snapshot.unassigned_sessions {
        result.push(ResourceAttribution {
            session_id: session.id.clone(),
            project_id: None,
            tool_name: session.tool_name.clone(),
            cpu_percent: session.cpu_percent,
            memory_bytes: session.memory_bytes,
            process_count: 1,
            duration_seconds: session.elapsed_seconds,
            open_dev_ports: 0,
            power_eligible: eligible,
            confidence: "unassigned".into(),
            reason: "The agent process is verified, but project correlation cannot be proven."
                .into(),
            mutable_actions_allowed: false,
        });
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unassigned_never_has_mutable_authority() {
        let snapshot = AgentActivitySnapshot {
            observed_at: 1,
            quality: SnapshotQuality::Partial,
            projects: vec![],
            unassigned_sessions: vec![AgentSession {
                id: "opaque".into(),
                tool_id: "codex".into(),
                tool_name: "Codex".into(),
                status: AgentActivityStatus::Active,
                evidence: AgentEvidence::ProcessObserved,
                observed_at: 1,
                started_at: 1,
                elapsed_seconds: 1,
                cpu_percent: 1.0,
                memory_bytes: 1,
                project_id: None,
                detail: "x".into(),
            }],
            adapters: vec![],
            partial_errors: vec![],
        };
        let result = attribute(&snapshot, &HashMap::new(), &[], PowerSourceType::Ac, true);
        assert_eq!(result[0].confidence, "unassigned");
        assert!(!result[0].mutable_actions_allowed);
    }
}
