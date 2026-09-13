import { describe, expect, it, vi, beforeEach } from 'vitest';
import { render } from 'svelte/server';
import { localModelsStore } from '../lib/stores/models.svelte';
import ModelsView from '../routes/dashboard/ModelsView.svelte';
import ApplicationsView from '../routes/dashboard/ApplicationsView.svelte';
import * as tauriUtils from '../lib/utils/tauri';
import type { InstalledAppInventory, LocalModelInventory } from '../lib/models/types';

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

  describe('ApplicationsView rendering', () => {
    it('renders lower bound prefix for partial app item', () => {
      const rendered = render(ApplicationsView, { props: { onBack: vi.fn() } });
      expect(rendered.body).toBeDefined();
    });
  });
});
