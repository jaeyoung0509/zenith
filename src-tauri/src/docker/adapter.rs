use crate::models::{
    Category, DockerContainerItem, DockerImageItem, DockerOverview, DockerStatus, DockerVolumeItem,
    FileSize, RiskTier, ScanItem, ZenithError,
};
use crate::tooling;

pub struct DockerAdapter;

impl DockerAdapter {
    /// Checks if the Docker CLI is installed and the daemon is currently running.
    pub fn get_status() -> DockerStatus {
        let mut cli_cmd = tooling::command("docker");
        cli_cmd.arg("--version");
        let cli_check = tooling::run_with_timeout(cli_cmd, std::time::Duration::from_secs(3));

        let (is_available, version) = match cli_check {
            Ok(output) if output.status.success() => {
                let ver_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
                (true, Some(ver_str))
            }
            _ => (false, None),
        };

        if !is_available {
            return DockerStatus {
                is_available: false,
                is_running: false,
                version: None,
                error_message: Some("Docker CLI is not installed or not in PATH".to_string()),
                overview: None,
                images: Vec::new(),
                containers: Vec::new(),
                volumes: Vec::new(),
            };
        }

        // Check if Docker daemon is running
        let mut ping_cmd = tooling::command("docker");
        ping_cmd.args(["info", "--format", "{{.ServerVersion}}"]);
        let ping = tooling::run_with_timeout(ping_cmd, std::time::Duration::from_secs(4));

        let is_running = matches!(ping, Ok(output) if output.status.success());

        if !is_running {
            return DockerStatus {
                is_available: true,
                is_running: false,
                version,
                error_message: Some("Docker daemon is not running".to_string()),
                overview: None,
                images: Vec::new(),
                containers: Vec::new(),
                volumes: Vec::new(),
            };
        }

        let overview = Self::get_overview();
        let images = Self::get_images();
        let containers = Self::get_containers();
        let volumes = Self::get_volumes();

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
                is_selected: false,
                last_modified: None,
                exists: true,
            });
        }

        items
    }

    /// Queries `docker system df` and parses image, container, volume, and build cache usage.
    pub fn get_overview() -> DockerOverview {
        let mut cmd = tooling::command("docker");
        cmd.args(["system", "df", "--format", "{{json .}}"]);
        let output = tooling::run_with_timeout(cmd, std::time::Duration::from_secs(5));

        let stdout = match output {
            Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout).to_string(),
            _ => return DockerOverview::default(),
        };

        Self::parse_overview(&stdout)
    }

    fn parse_overview(stdout: &str) -> DockerOverview {
        let mut overview = DockerOverview::default();

        for line in stdout.lines() {
            let val: serde_json::Value = match serde_json::from_str(line) {
                Ok(v) => v,
                Err(_) => continue,
            };

            let item_type = val.get("Type").and_then(|v| v.as_str()).unwrap_or("");
            let size_str = val.get("Size").and_then(|v| v.as_str()).unwrap_or("0B");
            let reclaim_str = val
                .get("Reclaimable")
                .and_then(|v| v.as_str())
                .unwrap_or("0B");

            let size = Self::parse_docker_size(size_str);
            let reclaim = Self::parse_docker_reclaimable(reclaim_str);

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
        let used_images: std::collections::HashSet<String> = Self::get_containers()
            .into_iter()
            .map(|c| c.image)
            .collect();

        let mut cmd = tooling::command("docker");
        cmd.args(["images", "--format", "{{json .}}"]);
        let output = tooling::run_with_timeout(cmd, std::time::Duration::from_secs(5));

        let mut images = Vec::new();
        if let Ok(out) = output {
            if out.status.success() {
                for line in String::from_utf8_lossy(&out.stdout).lines() {
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
                        let id = v
                            .get("ID")
                            .and_then(|s| s.as_str())
                            .unwrap_or("")
                            .to_string();
                        let repo = v
                            .get("Repository")
                            .and_then(|s| s.as_str())
                            .unwrap_or("")
                            .to_string();
                        let tag = v
                            .get("Tag")
                            .and_then(|s| s.as_str())
                            .unwrap_or("")
                            .to_string();
                        let size_str = v.get("Size").and_then(|s| s.as_str()).unwrap_or("0B");
                        let size_bytes = Self::parse_docker_size(size_str);
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
                }
            }
        }
        images
    }

    pub fn get_containers() -> Vec<DockerContainerItem> {
        let mut cmd = tooling::command("docker");
        cmd.args(["ps", "-a", "--format", "{{json .}}"]);
        let output = tooling::run_with_timeout(cmd, std::time::Duration::from_secs(5));

        let mut containers = Vec::new();
        if let Ok(out) = output {
            if out.status.success() {
                for line in String::from_utf8_lossy(&out.stdout).lines() {
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
                        let id = v
                            .get("ID")
                            .and_then(|s| s.as_str())
                            .unwrap_or("")
                            .to_string();
                        let name = v
                            .get("Names")
                            .and_then(|s| s.as_str())
                            .unwrap_or("")
                            .to_string();
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
                        let size_str = v.get("Size").and_then(|s| s.as_str()).unwrap_or("0B");
                        let size_bytes = Self::parse_docker_size(size_str);
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
                }
            }
        }
        containers
    }

    pub fn get_volumes() -> Vec<DockerVolumeItem> {
        let mut cmd = tooling::command("docker");
        cmd.args(["volume", "ls", "--format", "{{json .}}"]);
        let output = tooling::run_with_timeout(cmd, std::time::Duration::from_secs(5));

        let mut volumes = Vec::new();
        if let Ok(out) = output {
            if out.status.success() {
                for line in String::from_utf8_lossy(&out.stdout).lines() {
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
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
                }
            }
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

        let (res, delta_kind) = match signature_id {
            "container.docker.dangling_images" => {
                let mut cmd = tooling::command("docker");
                cmd.args(["image", "prune", "-f"]);
                (
                    tooling::run_with_timeout(cmd, prune_timeout),
                    CategoryDelta::Images,
                )
            }
            "container.docker.unused_images" => {
                let mut cmd = tooling::command("docker");
                cmd.args(["image", "prune", "-a", "-f"]);
                (
                    tooling::run_with_timeout(cmd, prune_timeout),
                    CategoryDelta::Images,
                )
            }
            "container.docker.builder" => {
                let mut cmd = tooling::command("docker");
                cmd.args(["builder", "prune", "-f"]);
                (
                    tooling::run_with_timeout(cmd, prune_timeout),
                    CategoryDelta::BuildCache,
                )
            }
            "container.docker.stopped_containers" => {
                let mut cmd = tooling::command("docker");
                cmd.args(["container", "prune", "-f"]);
                (
                    tooling::run_with_timeout(cmd, prune_timeout),
                    CategoryDelta::Containers,
                )
            }
            "container.docker.unused_volumes" => {
                let mut cmd = tooling::command("docker");
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
}
