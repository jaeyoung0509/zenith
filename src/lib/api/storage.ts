import { Channel } from '@tauri-apps/api/core';
import { commands } from '../bindings/tauri';
import type {
  AppUninstallInspection,
  InstalledApp,
  LargeFileItem,
  LargeFileScanEvent,
  LargeFileScanRequest,
  LargeFileScanResult,
  TrashPlanPreview,
  TrashResult,
} from '../models/types';

type CommandResult<T, E> = { status: 'ok'; data: T } | { status: 'error'; error: E };

async function unwrap<T, E>(promise: Promise<CommandResult<T, E>>): Promise<T> {
  const result = await promise;
  if (result.status === 'error') {
    throw new Error(String(result.error));
  }
  return result.data;
}

export interface StorageManagementApi {
  startLargeFileScan(
    request: LargeFileScanRequest,
    onEvent: (event: LargeFileScanEvent) => void
  ): Promise<LargeFileScanResult>;
  cancelLargeFileScan(scanId: string): Promise<void>;
  prepareLargeFileTrash(scanId: string, selectedItemIds: string[]): Promise<TrashPlanPreview>;
  getInstalledApps(): Promise<InstalledApp[]>;
  inspectAppUninstall(appId: string): Promise<AppUninstallInspection>;
  prepareAppUninstall(
    inspectionId: string,
    selectedRelatedIds: string[]
  ): Promise<TrashPlanPreview>;
  executeTrashPlan(planId: string): Promise<TrashResult>;
}

const nativeStorageApi: StorageManagementApi = {
  async startLargeFileScan(request, onEvent) {
    const channel = new Channel<LargeFileScanEvent>();
    channel.onmessage = onEvent;
    return await unwrap(commands.startLargeFileScan(request, channel));
  },

  async cancelLargeFileScan(scanId) {
    await unwrap(commands.cancelLargeFileScan(scanId));
  },

  async prepareLargeFileTrash(scanId, selectedItemIds) {
    return await unwrap(commands.prepareLargeFileTrash(scanId, selectedItemIds));
  },

  async getInstalledApps() {
    return await unwrap(commands.getInstalledApps());
  },

  async inspectAppUninstall(appId) {
    return await unwrap(commands.inspectAppUninstall(appId));
  },

  async prepareAppUninstall(inspectionId, selectedRelatedIds) {
    return await unwrap(commands.prepareAppUninstall(inspectionId, selectedRelatedIds));
  },

  async executeTrashPlan(planId) {
    return await unwrap(commands.executeTrashPlan(planId));
  },
};

const GIB = 1024 * 1024 * 1024;
const MIB = 1024 * 1024;

const mockLargeFiles: LargeFileItem[] = [
  {
    id: 'large-video-1',
    name: 'screen-recording.mov',
    display_parent: '/Users/mock/Downloads',
    logical_size: 4.8 * GIB,
    allocated_size: 4.8 * GIB,
    modified_at: Math.floor(Date.now() / 1000) - 86400 * 18,
    kind: 'video',
    extension: 'mov',
  },
  {
    id: 'large-model-1',
    name: 'qwen-coder-q4.gguf',
    display_parent: '/Users/mock/Documents/models',
    logical_size: 3.1 * GIB,
    allocated_size: 3.1 * GIB,
    modified_at: Math.floor(Date.now() / 1000) - 86400 * 8,
    kind: 'ai_model',
    extension: 'gguf',
  },
  {
    id: 'large-archive-1',
    name: 'project-backup.zip',
    display_parent: '/Users/mock/Desktop',
    logical_size: 760 * MIB,
    allocated_size: 760 * MIB,
    modified_at: Math.floor(Date.now() / 1000) - 86400 * 43,
    kind: 'archive',
    extension: 'zip',
  },
];

const mockApps: InstalledApp[] = [
  {
    id: 'app-vscode',
    name: 'Visual Studio Code',
    bundle_id: 'com.microsoft.VSCode',
    version: '1.104.0',
    display_path: '/Applications/Visual Studio Code.app',
    executable_name: 'Electron',
    logical_size: 612 * MIB,
    allocated_size: 618 * MIB,
    modified_at: Math.floor(Date.now() / 1000) - 86400 * 5,
    install_source: 'application_bundle',
    is_running: false,
    is_system_protected: false,
  },
  {
    id: 'app-docker',
    name: 'Docker',
    bundle_id: 'com.docker.docker',
    version: '4.44.0',
    display_path: '/Applications/Docker.app',
    executable_name: 'Docker',
    logical_size: 1.4 * GIB,
    allocated_size: 1.4 * GIB,
    modified_at: Math.floor(Date.now() / 1000) - 86400 * 2,
    install_source: 'application_bundle',
    is_running: true,
    is_system_protected: false,
  },
  {
    id: 'app-obsidian',
    name: 'Obsidian',
    bundle_id: 'md.obsidian',
    version: '1.8.10',
    display_path: '/Applications/Obsidian.app',
    executable_name: 'Obsidian',
    logical_size: 278 * MIB,
    allocated_size: 282 * MIB,
    modified_at: Math.floor(Date.now() / 1000) - 86400 * 31,
    install_source: 'application_bundle',
    is_running: false,
    is_system_protected: false,
  },
];

interface MockTrashPlan {
  preview: TrashPlanPreview;
  itemIds: string[];
}

let activeMockScanId: string | null = null;
let mockScanCancelled = false;
const mockScans = new Map<string, LargeFileScanResult>();
const mockInspections = new Map<string, AppUninstallInspection>();
const mockPlans = new Map<string, MockTrashPlan>();

function inspectionFor(app: InstalledApp): AppUninstallInspection {
  const inspectionId = `mock-inspection-${app.id}-${Date.now()}`;
  const bundleId = app.bundle_id ?? app.name.toLowerCase().replaceAll(' ', '-');
  const inspection: AppUninstallInspection = {
    inspection_id: inspectionId,
    app,
    related_items: [
      {
        id: `${app.id}-support`,
        name: bundleId,
        display_path: `/Users/mock/Library/Application Support/${bundleId}`,
        kind: 'application_support',
        confidence: 'high',
        evidence: 'Exact CFBundleIdentifier match',
        logical_size: 420 * MIB,
        allocated_size: 424 * MIB,
        selected_by_default: true,
      },
      {
        id: `${app.id}-cache`,
        name: bundleId,
        display_path: `/Users/mock/Library/Caches/${bundleId}`,
        kind: 'cache',
        confidence: 'high',
        evidence: 'Exact CFBundleIdentifier match',
        logical_size: 168 * MIB,
        allocated_size: 170 * MIB,
        selected_by_default: true,
      },
      {
        id: `${app.id}-name`,
        name: app.name,
        display_path: `/Users/mock/Library/Logs/${app.name}`,
        kind: 'log',
        confidence: 'medium',
        evidence: 'Exact application display-name match',
        logical_size: 18 * MIB,
        allocated_size: 18 * MIB,
        selected_by_default: false,
      },
    ],
    incomplete: false,
    warnings: [],
  };
  mockInspections.set(inspectionId, inspection);
  return inspection;
}

const mockStorageApi: StorageManagementApi = {
  async startLargeFileScan(request, onEvent) {
    const scanId = `mock-large-scan-${Date.now()}`;
    activeMockScanId = scanId;
    mockScanCancelled = false;
    onEvent({ type: 'started', scan_id: scanId });

    const roots = request.roots.length > 0 ? request.roots : ['downloads', 'desktop', 'documents'];
    for (const root of roots) {
      if (mockScanCancelled) break;
      onEvent({ type: 'root_started', root });
      onEvent({
        type: 'progress',
        root,
        entries_scanned: 1200,
        matches_found: mockLargeFiles.length,
      });
      onEvent({ type: 'root_finished', root });
    }

    const threshold = Math.max(request.min_size_bytes, 100 * MIB);
    const items = mockScanCancelled
      ? []
      : mockLargeFiles.filter((item) => item.logical_size >= threshold);
    for (const item of items) {
      onEvent({ type: 'item_found', item });
    }

    const result: LargeFileScanResult = {
      scan_id: scanId,
      items,
      entries_scanned: mockScanCancelled ? 0 : 3600,
      skipped_entries: 14,
      cancelled: mockScanCancelled,
      truncated: false,
    };
    mockScans.set(scanId, result);
    if (mockScanCancelled) {
      onEvent({ type: 'cancelled', scan_id: scanId });
    } else {
      onEvent({ type: 'finished', result });
    }
    activeMockScanId = null;
    return result;
  },

  async cancelLargeFileScan(scanId) {
    if (activeMockScanId !== scanId) {
      throw new Error('Large-file scan is no longer running');
    }
    mockScanCancelled = true;
  },

  async prepareLargeFileTrash(scanId, selectedItemIds) {
    const scan = mockScans.get(scanId);
    if (!scan) throw new Error('Large-file inventory expired. Scan again.');
    const selected = scan.items.filter((item) => selectedItemIds.includes(item.id));
    if (selected.length === 0) throw new Error('Select at least one file to move to Trash.');
    const planId = `mock-trash-plan-${Date.now()}`;
    const preview: TrashPlanPreview = {
      id: planId,
      item_count: selected.length,
      logical_size: selected.reduce((sum, item) => sum + item.logical_size, 0),
      allocated_size: selected.reduce((sum, item) => sum + item.allocated_size, 0),
      expires_at: Math.floor(Date.now() / 1000) + 300,
    };
    mockPlans.set(planId, { preview, itemIds: selected.map((item) => item.id) });
    return preview;
  },

  async getInstalledApps() {
    return mockApps.map((app) => ({ ...app }));
  },

  async inspectAppUninstall(appId) {
    const app = mockApps.find((candidate) => candidate.id === appId);
    if (!app) throw new Error('Application inventory is stale. Refresh applications.');
    if (app.is_running) throw new Error(`Quit ${app.name} before reviewing uninstall data.`);
    return inspectionFor({ ...app });
  },

  async prepareAppUninstall(inspectionId, selectedRelatedIds) {
    const inspection = mockInspections.get(inspectionId);
    if (!inspection) throw new Error('App uninstall review expired. Review the app again.');
    const selectedRelated = inspection.related_items.filter((item) =>
      selectedRelatedIds.includes(item.id)
    );
    const planId = `mock-app-trash-plan-${Date.now()}`;
    const preview: TrashPlanPreview = {
      id: planId,
      item_count: selectedRelated.length + 1,
      logical_size:
        inspection.app.logical_size +
        selectedRelated.reduce((sum, item) => sum + item.logical_size, 0),
      allocated_size:
        inspection.app.allocated_size +
        selectedRelated.reduce((sum, item) => sum + item.allocated_size, 0),
      expires_at: Math.floor(Date.now() / 1000) + 300,
    };
    mockPlans.set(planId, {
      preview,
      itemIds: [inspection.app.id, ...selectedRelated.map((item) => item.id)],
    });
    return preview;
  },

  async executeTrashPlan(planId) {
    const plan = mockPlans.get(planId);
    if (!plan) throw new Error('Trash plan not found or already used');
    mockPlans.delete(planId);
    return {
      moved_count: plan.itemIds.length,
      failed_count: 0,
      skipped_count: 0,
      moved_allocated_size: plan.preview.allocated_size,
      items: plan.itemIds.map((itemId) => ({
        item_id: itemId,
        success: true,
        message: 'Moved to Trash',
      })),
    };
  },
};

function isTauriRuntime(): boolean {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
}

export const storageApi: StorageManagementApi = isTauriRuntime()
  ? nativeStorageApi
  : mockStorageApi;

export { mockStorageApi, nativeStorageApi };
