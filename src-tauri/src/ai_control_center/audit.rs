use crate::models::AuditEntry;
use std::collections::VecDeque;
use std::path::Path;
use uuid::Uuid;

const FILE_NAME: &str = "ai-control-audit.json";
const MAX_ENTRIES: usize = 1024;
const MAX_FILE_BYTES: u64 = 512 * 1024;

#[derive(Debug, Clone, Default)]
pub struct AuditStore {
    entries: VecDeque<AuditEntry>,
}
impl AuditStore {
    pub fn load(config: &Path) -> Self {
        let path = config.join(FILE_NAME);
        let Ok(meta) = path.metadata() else {
            return Self::default();
        };
        if meta.len() > MAX_FILE_BYTES {
            return Self::default();
        }
        let Ok(bytes) = std::fs::read(path) else {
            return Self::default();
        };
        let mut entries =
            serde_json::from_slice::<VecDeque<AuditEntry>>(&bytes).unwrap_or_default();
        for entry in &mut entries {
            entry.event_kind = safe_label(&entry.event_kind);
            entry.outcome = safe_label(&entry.outcome);
            entry.project_ref = entry.project_ref.as_deref().map(safe_label);
            entry.message = crate::diagnostics::sanitize_log(&entry.message)
                .chars()
                .take(240)
                .collect();
        }
        while entries.len() > MAX_ENTRIES {
            entries.pop_front();
        }
        Self { entries }
    }
    pub fn append(
        &mut self,
        now: u64,
        event_kind: &str,
        outcome: &str,
        project_ref: Option<String>,
        message: &str,
        retention_days: u16,
    ) {
        let cutoff = now.saturating_sub(u64::from(retention_days.clamp(1, 365)) * 86400);
        self.entries.retain(|entry| entry.timestamp >= cutoff);
        let clean = crate::diagnostics::sanitize_log(message)
            .chars()
            .take(240)
            .collect();
        self.entries.push_back(AuditEntry {
            id: Uuid::new_v4().to_string(),
            timestamp: now,
            event_kind: safe_label(event_kind),
            outcome: safe_label(outcome),
            project_ref,
            message: clean,
        });
        while self.entries.len() > MAX_ENTRIES {
            self.entries.pop_front();
        }
    }
    pub fn entries(&self) -> Vec<AuditEntry> {
        self.entries.iter().rev().take(100).cloned().collect()
    }
    pub fn save(&self, config: &Path) -> Result<(), String> {
        std::fs::create_dir_all(config).map_err(|error| error.to_string())?;
        let path = config.join(FILE_NAME);
        let temp = config.join(format!("{FILE_NAME}.tmp"));
        let bytes = serde_json::to_vec(&self.entries).map_err(|error| error.to_string())?;
        if bytes.len() as u64 > MAX_FILE_BYTES {
            return Err("AI Control Center audit cap exceeded".into());
        }
        std::fs::write(&temp, bytes).map_err(|error| error.to_string())?;
        std::fs::rename(temp, path).map_err(|error| error.to_string())
    }
}
fn safe_label(value: &str) -> String {
    value
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
        .take(48)
        .collect()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn audit_is_redacted_bounded_and_persisted() {
        let temp = tempfile::tempdir().unwrap();
        let mut store = AuditStore::default();
        for index in 0..1100 {
            store.append(
                1000 + index,
                "scan",
                "ok",
                Some("project-opaque".into()),
                "token sk-abcdefghijklmnop1234",
                30,
            );
        }
        assert_eq!(store.entries.len(), MAX_ENTRIES);
        assert!(!serde_json::to_string(&store.entries())
            .unwrap()
            .contains("abcdefghijklmnop1234"));
        store.save(temp.path()).unwrap();
        assert_eq!(AuditStore::load(temp.path()).entries.len(), MAX_ENTRIES);
    }

    #[test]
    fn load_revalidates_tampered_entries_and_enforces_the_entry_cap() {
        let temp = tempfile::tempdir().unwrap();
        let entries = (0..1_100)
            .map(|index| AuditEntry {
                id: index.to_string(),
                timestamp: index,
                event_kind: "scan<script>".into(),
                outcome: "ok<script>".into(),
                project_ref: Some("project/<unsafe>".into()),
                message: "token sk-abcdefghijklmnop1234".into(),
            })
            .collect::<VecDeque<_>>();
        std::fs::write(
            temp.path().join(FILE_NAME),
            serde_json::to_vec(&entries).unwrap(),
        )
        .unwrap();

        let loaded = AuditStore::load(temp.path());
        assert_eq!(loaded.entries.len(), MAX_ENTRIES);
        let serialized = serde_json::to_string(&loaded.entries).unwrap();
        assert!(!serialized.contains("abcdefghijklmnop1234"));
        assert!(!serialized.contains('<'));
    }
}
