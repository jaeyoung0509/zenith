use std::fmt;

use crate::domain::cleanup::PlanItemRefusal;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ZenithError {
    PermissionDenied(String),
    PathNotAllowed(String),
    SymlinkEscape(String),
    ChangedSinceScan(String),
    /// The path no longer exists. Absence is deliberately not
    /// [`ZenithError::ChangedSinceScan`]: the caller's postcondition already
    /// holds, so it is classified as already-absent rather than as a mutation
    /// failure. Every other failure still fails closed.
    Missing(String),
    SignatureMismatch(String),
    ToolUnavailable(String),
    ExternalCommandFailed(String),
    BlacklistedPath(String),
    InvalidPlan(String),
    UnsupportedManualOperation(String),
    /// Nothing the caller selected could be authorized, and every item states
    /// why.
    ///
    /// This is an answer about the items rather than a failure of the scan: a
    /// current policy refused each of them, so the inventory the caller holds
    /// is still the truth about the machine and the refusal is stated per item
    /// instead of discarding the selection.
    RefusedSelection(Vec<PlanItemRefusal>),
    Io(String),
}

impl fmt::Display for ZenithError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ZenithError::PermissionDenied(p) => write!(f, "Permission denied for path: {}", p),
            ZenithError::PathNotAllowed(p) => write!(f, "Path is not allowed: {}", p),
            ZenithError::SymlinkEscape(p) => write!(f, "Symlink escape attempt rejected: {}", p),
            ZenithError::ChangedSinceScan(p) => write!(f, "File changed since scan: {}", p),
            ZenithError::Missing(p) => write!(f, "Path no longer exists: {}", p),
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
            ZenithError::RefusedSelection(refusals) => {
                let first = refusals
                    .first()
                    .map(|refusal| refusal.message.as_str())
                    .unwrap_or("no item in the selection can be cleaned");
                write!(
                    f,
                    "Nothing in the selection can be cleaned ({} item(s) refused): {}",
                    refusals.len(),
                    first
                )
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
