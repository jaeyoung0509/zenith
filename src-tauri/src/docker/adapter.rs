use crate::models::{
    Category, DockerContainerItem, DockerImageItem, DockerOverview, DockerStatus, DockerVolumeItem,
    FileSize, RiskTier, ScanItem, ZenithError,
};
use crate::tooling;

/// A Docker-compatible container CLI. `docker` is preferred; `podman` is a
/// real fallback, so a machine with a stopped Docker daemon but a running
/// Podman machine stays supported.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ContainerCli {
    Docker,
    Podman,
}

impl ContainerCli {
    const ALL: [Self; 2] = [Self::Docker, Self::Podman];

    fn executable(self) -> &'static str {
        match self {
            Self::Docker => "docker",
            Self::Podman => "podman",
        }
    }
}

/// True when a Docker-compatible CLI is installed. Capability reporting uses
/// this same source of truth so a Podman-only machine is not reported as
/// unsupported while the adapter can drive Podman.
pub fn container_cli_detected() -> bool {
    container_cli_detected_with(|name| tooling::resolve(name).is_some())
}

fn container_cli_detected_with(resolve: impl Fn(&str) -> bool) -> bool {
    ContainerCli::ALL
        .iter()
        .any(|cli| resolve(cli.executable()))
}

/// Selects the first installed CLI that can actually reach a daemon.
fn select_running_cli(
    installed: &[ContainerCli],
    reaches_daemon: impl Fn(ContainerCli) -> bool,
) -> Option<ContainerCli> {
    installed.iter().copied().find(|cli| reaches_daemon(*cli))
}

pub struct DockerAdapter;

impl DockerAdapter {
    fn installed_clis() -> Vec<ContainerCli> {
        ContainerCli::ALL
            .iter()
            .copied()
            .filter(|cli| tooling::resolve(cli.executable()).is_some())
            .collect()
    }

    fn cli_reaches_daemon(cli: ContainerCli) -> bool {
        let mut cmd = tooling::command(cli.executable());
        cmd.args(["info", "--format", "{{.ServerVersion}}"]);
        matches!(
            tooling::run_with_timeout(cmd, std::time::Duration::from_secs(4)),
            Ok(output) if output.status.success()
        )
    }

    fn active_cli() -> Option<ContainerCli> {
        select_running_cli(&Self::installed_clis(), Self::cli_reaches_daemon)
    }

    /// The CLI used by operations that were already authorized by a status
    /// check: the running runtime when one exists, otherwise the first
    /// installed CLI so error reporting still names a real executable.
    fn preferred_cli() -> ContainerCli {
        Self::active_cli()
            .or_else(|| Self::installed_clis().first().copied())
            .unwrap_or(ContainerCli::Docker)
    }

    fn cli_version(cli: ContainerCli) -> Option<String> {
        let mut cmd = tooling::command(cli.executable());
        cmd.arg("--version");
        tooling::run_with_timeout(cmd, std::time::Duration::from_secs(3))
            .ok()
            .filter(|output| output.status.success())
            .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
            .filter(|version| !version.is_empty())
    }

    fn missing_cli_message() -> String {
        match std::env::var_os("DOCKER_HOST") {
            Some(host) => format!(
                "DOCKER_HOST={} is set but no docker or podman CLI was detected in PATH or known tool locations.",
                host.to_string_lossy()
            ),
            None => {
                "No docker or podman CLI was detected in PATH or known tool locations.".to_string()
            }
        }
    }

    fn daemon_unreachable_message() -> String {
        match std::env::var_os("DOCKER_HOST") {
            Some(host) => format!(
                "No container daemon responded at DOCKER_HOST={}. Start the runtime or clear DOCKER_HOST to use the active context.",
                host.to_string_lossy()
            ),
            None => {
                "No container daemon responded for the active context. Start Docker, Podman, or Rancher Desktop.".to_string()
            }
        }
    }

    /// Checks installed CLIs and reports the first one whose daemon answers.
    pub fn get_status() -> DockerStatus {
        let installed = Self::installed_clis();
        if installed.is_empty() {
            return DockerStatus {
                is_available: false,
                is_running: false,
                version: None,
                error_message: Some(Self::missing_cli_message()),
                overview: None,
                images: Vec::new(),
                containers: Vec::new(),
                volumes: Vec::new(),
            };
        }

        let version = Self::cli_version(installed[0]);
        let Some(cli) = Self::active_cli() else {
            return DockerStatus {
                is_available: true,
                is_running: false,
                version,
                error_message: Some(Self::daemon_unreachable_message()),
                overview: None,
                images: Vec::new(),
                containers: Vec::new(),
                volumes: Vec::new(),
            };
        };

        let overview = Self::get_overview_with(cli);
        let containers = Self::get_containers_with(cli);
        let images = Self::get_images_from_containers_with(cli, &containers);
        let volumes = Self::get_volumes_with(cli);

        DockerStatus {
            is_available: true,
            is_running: true,
            version,
            error_message: None,
            overview: Some(overview),
            images,
            containers,
            volumes,
        }
    }

    /// Generates ScanItems for Docker artifacts when the Docker daemon is active.
    pub fn scan_items() -> Vec<ScanItem> {
        let overview = Self::get_overview();
        if overview.total_bytes == 0 && overview.total_reclaimable_bytes == 0 {
            return Vec::new();
        }

        let images = Self::get_images();
        let mut items = Vec::new();

        let dangling_size: u64 = images
            .iter()
            .filter(|i| i.is_dangling)
            .map(|i| i.size_bytes)
            .sum();
        let dangling_count = images.iter().filter(|i| i.is_dangling).count();

        if dangling_count > 0 {
            items.push(ScanItem {
                id: "container.docker.dangling_images".to_string(),
                signature_id: "container.docker.dangling_images".to_string(),
                name: "Docker Dangling Images".to_string(),
                category: Category::Container,
                risk: RiskTier::Safe,
                path: "docker://images/dangling".to_string(),
                size: FileSize::new(dangling_size, Some(dangling_size)),
                file_count: dangling_count,
                description: format!("{dangling_count} untagged intermediate image layers"),
                cache_metadata: Default::default(),
                is_selected: true,
                last_modified: None,
                exists: true,
            });
        }

        if overview.build_cache.reclaimable_bytes > 0 {
            items.push(ScanItem {
                id: "container.docker.builder".to_string(),
                signature_id: "container.docker.builder".to_string(),
                name: "Docker BuildKit Cache".to_string(),
                category: Category::Container,
                risk: RiskTier::Safe,
                path: "docker://buildkit/cache".to_string(),
                size: FileSize::new(
                    overview.build_cache.reclaimable_bytes,
                    Some(overview.build_cache.reclaimable_bytes),
                ),
                file_count: 0,
                description: "Reusable BuildKit build cache layers".to_string(),
                cache_metadata: Default::default(),
                is_selected: true,
                last_modified: None,
                exists: true,
            });
        }

        if overview.images.reclaimable_bytes > 0 {
            let unused_count = images.iter().filter(|i| !i.is_in_use).count();
            items.push(ScanItem {
                id: "container.docker.unused_images".to_string(),
                signature_id: "container.docker.unused_images".to_string(),
                name: "Docker Unused Images".to_string(),
                category: Category::Container,
                risk: RiskTier::Rebuild,
                path: "docker://images/unused".to_string(),
                size: FileSize::new(
                    overview.images.reclaimable_bytes,
                    Some(overview.images.reclaimable_bytes),
                ),
                file_count: unused_count,
                description: "Images not referenced by any running or stopped container"
                    .to_string(),
                cache_metadata: Default::default(),
                is_selected: false,
                last_modified: None,
                exists: true,
            });
        }

        if overview.containers.reclaimable_bytes > 0 {
            items.push(ScanItem {
                id: "container.docker.stopped_containers".to_string(),
                signature_id: "container.docker.stopped_containers".to_string(),
                name: "Docker Stopped Containers".to_string(),
                category: Category::Container,
                risk: RiskTier::Rebuild,
                path: "docker://containers/stopped".to_string(),
                size: FileSize::new(
                    overview.containers.reclaimable_bytes,
                    Some(overview.containers.reclaimable_bytes),
                ),
                file_count: 0,
                description: "Exited containers holding read-write layer state".to_string(),
                cache_metadata: Default::default(),
                is_selected: false,
                last_modified: None,
                exists: true,
            });
        }

        if overview.volumes.reclaimable_bytes > 0 {
            items.push(ScanItem {
                id: "container.docker.unused_volumes".to_string(),
                signature_id: "container.docker.unused_volumes".to_string(),
                name: "Docker Unused Volumes".to_string(),
                category: Category::Container,
                risk: RiskTier::Manual,
                path: "docker://volumes/unused".to_string(),
                size: FileSize::new(
                    overview.volumes.reclaimable_bytes,
                    Some(overview.volumes.reclaimable_bytes),
                ),
                file_count: 0,
                description: "Anonymous and orphaned persistent storage volumes".to_string(),
                cache_metadata: Default::default(),
                is_selected: false,
                last_modified: None,
                exists: true,
            });
        }

        items
    }

    /// Queries `docker system df` / `podman system df` and parses image,
    /// container, volume, and build cache usage.
    pub fn get_overview() -> DockerOverview {
        Self::get_overview_with(Self::preferred_cli())
    }

    fn get_overview_with(cli: ContainerCli) -> DockerOverview {
        let args: &[&str] = match cli {
            ContainerCli::Docker => &["system", "df", "--format", "{{json .}}"],
            // Podman emits a single JSON array for `--format json`.
            ContainerCli::Podman => &["system", "df", "--format", "json"],
        };
        let records = Self::run_json_records(cli, args, 5);
        Self::parse_overview_records(&records)
    }

    /// Runs a JSON-emitting command and accepts both Docker's JSON-lines
    /// output and Podman's single JSON array.
    fn run_json_records(
        cli: ContainerCli,
        args: &[&str],
        timeout_secs: u64,
    ) -> Vec<serde_json::Value> {
        let mut cmd = tooling::command(cli.executable());
        cmd.args(args);
        let output = tooling::run_with_timeout(cmd, std::time::Duration::from_secs(timeout_secs));
        match output {
            Ok(out) if out.status.success() => {
                Self::parse_json_records(&String::from_utf8_lossy(&out.stdout))
            }
            _ => Vec::new(),
        }
    }

    fn parse_json_records(stdout: &str) -> Vec<serde_json::Value> {
        let trimmed = stdout.trim();
        if trimmed.starts_with('[') {
            if let Ok(serde_json::Value::Array(items)) = serde_json::from_str(trimmed) {
                return items;
            }
        }
        trimmed
            .lines()
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect()
    }

    fn json_size(value: Option<&serde_json::Value>) -> u64 {
        match value {
            Some(serde_json::Value::String(text)) => Self::parse_docker_size(text),
            Some(serde_json::Value::Number(number)) => number.as_u64().unwrap_or(0),
            _ => 0,
        }
    }

    fn json_reclaimable(value: Option<&serde_json::Value>) -> u64 {
        match value {
            Some(serde_json::Value::String(text)) => Self::parse_docker_reclaimable(text),
            Some(serde_json::Value::Number(number)) => number.as_u64().unwrap_or(0),
            _ => 0,
        }
    }

    #[cfg(test)]
    fn parse_overview(stdout: &str) -> DockerOverview {
        Self::parse_overview_records(&Self::parse_json_records(stdout))
    }

    fn parse_overview_records(records: &[serde_json::Value]) -> DockerOverview {
        let mut overview = DockerOverview::default();

        for val in records {
            let item_type = val.get("Type").and_then(|v| v.as_str()).unwrap_or("");
            let size = Self::json_size(val.get("Size"));
            let reclaim = Self::json_reclaimable(val.get("Reclaimable"));

            match item_type {
                "Images" => {
                    overview.images.total_bytes = size;
                    overview.images.reclaimable_bytes = reclaim;
                }
                "Containers" => {
                    overview.containers.total_bytes = size;
                    overview.containers.reclaimable_bytes = reclaim;
                }
                "Local Volumes" => {
                    overview.volumes.total_bytes = size;
                    overview.volumes.reclaimable_bytes = reclaim;
                }
                "Build Cache" => {
                    overview.build_cache.total_bytes = size;
                    overview.build_cache.reclaimable_bytes = reclaim;
                }
                _ => {}
            }
        }

        overview.total_bytes = overview.images.total_bytes
            + overview.containers.total_bytes
            + overview.volumes.total_bytes
            + overview.build_cache.total_bytes;
        overview.total_reclaimable_bytes = overview.images.reclaimable_bytes
            + overview.containers.reclaimable_bytes
            + overview.volumes.reclaimable_bytes
            + overview.build_cache.reclaimable_bytes;

        overview.safe_cleanable_bytes =
            overview.images.reclaimable_bytes + overview.build_cache.reclaimable_bytes;

        overview
    }

    /// Parses Docker human-readable sizes (e.g., "1.24GB", "500MB", "12.5kB") to bytes.
    pub fn parse_docker_size(size_str: &str) -> u64 {
        let s = size_str.trim();
        let (num_part, unit) = if let Some(idx) = s.find(|c: char| c.is_alphabetic()) {
            (&s[..idx], &s[idx..])
        } else {
            (s, "B")
        };

        let num: f64 = num_part.trim().parse().unwrap_or(0.0);
        let unit = unit.trim().to_uppercase();

        let multiplier = match unit.as_str() {
            "B" => 1.0,
            "KB" | "KIB" | "K" => 1024.0,
            "MB" | "MIB" | "M" => 1024.0 * 1024.0,
            "GB" | "GIB" | "G" => 1024.0 * 1024.0 * 1024.0,
            "TB" | "TIB" | "T" => 1024.0 * 1024.0 * 1024.0 * 1024.0,
            _ => 1.0,
        };

        (num * multiplier) as u64
    }

    fn parse_docker_reclaimable(reclaim_str: &str) -> u64 {
        // format is often "1.2GB (50%)"
        let part = reclaim_str.split('(').next().unwrap_or(reclaim_str).trim();
        Self::parse_docker_size(part)
    }

    pub fn get_images() -> Vec<DockerImageItem> {
        let cli = Self::preferred_cli();
        let containers = Self::get_containers_with(cli);
        Self::get_images_from_containers_with(cli, &containers)
    }

    pub fn get_images_from_containers(containers: &[DockerContainerItem]) -> Vec<DockerImageItem> {
        Self::get_images_from_containers_with(Self::preferred_cli(), containers)
    }

    fn get_images_from_containers_with(
        cli: ContainerCli,
        containers: &[DockerContainerItem],
    ) -> Vec<DockerImageItem> {
        let used_images: std::collections::HashSet<String> =
            containers.iter().map(|c| c.image.clone()).collect();

        let args: &[&str] = match cli {
            ContainerCli::Docker => &["images", "--format", "{{json .}}"],
            ContainerCli::Podman => &["images", "--format", "json"],
        };
        let records = Self::run_json_records(cli, args, 5);
        Self::parse_images_records(&records, &used_images)
    }

    pub fn parse_images(
        stdout: &str,
        used_images: &std::collections::HashSet<String>,
    ) -> Vec<DockerImageItem> {
        Self::parse_images_records(&Self::parse_json_records(stdout), used_images)
    }

    fn parse_images_records(
        records: &[serde_json::Value],
        used_images: &std::collections::HashSet<String>,
    ) -> Vec<DockerImageItem> {
        let mut images = Vec::new();
        for v in records {
            // Docker exposes ID/Repository/Tag; Podman exposes Id/Names.
            let id = v
                .get("ID")
                .or_else(|| v.get("Id"))
                .and_then(|s| s.as_str())
                .unwrap_or("")
                .to_string();
            let (repo, tag) = Self::image_repo_and_tag(v);
            let size_bytes = Self::json_size(v.get("Size"));
            let is_dangling = repo == "<none>" || tag == "<none>";

            let full_name = format!("{repo}:{tag}");
            let is_in_use = !is_dangling
                && (used_images.contains(&id)
                    || used_images.contains(&repo)
                    || used_images.contains(&full_name));

            images.push(DockerImageItem {
                id,
                repository: repo,
                tag,
                size_bytes,
                is_dangling,
                is_in_use,
            });
        }
        images
    }

    fn image_repo_and_tag(value: &serde_json::Value) -> (String, String) {
        let repo = value
            .get("Repository")
            .and_then(|s| s.as_str())
            .unwrap_or("");
        let tag = value.get("Tag").and_then(|s| s.as_str()).unwrap_or("");
        if !repo.is_empty() || !tag.is_empty() {
            return (repo.to_string(), tag.to_string());
        }
        // Podman: `Names` is an array such as ["docker.io/library/redis:alpine"].
        let full = value
            .get("Names")
            .and_then(|names| match names {
                serde_json::Value::Array(items) => items.first(),
                serde_json::Value::String(_) => Some(names),
                _ => None,
            })
            .and_then(|name| name.as_str())
            .unwrap_or("");
        match full.rsplit_once(':') {
            Some((repo, tag)) if !tag.contains('/') => (repo.to_string(), tag.to_string()),
            _ => (full.to_string(), String::new()),
        }
    }

    pub fn get_containers() -> Vec<DockerContainerItem> {
        Self::get_containers_with(Self::preferred_cli())
    }

    fn get_containers_with(cli: ContainerCli) -> Vec<DockerContainerItem> {
        let args: &[&str] = match cli {
            ContainerCli::Docker => &["ps", "-a", "--format", "{{json .}}"],
            ContainerCli::Podman => &["ps", "-a", "--format", "json"],
        };
        let records = Self::run_json_records(cli, args, 5);

        let mut containers = Vec::new();
        for v in &records {
            let id = v
                .get("ID")
                .or_else(|| v.get("Id"))
                .and_then(|s| s.as_str())
                .unwrap_or("")
                .to_string();
            let name = match v.get("Names") {
                Some(serde_json::Value::Array(items)) => items
                    .iter()
                    .filter_map(|item| item.as_str())
                    .collect::<Vec<_>>()
                    .join(","),
                Some(serde_json::Value::String(text)) => text.clone(),
                _ => String::new(),
            };
            let image = v
                .get("Image")
                .and_then(|s| s.as_str())
                .unwrap_or("")
                .to_string();
            let state = v
                .get("State")
                .and_then(|s| s.as_str())
                .unwrap_or("")
                .to_string();
            let size_bytes = Self::json_size(v.get("Size"));
            let is_running = state.eq_ignore_ascii_case("running");

            containers.push(DockerContainerItem {
                id,
                name,
                image,
                state,
                size_bytes,
                is_running,
            });
        }
        containers
    }

    pub fn get_volumes() -> Vec<DockerVolumeItem> {
        Self::get_volumes_with(Self::preferred_cli())
    }

    fn get_volumes_with(cli: ContainerCli) -> Vec<DockerVolumeItem> {
        let args: &[&str] = match cli {
            ContainerCli::Docker => &["volume", "ls", "--format", "{{json .}}"],
            ContainerCli::Podman => &["volume", "ls", "--format", "json"],
        };
        let records = Self::run_json_records(cli, args, 5);

        let mut volumes = Vec::new();
        for v in &records {
            let name = v
                .get("Name")
                .and_then(|s| s.as_str())
                .unwrap_or("")
                .to_string();
            let driver = v
                .get("Driver")
                .and_then(|s| s.as_str())
                .unwrap_or("local")
                .to_string();

            volumes.push(DockerVolumeItem {
                name,
                driver,
                size_bytes: 0,
                is_in_use: true,
            });
        }
        volumes
    }

    /// Executes targeted Docker prune actions for a given signature.
    pub fn prune_category(signature_id: &str) -> Result<u64, ZenithError> {
        let overview_before = Self::get_overview();

        enum CategoryDelta {
            Images,
            BuildCache,
            Containers,
            Volumes,
        }

        let prune_timeout = std::time::Duration::from_secs(30);
        let cli = Self::preferred_cli().executable();

        let (res, delta_kind) = match signature_id {
            "container.docker.dangling_images" => {
                let mut cmd = tooling::command(cli);
                cmd.args(["image", "prune", "-f"]);
                (
                    tooling::run_with_timeout(cmd, prune_timeout),
                    CategoryDelta::Images,
                )
            }
            "container.docker.unused_images" => {
                let mut cmd = tooling::command(cli);
                cmd.args(["image", "prune", "-a", "-f"]);
                (
                    tooling::run_with_timeout(cmd, prune_timeout),
                    CategoryDelta::Images,
                )
            }
            "container.docker.builder" => {
                let mut cmd = tooling::command(cli);
                cmd.args(["builder", "prune", "-f"]);
                (
                    tooling::run_with_timeout(cmd, prune_timeout),
                    CategoryDelta::BuildCache,
                )
            }
            "container.docker.stopped_containers" => {
                let mut cmd = tooling::command(cli);
                cmd.args(["container", "prune", "-f"]);
                (
                    tooling::run_with_timeout(cmd, prune_timeout),
                    CategoryDelta::Containers,
                )
            }
            "container.docker.unused_volumes" => {
                let mut cmd = tooling::command(cli);
                cmd.args(["volume", "prune", "-f"]);
                (
                    tooling::run_with_timeout(cmd, prune_timeout),
                    CategoryDelta::Volumes,
                )
            }
            _ => {
                return Err(ZenithError::SignatureMismatch(format!(
                    "Unknown docker signature: {}",
                    signature_id
                )))
            }
        };

        match res {
            Ok(output) if output.status.success() => {
                let overview_after = Self::get_overview();
                let reclaimed = match delta_kind {
                    CategoryDelta::Images => overview_before
                        .images
                        .total_bytes
                        .saturating_sub(overview_after.images.total_bytes),
                    CategoryDelta::BuildCache => overview_before
                        .build_cache
                        .total_bytes
                        .saturating_sub(overview_after.build_cache.total_bytes),
                    CategoryDelta::Containers => overview_before
                        .containers
                        .total_bytes
                        .saturating_sub(overview_after.containers.total_bytes),
                    CategoryDelta::Volumes => overview_before
                        .volumes
                        .total_bytes
                        .saturating_sub(overview_after.volumes.total_bytes),
                };
                Ok(reclaimed)
            }
            Ok(output) => {
                let err_str = String::from_utf8_lossy(&output.stderr).to_string();
                crate::diagnostics::log_error("docker", &err_str);
                Err(ZenithError::ExternalCommandFailed(err_str))
            }
            Err(e) => {
                let err_str = e.to_string();
                crate::diagnostics::log_error("docker", &err_str);
                Err(ZenithError::ExternalCommandFailed(err_str))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::DockerAdapter;

    #[test]
    fn volume_total_is_not_reported_as_reclaimable() {
        let output = r#"
{"Type":"Images","Size":"10GB","Reclaimable":"2GB (20%)"}
{"Type":"Containers","Size":"3GB","Reclaimable":"1GB (33%)"}
{"Type":"Local Volumes","Size":"8GB","Reclaimable":"500MB (6%)"}
{"Type":"Build Cache","Size":"4GB","Reclaimable":"3GB (75%)"}
"#;
        let overview = DockerAdapter::parse_overview(output);
        assert_eq!(overview.volumes.total_bytes, 8 * 1024 * 1024 * 1024);
        assert_eq!(overview.volumes.reclaimable_bytes, 500 * 1024 * 1024);
        assert!(overview.total_reclaimable_bytes < overview.total_bytes);
    }

    #[test]
    fn parse_images_detects_used_and_dangling() {
        use std::collections::HashSet;

        let output = r#"
{"ID":"img1","Repository":"redis","Tag":"alpine","Size":"30MB"}
{"ID":"img2","Repository":"nginx","Tag":"latest","Size":"100MB"}
{"ID":"img3","Repository":"<none>","Tag":"<none>","Size":"50MB"}
"#;
        let mut used = HashSet::new();
        used.insert("redis:alpine".to_string());

        let images = DockerAdapter::parse_images(output, &used);
        assert_eq!(images.len(), 3);

        // img1 is used by redis:alpine
        assert!(images[0].is_in_use);
        assert!(!images[0].is_dangling);

        // img2 is not in use
        assert!(!images[1].is_in_use);
        assert!(!images[1].is_dangling);

        // img3 is dangling
        assert!(!images[2].is_in_use);
        assert!(images[2].is_dangling);
    }

    #[test]
    fn selects_the_first_installed_cli_that_reaches_a_daemon() {
        use super::{select_running_cli, ContainerCli};

        let both = [ContainerCli::Docker, ContainerCli::Podman];
        assert_eq!(
            select_running_cli(&both, |_| true),
            Some(ContainerCli::Docker)
        );
        assert_eq!(
            select_running_cli(&both, |cli| cli == ContainerCli::Podman),
            Some(ContainerCli::Podman)
        );
        assert_eq!(select_running_cli(&both, |_| false), None);
        assert_eq!(
            select_running_cli(&[ContainerCli::Podman], |_| true),
            Some(ContainerCli::Podman)
        );
        assert_eq!(select_running_cli(&[], |_| true), None);
    }

    #[test]
    fn container_cli_detection_accepts_podman_only() {
        use super::container_cli_detected_with;

        assert!(!container_cli_detected_with(|_| false));
        assert!(container_cli_detected_with(|name| name == "docker"));
        assert!(container_cli_detected_with(|name| name == "podman"));
    }

    #[test]
    fn podman_json_array_output_parses_like_docker_lines() {
        let podman_overview = r#"[
            {"Type":"Images","Size":123456,"Reclaimable":2000},
            {"Type":"Containers","Size":5000,"Reclaimable":1000},
            {"Type":"Local Volumes","Size":10000,"Reclaimable":0},
            {"Type":"Build Cache","Size":4000,"Reclaimable":3000}
        ]"#;
        let overview = DockerAdapter::parse_overview(podman_overview);
        assert_eq!(overview.images.total_bytes, 123456);
        assert_eq!(overview.images.reclaimable_bytes, 2000);
        assert_eq!(overview.build_cache.reclaimable_bytes, 3000);
        assert_eq!(overview.total_reclaimable_bytes, 6000);
    }

    #[test]
    fn podman_image_names_array_is_parsed() {
        use std::collections::HashSet;

        let output =
            r#"[{"Id":"img1","Names":["docker.io/library/redis:alpine"],"Size":30000000}]"#;
        let images = DockerAdapter::parse_images(output, &HashSet::new());
        assert_eq!(images.len(), 1);
        assert_eq!(images[0].repository, "docker.io/library/redis");
        assert_eq!(images[0].tag, "alpine");
        assert_eq!(images[0].size_bytes, 30_000_000);
        assert!(!images[0].is_dangling);
    }
}
