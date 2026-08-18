use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ZenithError {
    PermissionDenied(String),
    PathNotAllowed(String),
    ChangedSinceScan(String),
    SignatureMismatch(String),
    ToolUnavailable(String),
    ExternalCommandFailed(String),
    BlacklistedPath(String),
    InvalidPlan(String),
    UnsupportedManualOperation(String),
    Io(String),
}

impl fmt::Display for ZenithError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ZenithError::PermissionDenied(p) => write!(f, "Permission denied for path: {}", p),
            ZenithError::PathNotAllowed(p) => write!(f, "Path is not allowed: {}", p),
            ZenithError::ChangedSinceScan(p) => write!(f, "File changed since scan: {}", p),
            ZenithError::SignatureMismatch(id) => write!(f, "Signature mismatch: {}", id),
            ZenithError::ToolUnavailable(t) => write!(f, "Tool unavailable: {}", t),
            ZenithError::ExternalCommandFailed(e) => write!(f, "External command failed: {}", e),
            ZenithError::BlacklistedPath(p) => {
                write!(f, "Attempted operation on blacklisted path: {}", p)
            }
            ZenithError::InvalidPlan(msg) => write!(f, "Invalid delete plan: {}", msg),
            ZenithError::UnsupportedManualOperation(name) => {
                write!(f, "Manual item requires a dedicated adapter: {}", name)
            }
            ZenithError::Io(e) => write!(f, "IO error: {}", e),
        }
    }
}

impl std::error::Error for ZenithError {}

impl From<std::io::Error> for ZenithError {
    fn from(err: std::io::Error) -> Self {
        ZenithError::Io(err.to_string())
    }
}

pub type ZenithResult<T> = Result<T, ZenithError>;
