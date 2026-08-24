import { Channel } from '@tauri-apps/api/core';
import { isTauri } from './index';
import { commands } from '../bindings/tauri';
import type {
  AppUninstallInspection,
  DeveloperArtifact,
  DeveloperArtifactScanEvent,
  DeveloperArtifactScanResult,
  DeveloperWorkspace,
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
  pickDeveloperWorkspace(): Promise<DeveloperWorkspace | null>;
  startDeveloperArtifactScan(
    workspaceIds: string[],
    onEvent: (event: DeveloperArtifactScanEvent) => void
  ): Promise<DeveloperArtifactScanResult>;
  cancelDeveloperArtifactScan(scanId: string): Promise<void>;
  prepareDeveloperArtifactCleanup(
    scanId: string,
    selectedItemIds: string[]
  ): Promise<TrashPlanPreview>;
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

  async pickDeveloperWorkspace() {
    return await unwrap(commands.pickDeveloperWorkspace());
  },

  async startDeveloperArtifactScan(workspaceIds, onEvent) {
    const channel = new Channel<DeveloperArtifactScanEvent>();
    channel.onmessage = onEvent;
    return await unwrap(commands.startDeveloperArtifactScan(workspaceIds, channel));
  },

  async cancelDeveloperArtifactScan(scanId) {
    await unwrap(commands.cancelDeveloperArtifactScan(scanId));
  },

  async prepareDeveloperArtifactCleanup(scanId, selectedItemIds) {
    return await unwrap(commands.prepareDeveloperArtifactCleanup(scanId, selectedItemIds));
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
    id: 'installer-dmg-1',
    name: 'ExampleTool.dmg',
    display_parent: '/Users/mock/Downloads',
    logical_size: 55 * MIB,
    allocated_size: 55 * MIB,
    modified_at: Math.floor(Date.now() / 1000) - 86400 * 12,
    kind: 'disk_image',
    extension: 'dmg',
  },
  {
    id: 'installer-pkg-1',
    name: 'ExampleSDK.pkg',
    display_parent: '/Users/mock/Downloads',
    logical_size: 120 * MIB,
    allocated_size: 120 * MIB,
    modified_at: Math.floor(Date.now() / 1000) - 86400 * 22,
    kind: 'installer',
    extension: 'pkg',
  },
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

const mockDeveloperWorkspaces: DeveloperWorkspace[] = [
  {
    id: 'workspace-myproject',
    name: 'Myproject',
    display_path: '/Users/mock/Myproject',
  },
  {
    id: 'workspace-work',
    name: 'work',
    display_path: '/Users/mock/work',
  },
];

const mockDeveloperArtifacts: DeveloperArtifact[] = [
  {
    id: 'artifact-rust-target',
    workspace_id: 'workspace-myproject',
    project_name: 'clean1',
    ecosystem: 'rust',
    kind: 'cargo_target',
    path: '/Users/mock/Myproject/clean1/target',
    logical_bytes: 31 * GIB,
    allocated_bytes: 30.7 * GIB,
    file_count: 184231,
    newest_mtime: Math.floor(Date.now() / 1000) - 86400 * 3,
    rebuild_hint: 'cargo build',
    evidence: ['Cargo.toml'],
    complete: true,
    incomplete_reason: null,
    selected_by_default: false,
  },
  {
    id: 'artifact-node-modules',
    workspace_id: 'workspace-work',
    project_name: 'bitbreif',
    ecosystem: 'node',
    kind: 'node_modules',
    path: '/Users/mock/work/bitbreif/node_modules',
    logical_bytes: 1.2 * GIB,
    allocated_bytes: 1.18 * GIB,
    file_count: 55342,
    newest_mtime: Math.floor(Date.now() / 1000) - 86400 * 12,
    rebuild_hint: 'pnpm install',
    evidence: ['package.json', 'pnpm-lock.yaml'],
    complete: true,
    incomplete_reason: null,
    selected_by_default: false,
  },
  {
    id: 'artifact-gradle',
    workspace_id: 'workspace-work',
    project_name: 'android-app',
    ecosystem: 'kotlin',
    kind: 'gradle_build',
    path: '/Users/mock/work/android-app/build',
    logical_bytes: 4.6 * GIB,
    allocated_bytes: 4.4 * GIB,
    file_count: 87421,
    newest_mtime: Math.floor(Date.now() / 1000) - 86400 * 28,
    rebuild_hint: './gradlew build',
    evidence: ['build.gradle.kts'],
    complete: true,
    incomplete_reason: null,
    selected_by_default: false,
  },
  {
    id: 'artifact-php-vendor',
    workspace_id: 'workspace-work',
    project_name: 'billing-api',
    ecosystem: 'php',
    kind: 'composer_vendor',
    path: '/Users/mock/work/billing-api/vendor',
    logical_bytes: 640 * MIB,
    allocated_bytes: 612 * MIB,
    file_count: 24118,
    newest_mtime: Math.floor(Date.now() / 1000) - 86400 * 4,
    rebuild_hint: 'composer install',
    evidence: ['composer.json', 'composer.lock'],
    complete: true,
    incomplete_reason: null,
    selected_by_default: false,
  },
  {
    id: 'artifact-python-incomplete',
    workspace_id: 'workspace-myproject',
    project_name: 'sdk-python',
    ecosystem: 'python',
    kind: 'python_venv',
    path: '/Users/mock/Myproject/sdk-python/.venv',
    logical_bytes: 877 * MIB,
    allocated_bytes: 860 * MIB,
    file_count: 18201,
    newest_mtime: Math.floor(Date.now() / 1000),
    rebuild_hint: 'uv sync',
    evidence: ['pyproject.toml'],
    complete: false,
    incomplete_reason: 'Permission denied while reading one or more entries',
    selected_by_default: false,
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
let activeMockDeveloperScanId: string | null = null;
let mockDeveloperScanCancelled = false;
const mockScans = new Map<string, LargeFileScanResult>();
const mockDeveloperScans = new Map<string, DeveloperArtifactScanResult>();
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
    const isInstallerFilter = request.filter === 'installers';
    const threshold = Math.max(request.min_size_bytes, isInstallerFilter ? 10 * MIB : 100 * MIB);
    const matchingMockFiles = mockLargeFiles
      .filter((item) => item.logical_size >= threshold)
      .filter((item) => !isInstallerFilter || ['dmg', 'pkg', 'mpkg', 'xip', 'iso'].includes(item.extension ?? ''));
    onEvent({ type: 'started', scan_id: scanId });

    const roots = request.roots.length > 0 ? request.roots : ['downloads', 'desktop', 'documents'];
    for (const root of roots) {
      if (mockScanCancelled) break;
      onEvent({ type: 'root_started', root });
      onEvent({
        type: 'progress',
        root,
        entries_scanned: 1200,
        matches_found: matchingMockFiles.length,
      });
      onEvent({ type: 'root_finished', root });
    }

    const items = mockScanCancelled
      ? []
      : matchingMockFiles;
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

  async pickDeveloperWorkspace() {
    return { ...mockDeveloperWorkspaces[0] };
  },

  async startDeveloperArtifactScan(workspaceIds, onEvent) {
    const scanId = `mock-developer-scan-${Date.now()}`;
    activeMockDeveloperScanId = scanId;
    mockDeveloperScanCancelled = false;
    const selectedWorkspaces = mockDeveloperWorkspaces.filter((workspace) =>
      workspaceIds.includes(workspace.id)
    );
    onEvent({
      type: 'started',
      scan_id: scanId,
      workspace_count: selectedWorkspaces.length,
    });
    const items: DeveloperArtifact[] = [];
    for (const workspace of selectedWorkspaces) {
      if (mockDeveloperScanCancelled) break;
      onEvent({ type: 'workspace_started', workspace });
      for (const artifact of mockDeveloperArtifacts.filter(
        (item) => item.workspace_id === workspace.id
      )) {
        if (mockDeveloperScanCancelled) break;
        onEvent({
          type: 'project_discovered',
          workspace_id: workspace.id,
          project_name: artifact.project_name,
          ecosystem: artifact.ecosystem,
        });
        onEvent({
          type: 'artifact_measurement_started',
          artifact_id: artifact.id,
          project_name: artifact.project_name,
          kind: artifact.kind,
        });
        items.push({ ...artifact, evidence: [...artifact.evidence] });
        onEvent({ type: 'artifact_found', artifact });
        onEvent({
          type: 'progress',
          workspace_id: workspace.id,
          discovered_count: items.length,
          measured_count: items.length,
          skipped_entries: artifact.complete ? 0 : 1,
        });
      }
      onEvent({ type: 'workspace_finished', workspace_id: workspace.id });
    }
    const result: DeveloperArtifactScanResult = {
      scan_id: scanId,
      items,
      discovered_count: items.length,
      measured_count: items.length,
      skipped_entries: items.filter((item) => !item.complete).length,
      cancelled: mockDeveloperScanCancelled,
      truncated: false,
    };
    mockDeveloperScans.set(scanId, result);
    if (result.cancelled) onEvent({ type: 'cancelled', scan_id: scanId });
    onEvent({ type: 'finished', result });
    activeMockDeveloperScanId = null;
    return result;
  },

  async cancelDeveloperArtifactScan(scanId) {
    if (activeMockDeveloperScanId !== scanId) {
      throw new Error('Developer artifact scan is no longer running');
    }
    mockDeveloperScanCancelled = true;
  },

  async prepareDeveloperArtifactCleanup(scanId, selectedItemIds) {
    const scan = mockDeveloperScans.get(scanId);
    if (!scan) throw new Error('Developer artifact inventory expired. Scan again.');
    const selected = scan.items.filter((item) => selectedItemIds.includes(item.id));
    if (selected.length === 0) {
      throw new Error('Select at least one developer artifact to move to Trash.');
    }
    if (selected.some((item) => !item.complete)) {
      throw new Error('Incomplete artifacts cannot be cleaned until they are scanned again.');
    }
    const planId = `mock-developer-trash-plan-${Date.now()}`;
    const preview: TrashPlanPreview = {
      id: planId,
      item_count: selected.length,
      logical_size: selected.reduce((sum, item) => sum + item.logical_bytes, 0),
      allocated_size: selected.reduce((sum, item) => sum + item.allocated_bytes, 0),
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
export const storageApi: StorageManagementApi = isTauri()
  ? nativeStorageApi
  : mockStorageApi;

export { mockStorageApi, nativeStorageApi };
