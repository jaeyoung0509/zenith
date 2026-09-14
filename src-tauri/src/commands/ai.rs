//! AI usage, agent activity, and AI Control Center command handlers.
//!
//! Thin Tauri IPC adapters: decode the request, hand the desktop adapters to
//! one application use case, and map the typed result. The caches,
//! single-flight collections, Control Center state, audit store, and provider
//! credential flows belong to [`crate::services::AiService`], so no handler
//! locks a cache or invalidates a snapshot for itself.

use std::path::PathBuf;
use std::sync::Arc;

use tauri::ipc::Channel;
use tauri::{AppHandle, Manager, State};

use super::state::DesktopState;
use crate::events::ai::TauriProviderUsageProgress;
use crate::events::notifications::TauriNotifications;
use crate::models::{
    AgentActivitySnapshot, AgentIntegrationInfo, AgentIntegrationResult, AgentQuickSummary,
    AiControlCenterSnapshot, AiControlPreferences, AiProviderUsage, AiUsageSnapshot,
    ControlCenterQuickSummary, IngestedAgentEvent, RecommendationPreview,
};
use crate::services::progress::ProviderUsageSink;

/// The directory the desktop shell persists settings and audit entries in.
///
/// Resolving it here keeps the application services free of the Tauri path
/// API, which is what lets them be exercised without a runtime.
fn app_config_dir(app_handle: &AppHandle) -> Result<PathBuf, String> {
    app_handle
        .path()
        .app_config_dir()
        .map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn get_ai_usage(
    on_event: Channel<AiProviderUsage>,
    force: Option<bool>,
    state: State<'_, DesktopState>,
) -> Result<AiUsageSnapshot, String> {
    let progress: Arc<dyn ProviderUsageSink> = Arc::new(TauriProviderUsageProgress::new(on_event));
    state
        .ai
        .usage_snapshot(force.unwrap_or(false), progress)
        .await
}

#[tauri::command]
#[specta::specta]
pub async fn get_project_context(
    force: Option<bool>,
    app_handle: AppHandle,
    state: State<'_, DesktopState>,
) -> Result<AgentActivitySnapshot, String> {
    let notifications = Arc::new(TauriNotifications::new(app_handle));
    state
        .ai
        .project_context(force.unwrap_or(false), notifications)
        .await
}

#[tauri::command]
#[specta::specta]
pub async fn open_project_in_terminal(
    project_id: String,
    state: State<'_, DesktopState>,
) -> Result<(), String> {
    state.ai.open_project_in_terminal(&project_id).await
}

#[tauri::command]
#[specta::specta]
pub async fn request_stop_agent_session(
    session_id: String,
    lease_id: String,
    state: State<'_, DesktopState>,
) -> Result<(), String> {
    state.ai.stop_agent_session(&session_id, &lease_id).await
}

#[tauri::command]
#[specta::specta]
pub async fn get_agent_integrations(
    state: State<'_, DesktopState>,
) -> Result<Vec<AgentIntegrationInfo>, String> {
    state.ai.agent_integrations().await
}

#[tauri::command]
#[specta::specta]
pub async fn setup_agent_integration(
    tool_id: String,
    state: State<'_, DesktopState>,
) -> Result<AgentIntegrationResult, String> {
    state.ai.setup_agent_integration(&tool_id).await
}

#[tauri::command]
#[specta::specta]
pub async fn remove_agent_integration(
    tool_id: String,
    state: State<'_, DesktopState>,
) -> Result<AgentIntegrationResult, String> {
    state.ai.remove_agent_integration(&tool_id).await
}

#[tauri::command]
#[specta::specta]
pub async fn get_agent_quick_summary(
    state: State<'_, DesktopState>,
) -> Result<Option<AgentQuickSummary>, String> {
    state.ai.agent_quick_summary().await
}

#[tauri::command]
#[specta::specta]
pub fn post_agent_event(
    event: IngestedAgentEvent,
    state: State<'_, DesktopState>,
) -> Result<(), String> {
    state.ai.ingest_agent_event(event)
}

#[tauri::command]
#[specta::specta]
pub async fn get_ai_control_center(
    force: Option<bool>,
    app_handle: AppHandle,
    state: State<'_, DesktopState>,
) -> Result<AiControlCenterSnapshot, String> {
    let config_dir = app_config_dir(&app_handle)?;
    state
        .ai
        .control_center(force.unwrap_or(false), &config_dir)
        .await
}

#[tauri::command]
#[specta::specta]
pub fn get_ai_control_quick_summary(
    state: State<'_, DesktopState>,
) -> Option<ControlCenterQuickSummary> {
    state.ai.control_quick_summary()
}

#[tauri::command]
#[specta::specta]
pub async fn save_ai_control_preferences(
    preferences: AiControlPreferences,
    app_handle: AppHandle,
    state: State<'_, DesktopState>,
) -> Result<(), String> {
    let config_dir = app_config_dir(&app_handle)?;
    let notifications = Arc::new(TauriNotifications::new(app_handle));
    state
        .ai
        .save_control_preferences(preferences, &config_dir, notifications)
        .await
}

#[tauri::command]
#[specta::specta]
pub async fn run_ai_safety_scan(
    app_handle: AppHandle,
    state: State<'_, DesktopState>,
) -> Result<crate::models::SafetySnapshot, String> {
    let config_dir = app_config_dir(&app_handle)?;
    state.ai.run_safety_scan(&config_dir).await
}

#[tauri::command]
#[specta::specta]
pub async fn dismiss_ai_safety_finding(
    finding_id: String,
    app_handle: AppHandle,
    state: State<'_, DesktopState>,
) -> Result<(), String> {
    let config_dir = app_config_dir(&app_handle)?;
    state
        .ai
        .dismiss_safety_finding(&finding_id, &config_dir)
        .await
}

#[tauri::command]
#[specta::specta]
pub async fn preview_ai_recommendation(
    recommendation_id: String,
    app_handle: AppHandle,
    state: State<'_, DesktopState>,
) -> Result<RecommendationPreview, String> {
    let config_dir = app_config_dir(&app_handle)?;
    state
        .ai
        .preview_recommendation(&recommendation_id, &config_dir)
        .await
}

#[tauri::command]
#[specta::specta]
pub async fn consume_ai_recommendation_preview(
    preview_id: String,
    app_handle: AppHandle,
    state: State<'_, DesktopState>,
) -> Result<RecommendationPreview, String> {
    let config_dir = app_config_dir(&app_handle)?;
    state
        .ai
        .consume_recommendation_preview(&preview_id, &config_dir)
        .await
}

#[tauri::command]
#[specta::specta]
pub async fn get_ai_control_git_diff(
    project_id: String,
    app_handle: AppHandle,
    state: State<'_, DesktopState>,
) -> Result<String, String> {
    let config_dir = app_config_dir(&app_handle)?;
    state.ai.control_git_diff(&project_id, &config_dir).await
}

#[tauri::command]
#[specta::specta]
pub async fn connect_openrouter_oauth(state: State<'_, DesktopState>) -> Result<(), String> {
    state.ai.connect_openrouter().await
}

#[tauri::command]
#[specta::specta]
pub async fn delete_ai_provider_credential(
    provider: crate::models::ProviderId,
    state: State<'_, DesktopState>,
) -> Result<(), String> {
    state.ai.delete_provider_credential(provider).await
}

#[tauri::command]
#[specta::specta]
pub fn get_ai_provider_descriptors() -> Vec<crate::ai_providers::ProviderDescriptor> {
    crate::ai_providers::ProviderRegistry::all()
}
