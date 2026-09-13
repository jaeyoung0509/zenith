import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest';
import { render } from 'svelte/server';
import { localModelsStore } from '../lib/stores/models.svelte';
import { platformCapabilitiesStore } from '../lib/stores/platformCapabilities.svelte';
import { platformContextStore } from '../lib/stores/platformContext.svelte';
import ModelsView from '../routes/dashboard/ModelsView.svelte';
import ApplicationsView from '../routes/dashboard/ApplicationsView.svelte';
import * as tauriUtils from '../lib/utils/tauri';
import platformCapabilitiesGolden from '../lib/bindings/platform-capabilities.golden.json';
import platformContextGolden from '../lib/bindings/platform-context.golden.json';
import type {
  AppUninstallInspection,
  InstalledApp,
  InstalledAppInventory,
  LocalModelInventory,
  PlatformCapabilities,
  PlatformContext,
  TrashPlanPreview,
  TrashResult,
} from '../lib/models/types';
import { isSelectedAppTrashLowerBound } from '../lib/utils/storageManagement';

describe('Inventory Truthfulness (#196)', () => {
  beforeEach(() => {
    vi.restoreAllMocks();
  });

  describe('LocalModelsStore', () => {
    it('accurately distinguishes fresh, partial, and unavailable inventories', async () => {
      // 1. Fresh inventory
      const freshInventory: LocalModelInventory = {
        items: [
          {
            id: 'ollama.llama3:8b',
            name: 'llama3:8b',
            source: 'ollama',
            path: '/Users/test/.ollama/models/manifests/registry.ollama.ai/library/llama3/8b',
            size_bytes: 4 * 1024 * 1024 * 1024,
            format: 'GGUF',
            parameter_size: '8B',
            quantization: 'Q4_0',
            last_modified: 1700000000,
            quality: 'fresh',
            incomplete_reason: null,
            skipped_entries: 0,
          },
        ],
        quality: 'fresh',
        skipped_entry_count: 0,
        incomplete_reasons: [],
      };

      vi.spyOn(tauriUtils, 'tauriGetLocalModels').mockResolvedValue(freshInventory);
      await localModelsStore.refresh();

      expect(localModelsStore.models).toHaveLength(1);
      expect(localModelsStore.isPartial).toBe(false);
      expect(localModelsStore.isUnavailable).toBe(false);
      expect(localModelsStore.totalBytes).toBe(4 * 1024 * 1024 * 1024);

      // 2. Partial inventory (corrupted/skipped entries present)
      const partialInventory: LocalModelInventory = {
        items: [
          {
            id: 'ollama.llama3:8b',
            name: 'llama3:8b',
            source: 'ollama',
            path: '/Users/test/.ollama/models/manifests/registry.ollama.ai/library/llama3/8b',
            size_bytes: 4 * 1024 * 1024 * 1024,
            format: 'GGUF',
            parameter_size: '8B',
            quantization: 'Q4_0',
            last_modified: 1700000000,
            quality: 'partial',
            incomplete_reason: 'Skipped subdirectories',
            skipped_entries: 2,
          },
        ],
        quality: 'partial',
        skipped_entry_count: 2,
        incomplete_reasons: ['Could not read Ollama directory entry'],
      };

      vi.spyOn(tauriUtils, 'tauriGetLocalModels').mockResolvedValue(partialInventory);
      await localModelsStore.refresh();

      expect(localModelsStore.isPartial).toBe(true);
      expect(localModelsStore.isUnavailable).toBe(false);
      expect(localModelsStore.skippedEntryCount).toBe(2);
      expect(localModelsStore.incompleteReasons).toContain(
        'Could not read Ollama directory entry'
      );

      // 3. Unavailable inventory
      const unavailableInventory: LocalModelInventory = {
        items: [],
        quality: 'unavailable',
        skipped_entry_count: 1,
        incomplete_reasons: ['Permission denied reading ~/.ollama'],
      };

      vi.spyOn(tauriUtils, 'tauriGetLocalModels').mockResolvedValue(unavailableInventory);
      await localModelsStore.refresh();

      expect(localModelsStore.isPartial).toBe(false);
      expect(localModelsStore.isUnavailable).toBe(true);
      expect(localModelsStore.models).toHaveLength(0);
    });
  });

  describe('ModelsView rendering', () => {
    it('displays lower-bound prefix and partial warning for partial discovery', () => {
      localModelsStore.models = [
        {
          id: 'hf.model',
          name: 'meta-llama/Llama-3',
          source: 'huggingface',
          path: '/cache/hub/models--meta-llama',
          size_bytes: 8 * 1024 * 1024 * 1024,
          format: 'safetensors',
          parameter_size: null,
          quantization: null,
          last_modified: 1700000000,
          quality: 'partial',
          incomplete_reason: 'Permission denied in weights subdir',
          skipped_entries: 1,
        },
      ];
      localModelsStore.quality = 'partial';
      localModelsStore.skippedEntryCount = 1;
      localModelsStore.incompleteReasons = ['Weights subdir unreadable'];

      const rendered = render(ModelsView);
      expect(rendered.body).toContain('≥');
      expect(rendered.body).toContain('Partial model discovery');
      expect(rendered.body).toContain('Displayed model sizes are lower bounds');
    });

    it('distinguishes complete-empty from incomplete-empty state in ModelsView', () => {
      // Complete empty
      localModelsStore.models = [];
      localModelsStore.quality = 'fresh';
      localModelsStore.skippedEntryCount = 0;
      localModelsStore.incompleteReasons = [];

      let rendered = render(ModelsView);
      expect(rendered.body).toContain('No local models detected');
      expect(rendered.body).not.toContain('No complete models discovered');

      // Incomplete/partial empty
      localModelsStore.quality = 'partial';
      localModelsStore.skippedEntryCount = 1;
      localModelsStore.incompleteReasons = ['Unreadable directory'];

      rendered = render(ModelsView);
      expect(rendered.body).toContain('No complete models discovered');
      expect(rendered.body).not.toContain('No local models detected');

      // Unavailable empty
      localModelsStore.quality = 'unavailable';
      rendered = render(ModelsView);
      expect(rendered.body).toContain('Local model discovery unavailable');
    });
  });

  describe('isSelectedAppTrashLowerBound utility', () => {
    const baseApp: InstalledApp = {
      id: 'app-1',
      name: 'Test App',
      bundle_id: 'com.test.app',
      version: '1.0.0',
      display_path: '/Applications/Test App.app',
      executable_name: 'Test App',
      logical_size: 100_000,
      allocated_size: 104_857_600,
      modified_at: 1700000000,
      install_source: 'application_bundle',
      is_running: false,
      is_system_protected: false,
      quality: 'fresh',
      incomplete_reason: null,
      skipped_entries: 0,
    };

    const baseInspection: AppUninstallInspection = {
      inspection_id: 'insp-1',
      app: baseApp,
      related_items: [
        {
          id: 'related-1',
          name: 'Caches',
          display_path: '~/Library/Caches/com.test.app',
          kind: 'cache',
          confidence: 'high',
          logical_size: 10_000,
          allocated_size: 20_000_000,
          selected_by_default: true,
          quality: 'fresh',
          incomplete_reason: null,
          skipped_entries: 0,
          evidence: 'bundle id match',
        },
        {
          id: 'related-2',
          name: 'App Support',
          display_path: '~/Library/Application Support/Test App',
          kind: 'application_support',
          confidence: 'high',
          logical_size: 50_000,
          allocated_size: 60_000_000,
          selected_by_default: false,
          quality: 'partial',
          incomplete_reason: 'Skipped restricted subdir',
          skipped_entries: 1,
          evidence: 'exact name match',
        },
      ],
      incomplete: false,
      warnings: [],
    };

    it('returns false when app and selected related items are fresh', () => {
      // only related-1 selected (fresh)
      expect(isSelectedAppTrashLowerBound(baseInspection, ['related-1'])).toBe(false);
    });

    it('returns true when an unselected item is partial but gets selected', () => {
      expect(isSelectedAppTrashLowerBound(baseInspection, ['related-1', 'related-2'])).toBe(true);
    });

    it('returns false if partial related item is not selected', () => {
      expect(isSelectedAppTrashLowerBound(baseInspection, ['related-1'])).toBe(false);
    });

    it('returns true when app itself is partial, regardless of selected related items', () => {
      const partialAppInspection: AppUninstallInspection = {
        ...baseInspection,
        app: {
          ...baseApp,
          quality: 'partial',
          incomplete_reason: 'Unreadable bundle resource',
          skipped_entries: 1,
        },
      };
      expect(isSelectedAppTrashLowerBound(partialAppInspection, [])).toBe(true);
      expect(isSelectedAppTrashLowerBound(partialAppInspection, ['related-1'])).toBe(true);
    });
  });

  describe('ApplicationsView rendering truthfulness', () => {
    beforeEach(() => {
      platformCapabilitiesStore.capabilities = platformCapabilitiesGolden.macos as unknown as PlatformCapabilities;
      platformContextStore.context = platformContextGolden.macos as unknown as PlatformContext;
    });

    afterEach(() => {
      platformCapabilitiesStore.reset();
      platformContextStore.reset();
    });

    const partialApp: InstalledApp = {
      id: 'app-partial',
      name: 'Partial App',
      bundle_id: 'com.test.partial',
      version: '2.0.0',
      display_path: '/Applications/Partial App.app',
      executable_name: 'Partial App',
      logical_size: 100_000_000,
      allocated_size: 120_000_000,
      modified_at: 1700000000,
      install_source: 'application_bundle',
      is_running: false,
      is_system_protected: false,
      quality: 'partial',
      incomplete_reason: 'Unreadable framework',
      skipped_entries: 2,
    };

    const freshApp: InstalledApp = {
      id: 'app-fresh',
      name: 'Fresh App',
      bundle_id: 'com.test.fresh',
      version: '1.0.0',
      display_path: '/Applications/Fresh App.app',
      executable_name: 'Fresh App',
      logical_size: 50_000_000,
      allocated_size: 55_000_000,
      modified_at: 1700000000,
      install_source: 'application_bundle',
      is_running: false,
      is_system_protected: false,
      quality: 'fresh',
      incomplete_reason: null,
      skipped_entries: 0,
    };

    it('renders lower bound prefix ≥ for partial app in installed list', () => {
      const rendered = render(ApplicationsView, {
        props: {
          onBack: vi.fn(),
          initialApps: [partialApp, freshApp],
        },
      });

      expect(rendered.body).toContain('Partial App');
      expect(rendered.body).toContain('≥');
      expect(rendered.body).toContain('Fresh App');
    });

    it('renders lower bound prefix ≥ on partial app bundle and related items in detail pane', () => {
      const inspection: AppUninstallInspection = {
        inspection_id: 'insp-partial',
        app: partialApp,
        related_items: [
          {
            id: 'rel-1',
            name: 'Cache Dir',
            display_path: '~/Library/Caches/Partial App',
            kind: 'cache',
            confidence: 'high',
            logical_size: 10_000_000,
            allocated_size: 15_000_000,
            selected_by_default: true,
            quality: 'partial',
            incomplete_reason: 'Skipped subdirectory',
            skipped_entries: 1,
            evidence: 'bundle match',
          },
        ],
        incomplete: true,
        warnings: ['Unreadable framework'],
      };

      const rendered = render(ApplicationsView, {
        props: {
          onBack: vi.fn(),
          initialApps: [partialApp],
          initialInspection: inspection,
        },
      });

      // App bundle partial note
      expect(rendered.body).toContain('≥ 114.4 MB app bundle (partial)');
      // Ready for review banner has ≥ prefix
      expect(rendered.body).toContain('≥');
      expect(rendered.body).toContain('selected for review');
      // Related item has ≥ prefix
      expect(rendered.body).toContain('≥ 14.3 MB');
    });

    it('renders lower bound prefix ≥ in plan preview card when size_is_lower_bound is true', () => {
      const inspection: AppUninstallInspection = {
        inspection_id: 'insp-plan',
        app: partialApp,
        related_items: [],
        incomplete: false,
        warnings: [],
      };

      const plan: TrashPlanPreview = {
        id: 'plan-123',
        item_count: 1,
        logical_size: 100_000_000,
        allocated_size: 120_000_000,
        expires_at: Math.floor(Date.now() / 1000) + 300,
        size_is_lower_bound: true,
      };

      const rendered = render(ApplicationsView, {
        props: {
          onBack: vi.fn(),
          initialApps: [partialApp],
          initialInspection: inspection,
          initialPlan: plan,
        },
      });

      expect(rendered.body).toContain('Uninstall review ready');
      expect(rendered.body).toContain('App bundle plus reviewed data: 1 items · ≥ 114.4 MB');
    });

    it('renders lower bound prefix ≥ in trashResult summary card when size_is_lower_bound is true', () => {
      const trashResult: TrashResult = {
        moved_count: 2,
        failed_count: 0,
        skipped_count: 0,
        moved_allocated_size: 120_000_000,
        items: [
          { item_id: 'app-1', success: true, message: 'Moved to Trash' },
        ],
        size_is_lower_bound: true,
      };

      const rendered = render(ApplicationsView, {
        props: {
          onBack: vi.fn(),
          initialApps: [freshApp],
          initialTrashResult: trashResult,
        },
      });

      expect(rendered.body).toContain('Moved 2 reviewed items to Trash');
      expect(rendered.body).toContain('≥ 114.4 MB');
    });

    it('distinguishes complete-empty, incomplete-empty, and unavailable states', () => {
      // 1. Complete empty
      let rendered = render(ApplicationsView, {
        props: {
          onBack: vi.fn(),
          initialApps: [],
          initialInventoryQuality: 'fresh',
        },
      });
      expect(rendered.body).toContain('No applications found.');
      expect(rendered.body).not.toContain('Partial application discovery');
      expect(rendered.body).not.toContain('Application discovery unavailable');

      // 2. Incomplete empty
      rendered = render(ApplicationsView, {
        props: {
          onBack: vi.fn(),
          initialApps: [],
          initialInventoryQuality: 'partial',
          initialSkippedEntryCount: 3,
          initialIncompleteReasons: ['Skipped 3 root paths'],
        },
      });
      expect(rendered.body).toContain('Partial application discovery');
      expect(rendered.body).toContain('No applications discovered. Some application locations were inaccessible.');

      // 3. Unavailable empty
      rendered = render(ApplicationsView, {
        props: {
          onBack: vi.fn(),
          initialApps: [],
          initialInventoryQuality: 'unavailable',
          initialIncompleteReasons: ['Permission denied'],
        },
      });
      expect(rendered.body).toContain('Application discovery unavailable');
      expect(rendered.body).toContain('Application discovery failed. Could not read application directories.');
    });
  });
});
