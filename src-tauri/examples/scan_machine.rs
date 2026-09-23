//! Explicit, read-only real-machine scan evidence. No cleanup plan or executor.
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::{Duration, Instant};
use zenith_lib::cleaner::{LifecycleProviderRegistry, OwnerProviderRegistry};
use zenith_lib::docker::{ContainerHost, DockerAdapter};
use zenith_lib::models::{CancellationProbe, Category, CleanStrategy, ScanEvent};
use zenith_lib::orbstack::OrbStackAdapter;
use zenith_lib::scanner::ScanEngine;
use zenith_lib::signatures::SignatureRegistry;
use zenith_platform::PlatformEnvironment;

struct Deadline(Instant);
impl CancellationProbe for Deadline {
    fn is_cancelled(&self) -> bool {
        self.0.elapsed() >= Duration::from_secs(60)
    }
}

struct ProviderDeadline {
    started: Instant,
    cancel: Arc<AtomicBool>,
}

impl CancellationProbe for ProviderDeadline {
    fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed) || self.started.elapsed() >= Duration::from_secs(60)
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: BTreeSet<_> = std::env::args().skip(1).collect();
    let allowed = [
        "--live-read-only",
        "--providers-read-only",
        "--providers-cancel-after-first-root",
        "--containers-read-only",
    ];
    if !args.contains("--live-read-only")
        || (args.contains("--providers-cancel-after-first-root")
            && !args.contains("--providers-read-only"))
        || args
            .iter()
            .any(|argument| !allowed.contains(&argument.as_str()))
    {
        return Err(
            "usage: cargo run -p zenith-desktop --example scan_machine -- --live-read-only [--providers-read-only [--providers-cancel-after-first-root]] [--containers-read-only]".into(),
        );
    }
    // This executable is its own composition root. All platform facts are
    // constructed once and passed through the ordinary catalog/scan pipeline.
    let environment = PlatformEnvironment::native();
    let embedded = SignatureRegistry::load_embedded_with(&environment)?;
    let mut registry = SignatureRegistry::new();
    // Fixed filesystem-only catalog coverage: no Docker/API/provider commands.
    // Missing platform-specific signatures remain explicit in the report.
    let requested = [
        "system.developer_temp",
        "system.intensive.user_app_caches",
        "system.brave.code_cache",
        "system.brave.http_cache",
        "system.cloudkit.cache",
        "system.intensive.containers_caches",
        "system.intensive.group_containers_caches",
        "system.app_support_cache_segments",
        "system.windows.temp_known",
        "system.windows.temp_other",
        "system.windows.roaming_cache_segments",
        "system.intensive.windows_user_caches",
        "system.intensive.windows_browser_caches",
        "system.intensive.windows_packages",
        "ai.cursor.renderer_cache",
    ];
    let mut included = Vec::new();
    let mut unavailable = Vec::new();
    for id in requested {
        if let Some(signature) = embedded.get(id).filter(|signature| {
            signature.platforms.is_empty() || signature.platforms.contains(&environment.platform())
        }) {
            if !matches!(
                signature.strategy,
                CleanStrategy::DeleteContents
                    | CleanStrategy::DeleteDirectory
                    | CleanStrategy::DeleteStaleContents
                    | CleanStrategy::Manual
            ) {
                return Err(format!("{id} is no longer a filesystem-only signature").into());
            }
            registry.register(signature.clone());
            included.push(id);
        } else {
            unavailable.push(id);
        }
    }
    let result = ScanEngine::scan(
        &registry,
        &LifecycleProviderRegistry::new(Vec::new()),
        &OwnerProviderRegistry::new(Vec::new()),
        Some(&[Category::Ai, Category::Developer, Category::System]),
        &[],
        true,
        &environment,
        &Deadline(Instant::now()),
        |_| {},
    );
    // Only aggregate counts and catalog IDs are serialized. Paths, names,
    // free-form errors, tokens, and provider output never enter this report.
    let categories: Vec<_> = result
        .categories
        .iter()
        .map(|category| {
            serde_json::json!({
                "category": category.category,
                "items": category.items.len(),
                "observed_bytes": category.total_bytes,
                "cleanable_bytes": category.cleanable_bytes,
                "skipped_entries": category.skipped_entry_count,
            })
        })
        .collect();
    let signature_results: Vec<_> = included
        .iter()
        .map(|id| {
            let items: Vec<_> = result
                .categories
                .iter()
                .flat_map(|category| &category.items)
                .filter(|item| item.signature_id == *id)
                .collect();
            serde_json::json!({
                "signature_id": id,
                "items": items.len(),
                "observed_bytes": items.iter().map(|item| item.size.observed_bytes()).sum::<u64>(),
                "cleanable_bytes": items.iter().map(|item| item.cleanable_bytes()).sum::<u64>(),
            })
        })
        .collect();
    let provider_scan = args
        .contains("--providers-read-only")
        .then(|| {
            scan_providers(
                &embedded,
                &environment,
                args.contains("--providers-cancel-after-first-root"),
            )
        })
        .transpose()?;
    let container_scan = args
        .contains("--containers-read-only")
        .then(|| scan_containers(&environment));
    let report = serde_json::json!({
        "schema": 2,
        "version": env!("CARGO_PKG_VERSION"),
        "platform": environment.platform(),
        "started_at": result.started_at,
        "scope": "selected filesystem signatures; intensive observation enabled; no cleanup",
        "included_signatures": included,
        "unavailable_signatures": unavailable,
        "metrics": result.metrics,
        "categories": categories,
        "signature_results": signature_results,
        "observed_bytes": result.total_bytes,
        "cleanable_bytes": result.cleanable_bytes,
        "quality": result.quality,
        "cancelled": result.cancelled,
        "skipped_entries": result.skipped_entry_count,
        "incomplete_items": result.incomplete_item_count,
        "provider_scan": provider_scan,
        "container_scan": container_scan,
    });
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

fn scan_providers(
    embedded: &SignatureRegistry,
    environment: &PlatformEnvironment,
    cancel_after_first_root: bool,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    const PROVIDERS: [(&str, &str); 4] = [
        ("dev.uv.cache", "uv"),
        ("dev.pnpm.store", "pnpm"),
        ("dev.npm.cache", "npm"),
        ("dev.pip.cache", "pip3"),
    ];

    let mut registry = SignatureRegistry::new();
    for (signature_id, _) in PROVIDERS {
        let signature = embedded
            .get(signature_id)
            .ok_or_else(|| format!("missing provider signature {signature_id}"))?;
        if signature.strategy != CleanStrategy::ExternalCommand {
            return Err(format!("{signature_id} is no longer a provider signature").into());
        }
        registry.register(signature.clone());
    }

    let mut root_events = BTreeMap::<String, u64>::new();
    let cancel = Arc::new(AtomicBool::new(false));
    let cancellation = ProviderDeadline {
        started: Instant::now(),
        cancel: Arc::clone(&cancel),
    };
    let result = ScanEngine::scan(
        &registry,
        &LifecycleProviderRegistry::new(Vec::new()),
        &OwnerProviderRegistry::new(Vec::new()),
        Some(&[Category::Developer]),
        &[],
        false,
        environment,
        &cancellation,
        |event| {
            if let ScanEvent::RootStarted { signature_id, .. } = event {
                *root_events.entry(signature_id).or_default() += 1;
                if cancel_after_first_root {
                    cancel.store(true, Ordering::Relaxed);
                }
            }
        },
    );

    let providers: Vec<_> = PROVIDERS
        .iter()
        .map(|(signature_id, executable)| {
            let items: Vec<_> = result
                .categories
                .iter()
                .flat_map(|category| &category.items)
                .filter(|item| item.signature_id == *signature_id)
                .collect();
            serde_json::json!({
                "signature_id": signature_id,
                "tool_detected": zenith_lib::tooling::resolve_with(executable, environment).is_some(),
                "root_events": root_events.get(*signature_id).copied().unwrap_or_default(),
                "items": items.len(),
                "observed_bytes": items.iter().map(|item| item.size.observed_bytes()).sum::<u64>(),
                "cleanable_bytes": items.iter().map(|item| item.cleanable_bytes()).sum::<u64>(),
                "quality": items.first().map(|item| item.quality),
            })
        })
        .collect();

    Ok(serde_json::json!({
        "scope": "fixed-argument cache-directory discovery and measurement; no prune command",
        "cancellation_trigger": cancel_after_first_root.then_some("after_first_root_started"),
        "providers": providers,
        "metrics": result.metrics,
        "quality": result.quality,
        "gaps": result.gaps,
        "cancelled": result.cancelled,
        "skipped_entries": result.skipped_entry_count,
        "incomplete_items": result.incomplete_item_count,
    }))
}

fn scan_containers(environment: &PlatformEnvironment) -> serde_json::Value {
    let docker_host = std::env::var("DOCKER_HOST").ok();
    let host_was_stated = docker_host
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty());
    let status = DockerAdapter::get_status(environment, &ContainerHost::from_value(docker_host));
    let overview = status.overview.as_ref().map(|overview| {
        serde_json::json!({
            "images": overview.images,
            "containers": overview.containers,
            "volumes": overview.volumes,
            "build_cache": overview.build_cache,
            "total_bytes": overview.total_bytes,
            "total_reclaimable_bytes": overview.total_reclaimable_bytes,
            "safe_cleanable_bytes": overview.safe_cleanable_bytes,
        })
    });
    let orbstack_items = OrbStackAdapter::scan_items(environment);

    serde_json::json!({
        "scope": "fixed-argument runtime inspection and local metadata; no prune or delete command",
        "docker": {
            "host_was_stated": host_was_stated,
            "cli_available": status.is_available,
            "daemon_running": status.is_running,
            "overview": overview,
            "image_count": status.images.len(),
            "container_count": status.containers.len(),
            "volume_count": status.volumes.len(),
        },
        "orbstack": {
            "items": orbstack_items.len(),
            "observed_bytes": orbstack_items.iter().map(|item| item.size.observed_bytes()).sum::<u64>(),
            "cleanable_bytes": orbstack_items.iter().map(|item| item.cleanable_bytes()).sum::<u64>(),
        },
    })
}
