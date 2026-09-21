//! Explicit, read-only real-machine scan evidence. No cleanup plan or executor.
use std::time::{Duration, Instant};
use zenith_lib::cleaner::{LifecycleProviderRegistry, OwnerProviderRegistry};
use zenith_lib::models::{CancellationProbe, Category, CleanStrategy};
use zenith_lib::scanner::ScanEngine;
use zenith_lib::signatures::SignatureRegistry;
use zenith_platform::PlatformEnvironment;

struct Deadline(Instant);
impl CancellationProbe for Deadline {
    fn is_cancelled(&self) -> bool {
        self.0.elapsed() >= Duration::from_secs(60)
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::args().nth(1).as_deref() != Some("--live-read-only") {
        return Err(
            "usage: cargo run -p zenith-desktop --example scan_machine -- --live-read-only".into(),
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
        "system.intensive.containers_caches",
        "system.intensive.group_containers_caches",
        "system.app_support_cache_segments",
        "system.windows.temp_known",
        "system.windows.temp_other",
        "system.windows.roaming_cache_segments",
        "system.intensive.windows_user_caches",
        "system.intensive.windows_browser_caches",
        "system.intensive.windows_packages",
        "ai.cursor.cache",
        "dev.go.build",
        "dev.go.mod",
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
    let report = serde_json::json!({
        "schema": 1,
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
    });
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
