import { invoke } from '@tauri-apps/api/core';
import { Channel } from '@tauri-apps/api/core';
import type {
  AwakeBehavior,
  AwakeRule,
  AwakeState,
  Category,
  CleanEvent,
  CleanResult,
  DeletePlan,
  DiskMetrics,
  DockerStatus,
  LocalModelItem,
  MemoryMetrics,
  ScanEvent,
  ScanItem,
  ScanResult,
  ZenithSettings,
} from '../models/types';

export const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

export async function tauriScan(
  onEvent: (event: ScanEvent) => void,
  categories?: Category[]
): Promise<ScanResult> {
  if (!isTauri) {
    // Mock for browser testing
    return mockScan(onEvent);
  }

  const channel = new Channel<ScanEvent>();
  channel.onmessage = (event) => {
    onEvent(event);
  };

  return await invoke<ScanResult>('start_scan', {
    onEvent: channel,
    categories: categories || null,
  });
}

export async function tauriGetLastScan(): Promise<ScanResult | null> {
  if (!isTauri) return null;
  return await invoke<ScanResult | null>('get_last_scan');
}

export async function tauriCreatePlan(items: ScanItem[]): Promise<DeletePlan> {
  if (!isTauri) {
    return {
      id: 'mock-plan-1',
      targets: items.map((i) => ({
        item_id: i.id,
        signature_id: i.signature_id,
        name: i.name,
        path: i.path,
        strategy: 'delete_contents',
        expected_bytes: i.size.allocated ?? i.size.logical,
        risk: i.risk,
      })),
      expected_reclaim_bytes: items.reduce(
        (acc, i) => acc + (i.size.allocated ?? i.size.logical),
        0
      ),
      risk: {
        safe_count: items.filter((i) => i.risk === 'safe').length,
        rebuild_count: items.filter((i) => i.risk === 'rebuild').length,
        manual_count: items.filter((i) => i.risk === 'manual').length,
        safe_bytes: items
          .filter((i) => i.risk === 'safe')
          .reduce((acc, i) => acc + (i.size.allocated ?? i.size.logical), 0),
        rebuild_bytes: items
          .filter((i) => i.risk === 'rebuild')
          .reduce((acc, i) => acc + (i.size.allocated ?? i.size.logical), 0),
        manual_bytes: items
          .filter((i) => i.risk === 'manual')
          .reduce((acc, i) => acc + (i.size.allocated ?? i.size.logical), 0),
      },
      created_at: Math.floor(Date.now() / 1000),
    };
  }

  return await invoke<DeletePlan>('create_delete_plan', { items });
}

export async function tauriExecuteClean(
  plan: DeletePlan,
  onEvent: (event: CleanEvent) => void
): Promise<CleanResult> {
  if (!isTauri) {
    return mockClean(plan, onEvent);
  }

  const channel = new Channel<CleanEvent>();
  channel.onmessage = (event) => {
    onEvent(event);
  };

  return await invoke<CleanResult>('execute_clean', {
    plan,
    onEvent: channel,
  });
}

export async function tauriGetMemoryMetrics(): Promise<MemoryMetrics> {
  if (!isTauri) {
    return {
      total_bytes: 16 * 1024 * 1024 * 1024,
      used_bytes: 11.8 * 1024 * 1024 * 1024,
      available_bytes: 4.2 * 1024 * 1024 * 1024,
      free_bytes: 1.2 * 1024 * 1024 * 1024,
      compressed_bytes: 2.1 * 1024 * 1024 * 1024,
      swap_used_bytes: 650 * 1024 * 1024,
      swap_total_bytes: 2 * 1024 * 1024 * 1024,
      pressure: 'normal',
      top_processes: [
        { pid: 101, name: 'Cursor', memory_bytes: 2.8 * 1024 * 1024 * 1024, process_count: 14 },
        { pid: 102, name: 'Google Chrome', memory_bytes: 2.2 * 1024 * 1024 * 1024, process_count: 22 },
        { pid: 103, name: 'Docker Desktop', memory_bytes: 1.6 * 1024 * 1024 * 1024, process_count: 4 },
        { pid: 104, name: 'Claude', memory_bytes: 840 * 1024 * 1024, process_count: 2 },
        { pid: 105, name: 'Xcode', memory_bytes: 1.4 * 1024 * 1024 * 1024, process_count: 6 },
      ],
      timestamp: Math.floor(Date.now() / 1000),
    };
  }

  return await invoke<MemoryMetrics>('get_memory_metrics');
}

export async function tauriGetDiskMetrics(): Promise<DiskMetrics> {
  if (!isTauri) {
    return {
      mount_point: '/',
      total_bytes: 494 * 1024 * 1024 * 1024,
      used_bytes: 341 * 1024 * 1024 * 1024,
      free_bytes: 153 * 1024 * 1024 * 1024,
      available_bytes: 153 * 1024 * 1024 * 1024,
      percent_used: 69.0,
    };
  }

  return await invoke<DiskMetrics>('get_disk_metrics');
}

export async function tauriGetDockerStatus(): Promise<DockerStatus> {
  if (!isTauri) {
    return {
      is_available: true,
      is_running: true,
      version: 'Docker version 27.0.3',
      overview: {
        images_bytes: 7.2 * 1024 * 1024 * 1024,
        dangling_images_bytes: 2.1 * 1024 * 1024 * 1024,
        build_cache_bytes: 8.1 * 1024 * 1024 * 1024,
        stopped_containers_bytes: 1.4 * 1024 * 1024 * 1024,
        volumes_bytes: 1.6 * 1024 * 1024 * 1024,
        total_bytes: 18.3 * 1024 * 1024 * 1024,
        safe_cleanable_bytes: 10.2 * 1024 * 1024 * 1024,
      },
      images: [],
      containers: [],
      volumes: [],
    };
  }

  return await invoke<DockerStatus>('get_docker_status');
}

export async function tauriPruneDocker(signatureId: string): Promise<number> {
  if (!isTauri) return 1024 * 1024 * 1024;
  return await invoke<number>('prune_docker_target', { signatureId });
}

export async function tauriGetLocalModels(): Promise<LocalModelItem[]> {
  if (!isTauri) {
    return [
      {
        id: 'ollama.llama3:70b',
        name: 'llama3:70b',
        source: 'ollama',
        path: '~/.ollama/models/manifests/registry.ollama.ai/library/llama3/70b',
        size_bytes: 18.2 * 1024 * 1024 * 1024,
        format: 'GGUF',
        last_modified: Math.floor(Date.now() / 1000) - 86400 * 3,
      },
      {
        id: 'ollama.qwen2.5-coder:32b',
        name: 'qwen2.5-coder:32b',
        source: 'ollama',
        path: '~/.ollama/models/manifests/registry.ollama.ai/library/qwen2.5-coder/32b',
        size_bytes: 9.8 * 1024 * 1024 * 1024,
        format: 'GGUF',
        last_modified: Math.floor(Date.now() / 1000) - 86400 * 1,
      },
      {
        id: 'hf.meta-llama/Llama-3.2-3B',
        name: 'meta-llama/Llama-3.2-3B',
        source: 'huggingface',
        path: '~/.cache/huggingface/hub/models--meta-llama--Llama-3.2-3B',
        size_bytes: 4.2 * 1024 * 1024 * 1024,
        format: 'safetensors',
        last_modified: Math.floor(Date.now() / 1000) - 86400 * 5,
      },
    ];
  }

  return await invoke<LocalModelItem[]>('get_local_models');
}

export async function tauriDeleteLocalModel(path: string): Promise<number> {
  if (!isTauri) return 4.2 * 1024 * 1024 * 1024;
  return await invoke<number>('delete_local_model', { path });
}

export async function tauriGetAwakeState(): Promise<AwakeState> {
  if (!isTauri) {
    return {
      is_active: false,
      active_rules_count: 1,
    };
  }

  return await invoke<AwakeState>('get_awake_state');
}

export async function tauriSetAwakeRules(rules: AwakeRule[]): Promise<void> {
  if (!isTauri) return;
  await invoke('set_awake_rules', { rules });
}

export async function tauriSetManualAwake(
  durationSecs: number | null,
  behavior: AwakeBehavior
): Promise<void> {
  if (!isTauri) return;
  await invoke('set_manual_awake', { durationSecs, behavior });
}

export async function tauriDisableManualAwake(): Promise<void> {
  if (!isTauri) return;
  await invoke('disable_manual_awake');
}

export async function tauriGetSettings(): Promise<ZenithSettings> {
  if (!isTauri) {
    return {
      launch_at_login: false,
      clean_ai_tools: true,
      clean_developer_tools: true,
      clean_docker: true,
      clean_local_models: false,
      include_rebuild_caches: false,
      theme: 'system',
      excluded_signatures: [],
      awake_rules: [
        {
          id: 'rule.claude',
          app_name: 'Claude Code',
          executable_pattern: 'claude',
          behavior: 'prevent_system_sleep',
          enabled: true,
        },
      ],
    };
  }

  return await invoke<ZenithSettings>('get_settings');
}

export async function tauriSaveSettings(settings: ZenithSettings): Promise<void> {
  if (!isTauri) return;
  await invoke('save_settings', { settings });
}

export async function tauriRevealInFinder(path: string): Promise<void> {
  if (!isTauri) return;
  await invoke('reveal_in_finder', { path });
}

export async function tauriOpenDashboard(): Promise<void> {
  if (!isTauri) {
    window.location.hash = '#dashboard';
    return;
  }
  await invoke('open_dashboard_window');
}

export async function tauriToggleQuick(): Promise<void> {
  if (!isTauri) return;
  await invoke('toggle_quick_panel');
}

// Fallback browser mock helpers
function mockScan(onEvent: (event: ScanEvent) => void): Promise<ScanResult> {
  return new Promise((resolve) => {
    const scanId = 'mock-scan-' + Date.now();
    onEvent({ type: 'Started', scan_id: scanId });

    setTimeout(() => {
      onEvent({ type: 'CategoryStarted', category: 'ai' });
      onEvent({
        type: 'ItemFound',
        item: {
          id: 'ai.cursor.cache',
          signature_id: 'ai.cursor.cache',
          name: 'Cursor Editor Cache',
          category: 'ai',
          risk: 'safe',
          path: '~/Library/Caches/Cursor',
          size: { logical: 2.1 * 1024 * 1024 * 1024, allocated: 2.1 * 1024 * 1024 * 1024 },
          file_count: 3200,
          description: 'V8 code cache and GPU shader cache',
          is_selected: true,
          exists: true,
        },
      });
      onEvent({
        type: 'ItemFound',
        item: {
          id: 'ai.claude.logs',
          signature_id: 'ai.claude.logs',
          name: 'Claude Code Logs',
          category: 'ai',
          risk: 'safe',
          path: '~/.claude/logs',
          size: { logical: 1.1 * 1024 * 1024 * 1024, allocated: 1.1 * 1024 * 1024 * 1024 },
          file_count: 140,
          description: 'Session diagnostic logs',
          is_selected: true,
          exists: true,
        },
      });
      onEvent({ type: 'CategoryFinished', category: 'ai', bytes: 3.2 * 1024 * 1024 * 1024, item_count: 2 });
    }, 150);

    setTimeout(() => {
      onEvent({ type: 'CategoryStarted', category: 'developer' });
      onEvent({
        type: 'ItemFound',
        item: {
          id: 'dev.go.build',
          signature_id: 'dev.go.build',
          name: 'Go Build Cache',
          category: 'developer',
          risk: 'safe',
          path: '~/Library/Caches/go-build',
          size: { logical: 3.1 * 1024 * 1024 * 1024, allocated: 3.1 * 1024 * 1024 * 1024 },
          file_count: 12000,
          description: 'Compiled packages cache',
          is_selected: true,
          exists: true,
        },
      });
      onEvent({
        type: 'ItemFound',
        item: {
          id: 'dev.cargo.registry.cache',
          signature_id: 'dev.cargo.registry.cache',
          name: 'Cargo Registry Cache',
          category: 'developer',
          risk: 'rebuild',
          path: '~/.cargo/registry/cache',
          size: { logical: 2.0 * 1024 * 1024 * 1024, allocated: 2.0 * 1024 * 1024 * 1024 },
          file_count: 850,
          description: 'Downloaded crates archive',
          is_selected: false,
          exists: true,
        },
      });
      onEvent({ type: 'CategoryFinished', category: 'developer', bytes: 5.1 * 1024 * 1024 * 1024, item_count: 2 });
    }, 300);

    setTimeout(() => {
      const result: ScanResult = {
        scan_id: scanId,
        started_at: Math.floor(Date.now() / 1000) - 1,
        finished_at: Math.floor(Date.now() / 1000),
        categories: [
          {
            category: 'ai',
            display_name: 'AI Tools',
            items: [
              {
                id: 'ai.cursor.cache',
                signature_id: 'ai.cursor.cache',
                name: 'Cursor Editor Cache',
                category: 'ai',
                risk: 'safe',
                path: '~/Library/Caches/Cursor',
                size: { logical: 2.1 * 1024 * 1024 * 1024, allocated: 2.1 * 1024 * 1024 * 1024 },
                file_count: 3200,
                description: 'V8 code cache and GPU shader cache',
                is_selected: true,
                exists: true,
              },
              {
                id: 'ai.claude.logs',
                signature_id: 'ai.claude.logs',
                name: 'Claude Code Logs',
                category: 'ai',
                risk: 'safe',
                path: '~/.claude/logs',
                size: { logical: 1.1 * 1024 * 1024 * 1024, allocated: 1.1 * 1024 * 1024 * 1024 },
                file_count: 140,
                description: 'Session diagnostic logs',
                is_selected: true,
                exists: true,
              },
            ],
            total_bytes: 3.2 * 1024 * 1024 * 1024,
            safe_bytes: 3.2 * 1024 * 1024 * 1024,
            rebuild_bytes: 0,
            manual_bytes: 0,
          },
          {
            category: 'developer',
            display_name: 'Developer',
            items: [
              {
                id: 'dev.go.build',
                signature_id: 'dev.go.build',
                name: 'Go Build Cache',
                category: 'developer',
                risk: 'safe',
                path: '~/Library/Caches/go-build',
                size: { logical: 3.1 * 1024 * 1024 * 1024, allocated: 3.1 * 1024 * 1024 * 1024 },
                file_count: 12000,
                description: 'Compiled packages cache',
                is_selected: true,
                exists: true,
              },
              {
                id: 'dev.cargo.registry.cache',
                signature_id: 'dev.cargo.registry.cache',
                name: 'Cargo Registry Cache',
                category: 'developer',
                risk: 'rebuild',
                path: '~/.cargo/registry/cache',
                size: { logical: 2.0 * 1024 * 1024 * 1024, allocated: 2.0 * 1024 * 1024 * 1024 },
                file_count: 850,
                description: 'Downloaded crates archive',
                is_selected: false,
                exists: true,
              },
            ],
            total_bytes: 5.1 * 1024 * 1024 * 1024,
            safe_bytes: 3.1 * 1024 * 1024 * 1024,
            rebuild_bytes: 2.0 * 1024 * 1024 * 1024,
            manual_bytes: 0,
          },
        ],
        total_bytes: 8.3 * 1024 * 1024 * 1024,
        safe_bytes: 6.3 * 1024 * 1024 * 1024,
        rebuild_bytes: 2.0 * 1024 * 1024 * 1024,
        manual_bytes: 0,
      };

      onEvent({ type: 'Finished', result });
      resolve(result);
    }, 450);
  });
}

function mockClean(
  plan: DeletePlan,
  onEvent: (event: CleanEvent) => void
): Promise<CleanResult> {
  return new Promise((resolve) => {
    onEvent({
      type: 'Started',
      plan_id: plan.id,
      total_targets: plan.targets.length,
      expected_bytes: plan.expected_reclaim_bytes,
    });

    const items: any[] = [];
    plan.targets.forEach((t, i) => {
      setTimeout(() => {
        onEvent({
          type: 'ItemStarted',
          item_id: t.item_id,
          name: t.name,
          index: i + 1,
          total: plan.targets.length,
        });

        setTimeout(() => {
          onEvent({
            type: 'ItemFinished',
            item_id: t.item_id,
            name: t.name,
            success: true,
            reclaimed_bytes: t.expected_bytes,
          });
          items.push({
            item_id: t.item_id,
            name: t.name,
            path: t.path,
            success: true,
            bytes_reclaimed: t.expected_bytes,
          });

          if (items.length === plan.targets.length) {
            const res: CleanResult = {
              plan_id: plan.id,
              started_at: plan.created_at,
              finished_at: Math.floor(Date.now() / 1000),
              total_reclaimed_bytes: plan.expected_reclaim_bytes,
              total_failed_bytes: 0,
              items,
              actual_disk_free_delta: plan.expected_reclaim_bytes,
            };
            onEvent({ type: 'Finished', result: res });
            resolve(res);
          }
        }, 100);
      }, i * 200);
    });
  });
}
