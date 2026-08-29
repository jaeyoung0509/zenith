use crate::models::{AgentSession, DevelopmentListener, ProjectContext, ProjectIdentity};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub fn correlate(
    mut projects: HashMap<String, (PathBuf, ProjectIdentity)>,
    sessions: Vec<(Option<PathBuf>, AgentSession)>,
    dev_listeners: &[DevelopmentListener],
    artifact_sizes: &HashMap<PathBuf, u64>,
    now: u64,
) -> (Vec<ProjectContext>, Vec<AgentSession>) {
    let mut project_contexts: HashMap<String, ProjectContext> = HashMap::new();
    let mut project_roots: HashMap<String, PathBuf> = HashMap::new();

    for (id, (root, identity)) in projects.drain() {
        project_roots.insert(id.clone(), root);
        project_contexts.insert(
            id,
            ProjectContext {
                identity,
                sessions: Vec::new(),
                last_seen_at: now,
                dev_ports: Vec::new(),
                artifact_size_bytes: None,
            },
        );
    }

    let mut unassigned_sessions = Vec::new();

    for (cwd, mut session) in sessions {
        let matched_project_id = cwd
            .as_deref()
            .and_then(|path| find_deepest_ancestor_project(path, &project_roots));

        if let Some(project_id) = matched_project_id {
            session.project_id = Some(project_id.clone());
            if let Some(context) = project_contexts.get_mut(&project_id) {
                context.sessions.push(session);
            } else {
                unassigned_sessions.push(session);
            }
        } else {
            session.project_id = None;
            unassigned_sessions.push(session);
        }
    }

    // Correlate development listeners
    for listener in dev_listeners {
        let Some(dir) = listener.working_directory.as_deref() else {
            continue;
        };
        let Ok(canonical_dir) = Path::new(dir).canonicalize() else {
            continue;
        };
        if let Some(project_id) = find_deepest_ancestor_project(&canonical_dir, &project_roots) {
            if let Some(context) = project_contexts.get_mut(&project_id) {
                if !context.dev_ports.contains(&listener.port) {
                    context.dev_ports.push(listener.port);
                }
            }
        }
    }

    // Correlate developer artifact totals
    for (project_id, root) in &project_roots {
        if let Some(size) = artifact_sizes.get(root) {
            if let Some(context) = project_contexts.get_mut(project_id) {
                context.artifact_size_bytes = Some(*size);
            }
        }
    }

    for context in project_contexts.values_mut() {
        context.dev_ports.sort_unstable();
        context
            .sessions
            .sort_by(|a, b| a.tool_name.cmp(&b.tool_name).then_with(|| a.id.cmp(&b.id)));
    }

    let mut result_projects = project_contexts.into_values().collect::<Vec<_>>();

    // Sort: attention first, then active count, then most recently seen, then display name
    result_projects.sort_by(|a, b| {
        let a_attention = a.sessions.iter().any(|s| s.attention_reason.is_some());
        let b_attention = b.sessions.iter().any(|s| s.attention_reason.is_some());
        b_attention
            .cmp(&a_attention)
            .then_with(|| b.sessions.len().cmp(&a.sessions.len()))
            .then_with(|| b.last_seen_at.cmp(&a.last_seen_at))
            .then_with(|| a.identity.display_name.cmp(&b.identity.display_name))
    });

    unassigned_sessions.sort_by(|a, b| a.tool_name.cmp(&b.tool_name).then_with(|| a.id.cmp(&b.id)));

    (result_projects, unassigned_sessions)
}

/// Finds the deepest canonical ancestor project root for a given path.
/// Canonical path matching prevents same-basename projects and sibling worktrees from colliding.
pub fn find_deepest_ancestor_project(
    path: &Path,
    project_roots: &HashMap<String, PathBuf>,
) -> Option<String> {
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let mut best_match: Option<(String, usize)> = None;

    for (id, root) in project_roots {
        if canonical.starts_with(root) || path.starts_with(root) {
            let component_count = root.components().count();
            if best_match
                .as_ref()
                .is_none_or(|(_, count)| component_count > *count)
            {
                best_match = Some((id.clone(), component_count));
            }
        }
    }

    best_match.map(|(id, _)| id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{AgentActivityStatus, AgentEvidence, AttentionReason};

    fn make_session(id: &str, tool: &str) -> AgentSession {
        AgentSession {
            id: id.into(),
            tool_id: tool.to_lowercase(),
            tool_name: tool.into(),
            status: AgentActivityStatus::Working,
            attention_reason: None,
            evidence: AgentEvidence::ProcessObserved,
            observed_at: 100,
            started_at: 10,
            elapsed_seconds: 90,
            cpu_percent: 1.0,
            memory_bytes: 1024,
            project_id: None,
            worktree_id: None,
            detail: "Working".into(),
            can_stop: true,
            stop_lease_id: None,
        }
    }

    fn make_identity(id: &str, name: &str) -> ProjectIdentity {
        ProjectIdentity {
            id: id.into(),
            display_name: name.into(),
            location_hint: name.into(),
            display_path: format!("/path/{name}"),
            repository_id: Some("repo-1".into()),
            worktree_id: None,
            is_worktree: false,
            branch: Some("main".into()),
            is_dirty: false,
            is_detached: false,
        }
    }

    #[test]
    fn deepest_canonical_ancestry_matching_rejects_parent_or_same_basename() {
        let mut roots = HashMap::new();
        roots.insert("parent".into(), PathBuf::from("/workspace/mono"));
        roots.insert(
            "child".into(),
            PathBuf::from("/workspace/mono/services/api"),
        );
        roots.insert("sibling".into(), PathBuf::from("/workspace/other/api"));

        assert_eq!(
            find_deepest_ancestor_project(Path::new("/workspace/mono/services/api/src"), &roots)
                .as_deref(),
            Some("child")
        );
        assert_eq!(
            find_deepest_ancestor_project(Path::new("/workspace/mono/packages/ui"), &roots)
                .as_deref(),
            Some("parent")
        );
        assert_eq!(
            find_deepest_ancestor_project(Path::new("/workspace/other/api/src"), &roots).as_deref(),
            Some("sibling")
        );
        assert_eq!(
            find_deepest_ancestor_project(Path::new("/tmp/unrelated"), &roots),
            None
        );
    }

    #[test]
    fn unprovable_sessions_remain_unassigned() {
        let mut projects = HashMap::new();
        projects.insert(
            "p1".into(),
            (PathBuf::from("/workspace/app"), make_identity("p1", "app")),
        );

        let sessions = vec![
            (
                Some(PathBuf::from("/workspace/app/src")),
                make_session("s1", "Antigravity"),
            ),
            (
                Some(PathBuf::from("/different/path")),
                make_session("s2", "Codex"),
            ),
            (None, make_session("s3", "Claude")),
        ];

        let (contexts, unassigned) = correlate(projects, sessions, &[], &HashMap::new(), 100);

        assert_eq!(contexts.len(), 1);
        assert_eq!(contexts[0].sessions.len(), 1);
        assert_eq!(contexts[0].sessions[0].id, "s1");
        assert_eq!(unassigned.len(), 2);
        assert_eq!(unassigned[0].id, "s3");
        assert_eq!(unassigned[1].id, "s2");
    }

    #[test]
    fn attention_first_sorting() {
        let mut projects = HashMap::new();
        projects.insert(
            "p1".into(),
            (
                PathBuf::from("/workspace/app1"),
                make_identity("p1", "app1"),
            ),
        );
        projects.insert(
            "p2".into(),
            (
                PathBuf::from("/workspace/app2"),
                make_identity("p2", "app2"),
            ),
        );

        let mut s2 = make_session("s2", "Codex");
        s2.attention_reason = Some(AttentionReason::Input);

        let sessions = vec![
            (
                Some(PathBuf::from("/workspace/app1")),
                make_session("s1", "Antigravity"),
            ),
            (Some(PathBuf::from("/workspace/app2")), s2),
        ];

        let (contexts, _) = correlate(projects, sessions, &[], &HashMap::new(), 100);

        assert_eq!(contexts.len(), 2);
        assert_eq!(contexts[0].identity.id, "p2"); // Attention reason sorts first!
        assert_eq!(contexts[1].identity.id, "p1");
    }
}
