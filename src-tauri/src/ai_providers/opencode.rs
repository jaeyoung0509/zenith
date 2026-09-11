use super::registry::ProviderRegistry;
use super::support::{base_provider, parse_stat_f64, parse_stat_u64, strip_ansi};
use super::{CollectionContext, ProviderAdapter, ProviderDescriptor, ProviderError};
use crate::models::{AiProviderUsage, ProviderId, UsageSupport};
use crate::tooling;
use std::time::Duration;

#[derive(Default)]
pub struct OpenCodeAdapter;

impl ProviderAdapter for OpenCodeAdapter {
    fn id(&self) -> ProviderId {
        ProviderId::OpenCode
    }

    fn descriptor(&self) -> ProviderDescriptor {
        ProviderRegistry::find(ProviderId::OpenCode)
            .expect("OpenCode must exist in registry")
            .to_descriptor()
    }

    fn collect(&self, _ctx: &CollectionContext<'_>) -> Result<AiProviderUsage, ProviderError> {
        let mut provider = base_provider(ProviderId::OpenCode, "OpenCode", "Provider OAuth");
        provider.support = UsageSupport::Local;
        provider.action_url = Some("https://opencode.ai/docs/providers".into());

        let mut auth_cmd = tooling::command("opencode");
        auth_cmd.args(["auth", "list"]);
        let auth = match tooling::run_with_timeout(auth_cmd, Duration::from_secs(4)) {
            Ok(output) => output,
            Err(error) => {
                let error_str = error.to_string();
                if error_str.contains("No such file") || error_str.contains("not found") {
                    return Err(ProviderError::CliNotInstalled(
                        "OpenCode CLI was not detected in PATH or known tool locations.".into(),
                    ));
                } else {
                    return Err(ProviderError::CliFailed(format!(
                        "Could not inspect OpenCode: {error}"
                    )));
                }
            }
        };
        provider.installed = true;
        let auth_output = strip_ansi(&String::from_utf8_lossy(&auth.stdout));
        let oauth_count = auth_output
            .lines()
            .filter(|line| line.to_ascii_lowercase().contains("oauth"))
            .count();
        provider.connected = oauth_count > 0;
        provider.auth_label = format!(
            "{oauth_count} OAuth provider{}",
            if oauth_count == 1 { "" } else { "s" }
        );
        provider.status_message =
            "Local activity from `opencode stats`; quotas remain provider-owned.".into();

        let mut stats_cmd = tooling::command("opencode");
        stats_cmd.args(["stats", "--days", "7"]);
        if let Ok(output) = tooling::run_with_timeout(stats_cmd, Duration::from_secs(4)) {
            let stats = strip_ansi(&String::from_utf8_lossy(&output.stdout));
            provider.summary.local_sessions = parse_stat_u64(&stats, "Sessions");
            provider.summary.local_cost_usd = parse_stat_f64(&stats, "Total Cost");
        }
        Ok(provider)
    }
}
