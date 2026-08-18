import type { LocalModelItem } from '../models/types';
import { tauriDeleteLocalModel, tauriGetLocalModels } from '../utils/tauri';

class LocalModelsStore {
  models = $state<LocalModelItem[]>([]);
  isLoading = $state(false);
  isDeleting = $state(false);
  error = $state<string | null>(null);

  totalBytes = $derived.by(() => {
    return this.models.reduce((acc, m) => acc + m.size_bytes, 0);
  });

  constructor() {
    this.refresh();
  }

  async refresh() {
    this.isLoading = true;
    this.error = null;
    try {
      this.models = await tauriGetLocalModels();
    } catch (e: any) {
      this.error = e?.toString() || 'Failed to scan local models';
    } finally {
      this.isLoading = false;
    }
  }

  async deleteModel(model: LocalModelItem): Promise<boolean> {
    this.isDeleting = true;
    this.error = null;
    try {
      await tauriDeleteLocalModel(model.path);
      await this.refresh();
      return true;
    } catch (e: any) {
      this.error = e?.toString() || `Failed to delete model ${model.name}`;
      return false;
    } finally {
      this.isDeleting = false;
    }
  }
}

export const localModelsStore = new LocalModelsStore();
