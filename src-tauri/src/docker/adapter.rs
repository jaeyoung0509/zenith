use crate::models::{
    Category, DockerContainerItem, DockerImageItem, DockerOverview, DockerStatus, DockerVolumeItem,
    FileSize, RiskTier, ScanItem, ZenithError,
};
use crate::tooling;

pub struct DockerAdapter;

impl DockerAdapter {
    /// Checks if the Docker CLI is installed and the daemon is currently running.
    pub fn get_status() -> DockerStatus {
        let cli_check = tooling::command("docker").arg("--version").output();

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
        let ping = tooling::command("docker")
            .args(["info", "--format", "{{.ServerVersion}}"])
            .output();

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
        let status = Self::get_status();
        if !status.is_running {
            return Vec::new();
        }

        let overview = match status.overview {
            Some(o) => o,
            None => return Vec::new(),
        };

        let mut items = Vec::new();

        if overview.images.reclaimable_bytes > 0 {
            items.push(ScanItem {
                id: "container.docker.dangling_images".to_string(),
                signature_id: "container.docker.dangling_images".to_string(),
                name: "Docker Dangling Images".to_string(),
                category: Category::Container,
                risk: RiskTier::Safe,
                path: "docker://images/dangling".to_string(),
                size: FileSize::new(
                    overview.images.reclaimable_bytes,
                    Some(overview.images.reclaimable_bytes),
                ),
                file_count: 0,
                description: "Untagged intermediate image layers".to_string(),
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
        let output = tooling::command("docker")
            .args(["system", "df", "--format", "{{json .}}"])
            .output();

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

            let total_bytes = Self::parse_docker_size(size_str);
            let reclaim_bytes = Self::parse_docker_reclaimable(reclaim_str);

            match item_type {
                "Images" => {
                    overview.images.total_bytes = total_bytes;
                    overview.images.reclaimable_bytes = reclaim_bytes;
                }
                "Containers" => {
                    overview.containers.total_bytes = total_bytes;
                    overview.containers.reclaimable_bytes = reclaim_bytes;
                }
                "Local Volumes" => {
                    overview.volumes.total_bytes = total_bytes;
                    overview.volumes.reclaimable_bytes = reclaim_bytes;
                }
                "Build Cache" => {
                    overview.build_cache.total_bytes = total_bytes;
                    overview.build_cache.reclaimable_bytes = reclaim_bytes;
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
        let output = tooling::command("docker")
            .args(["images", "--format", "{{json .}}"])
            .output();

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

                        images.push(DockerImageItem {
                            id,
                            repository: repo,
                            tag,
                            size_bytes,
                            is_dangling,
                            is_in_use: !is_dangling,
                        });
                    }
                }
            }
        }
        images
    }

    pub fn get_containers() -> Vec<DockerContainerItem> {
        let output = tooling::command("docker")
            .args(["ps", "-a", "--format", "{{json .}}"])
            .output();

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
        let output = tooling::command("docker")
            .args(["volume", "ls", "--format", "{{json .}}"])
            .output();

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

        let res = match signature_id {
            "container.docker.dangling_images" => tooling::command("docker")
                .args(["image", "prune", "-f"])
                .output(),
            "container.docker.builder" => tooling::command("docker")
                .args(["builder", "prune", "-f"])
                .output(),
            "container.docker.stopped_containers" => tooling::command("docker")
                .args(["container", "prune", "-f"])
                .output(),
            "container.docker.unused_volumes" => tooling::command("docker")
                .args(["volume", "prune", "-f"])
                .output(),
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
                let reclaimed = overview_before
                    .total_bytes
                    .saturating_sub(overview_after.total_bytes);
                Ok(reclaimed)
            }
            Ok(output) => {
                let err_str = String::from_utf8_lossy(&output.stderr);
                Err(ZenithError::ExternalCommandFailed(err_str.to_string()))
            }
            Err(e) => Err(ZenithError::ExternalCommandFailed(e.to_string())),
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
