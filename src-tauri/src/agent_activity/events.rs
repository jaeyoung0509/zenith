use crate::models::{AgentLifecycleEvent, AttentionReason, IngestedAgentEvent};
use std::path::PathBuf;

pub const MAX_PAYLOAD_BYTES: usize = 16 * 1024; // 16 KB
pub const MAX_TIMESTAMP_AGE_SECS: u64 = 3600; // 1 hour
pub const MAX_FUTURE_DRIFT_SECS: u64 = 60; // 60 seconds
pub const MAX_SESSION_ID_BYTES: usize = 256;
pub const MAX_TURN_ID_BYTES: usize = 256;
pub const MAX_CWD_BYTES: usize = 4096;
pub const ALLOWED_TOOLS: &[&str] = &[
    "antigravity",
    "claude",
    "cursor",
    "grok",
    "copilot",
    "codex",
    "gemini",
    "opencode",
];

#[derive(Debug, Clone, serde::Deserialize)]
pub struct RawAllowlistEvent {
    pub tool_id: String,
    pub session_id: String,
    pub cwd: Option<String>,
    pub event_type: String,
    pub timestamp: Option<u64>,
    pub turn_id: Option<String>,
    pub attention_reason: Option<String>,
    // All unknown fields (e.g. prompt, email, transcript, auth, secrets) are ignored by serde!
}

pub fn parse_and_validate_event(bytes: &[u8], now: u64) -> Result<IngestedAgentEvent, String> {
    if bytes.len() > MAX_PAYLOAD_BYTES {
        return Err("Payload exceeds maximum permitted size (16KB).".to_string());
    }

    let raw: RawAllowlistEvent = serde_json::from_slice(bytes)
        .map_err(|e| format!("Invalid or malformed event payload: {e}"))?;

    if !ALLOWED_TOOLS.contains(&raw.tool_id.as_str()) {
        return Err(format!("Unknown or unapproved tool ID: {}", raw.tool_id));
    }

    if raw.session_id.trim().is_empty() {
        return Err("Session ID must not be empty.".to_string());
    }

    let lifecycle = match raw.event_type.to_lowercase().as_str() {
        "session_start" | "sessionstart" | "start" => AgentLifecycleEvent::SessionStart,
        "working" | "busy" | "running" => AgentLifecycleEvent::Working,
        "waiting_for_user" | "waiting" | "input_needed" | "approval_needed" => {
            AgentLifecycleEvent::WaitingForUser
        }
        "idle" => AgentLifecycleEvent::Idle,
        "turn_complete" | "completed" | "stop" => AgentLifecycleEvent::TurnComplete,
        "session_end" | "sessionend" | "exit" => AgentLifecycleEvent::SessionEnd,
        _ => return Err(format!("Unknown lifecycle event: {}", raw.event_type)),
    };

    let attention_reason = match raw
        .attention_reason
        .as_deref()
        .map(str::to_lowercase)
        .as_deref()
    {
        Some("approval") => Some(AttentionReason::Approval),
        Some("input") => Some(AttentionReason::Input),
        Some("turn_complete") => Some(AttentionReason::TurnComplete),
        Some("inactivity") => Some(AttentionReason::Inactivity),
        _ => {
            if lifecycle == AgentLifecycleEvent::WaitingForUser {
                Some(AttentionReason::Input)
            } else if lifecycle == AgentLifecycleEvent::TurnComplete {
                Some(AttentionReason::TurnComplete)
            } else {
                None
            }
        }
    };

    let timestamp = raw.timestamp.unwrap_or(now);
    if timestamp.saturating_add(MAX_TIMESTAMP_AGE_SECS) < now {
        return Err("Event timestamp is too stale (> 1 hour old).".to_string());
    }
    if timestamp > now + MAX_FUTURE_DRIFT_SECS {
        return Err("Event timestamp is in the future.".to_string());
    }

    let cwd = raw.cwd.and_then(|p| {
        let path = PathBuf::from(p);
        if path.is_absolute() {
            Some(path.display().to_string())
        } else {
            None
        }
    });

    validate_ingested_event(
        IngestedAgentEvent {
            tool_id: raw.tool_id,
            vendor_session_id: raw.session_id,
            cwd,
            lifecycle,
            timestamp,
            turn_id: raw.turn_id,
            attention_reason,
        },
        now,
    )
}

pub fn validate_ingested_event(
    event: IngestedAgentEvent,
    now: u64,
) -> Result<IngestedAgentEvent, String> {
    if !ALLOWED_TOOLS.contains(&event.tool_id.as_str()) {
        return Err(format!("Unknown or unapproved tool ID: {}", event.tool_id));
    }
    if event.vendor_session_id.trim().is_empty()
        || event.vendor_session_id.len() > MAX_SESSION_ID_BYTES
    {
        return Err("Session ID is empty or exceeds 256 bytes.".to_string());
    }
    if event
        .turn_id
        .as_ref()
        .is_some_and(|turn_id| turn_id.len() > MAX_TURN_ID_BYTES)
    {
        return Err("Turn ID exceeds 256 bytes.".to_string());
    }
    if event.timestamp.saturating_add(MAX_TIMESTAMP_AGE_SECS) < now {
        return Err("Event timestamp is too stale (> 1 hour old).".to_string());
    }
    if event.timestamp > now.saturating_add(MAX_FUTURE_DRIFT_SECS) {
        return Err("Event timestamp is in the future.".to_string());
    }
    if let Some(cwd) = &event.cwd {
        if cwd.len() > MAX_CWD_BYTES || !PathBuf::from(cwd).is_absolute() {
            return Err("Event working directory is invalid.".to_string());
        }
    }
    Ok(event)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_valid_allowlist_event_and_discards_sensitive_fields() {
        let now = 1_000_000;
        let payload = r#"{
            "tool_id": "antigravity",
            "session_id": "session-123",
            "cwd": "/Users/apple/Myproject/clean1",
            "event_type": "working",
            "timestamp": 1000000,
            "prompt": "Super secret prompt",
            "email": "user@example.com",
            "transcript_path": "/var/logs/transcript.jsonl",
            "api_key": "sk-1234567890"
        }"#;

        let res = parse_and_validate_event(payload.as_bytes(), now);
        assert!(res.is_ok());
        let event = res.unwrap();
        assert_eq!(event.tool_id, "antigravity");
        assert_eq!(event.vendor_session_id, "session-123");
        assert_eq!(event.lifecycle, AgentLifecycleEvent::Working);
        assert_eq!(event.timestamp, 1_000_000);

        // Serialize back to verify no sensitive fields leaked
        let serialized = serde_json::to_string(&event).unwrap();
        assert!(!serialized.contains("Super secret prompt"));
        assert!(!serialized.contains("user@example.com"));
        assert!(!serialized.contains("transcript.jsonl"));
        assert!(!serialized.contains("sk-1234567890"));
    }

    #[test]
    fn rejects_oversized_payload() {
        let now = 1_000_000;
        let oversized = vec![b'a'; MAX_PAYLOAD_BYTES + 10];
        let res = parse_and_validate_event(&oversized, now);
        assert!(res.is_err());
        assert!(res.unwrap_err().contains("maximum permitted size"));
    }

    #[test]
    fn rejects_stale_and_future_timestamps() {
        let now = 1_000_000;
        let stale_payload = r#"{
            "tool_id": "claude",
            "session_id": "session-stale",
            "event_type": "working",
            "timestamp": 900000
        }"#;
        let res = parse_and_validate_event(stale_payload.as_bytes(), now);
        assert!(res.is_err());
        assert!(res.unwrap_err().contains("too stale"));

        let future_payload = r#"{
            "tool_id": "claude",
            "session_id": "session-future",
            "event_type": "working",
            "timestamp": 1000100
        }"#;
        let res2 = parse_and_validate_event(future_payload.as_bytes(), now);
        assert!(res2.is_err());
        assert!(res2.unwrap_err().contains("in the future"));
    }

    #[test]
    fn rejects_unknown_tool_and_malformed_json() {
        let now = 1_000_000;
        let unknown_tool = r#"{
            "tool_id": "malicious_tool",
            "session_id": "s1",
            "event_type": "working"
        }"#;
        assert!(parse_and_validate_event(unknown_tool.as_bytes(), now).is_err());

        let malformed = b"{ not json }";
        assert!(parse_and_validate_event(malformed, now).is_err());
    }

    #[test]
    fn typed_event_validation_rejects_unbounded_or_relative_fields() {
        let event = IngestedAgentEvent {
            tool_id: "codex".into(),
            vendor_session_id: "x".repeat(MAX_SESSION_ID_BYTES + 1),
            cwd: Some("relative/path".into()),
            lifecycle: AgentLifecycleEvent::Working,
            timestamp: 1_000_000,
            turn_id: None,
            attention_reason: None,
        };
        assert!(validate_ingested_event(event, 1_000_000).is_err());
    }
}
