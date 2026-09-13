import type { LocalModelItem, ObservationQuality } from '../models/types';
import { refusalForPreview, tauriDeleteLocalModel, tauriGetLocalModels } from '../utils/tauri';

class LocalModelsStore {
  models = $state<LocalModelItem[]>([]);
  quality = $state<ObservationQuality>('fresh');
  skippedEntryCount = $state<number>(0);
  incompleteReasons = $state<string[]>([]);
  isLoading = $state(false);
  isDeleting = $state(false);
  error = $state<string | null>(null);

  isPartial = $derived.by(() => {
    return this.quality === 'partial' || this.models.some((m) => m.quality === 'partial');
  });

  isUnavailable = $derived.by(() => {
    return this.quality === 'unavailable';
  });

  totalBytes = $derived.by(() => {
    return this.models.reduce((acc, m) => acc + m.size_bytes, 0);
  });

  async refresh() {
    this.isLoading = true;
    this.error = null;
    try {
      const inventory = await tauriGetLocalModels();
      this.models = inventory.items;
      this.quality = inventory.quality;
      this.skippedEntryCount = inventory.skipped_entry_count;
      this.incompleteReasons = inventory.incomplete_reasons;
    } catch (e: any) {
      this.error = e?.toString() || 'Failed to scan local models';
    } finally {
      this.isLoading = false;
    }
  }

  async deleteModel(model: LocalModelItem): Promise<boolean> {
    const refusal = refusalForPreview('Deleting a local model');
    if (refusal) {
      this.error = refusal;
      return false;
    }
    this.isDeleting = true;
    this.error = null;
    try {
      await tauriDeleteLocalModel(model.id);
      await this.refresh();
      return true;
    } catch (e: any) {
      const deletionError = e?.toString() || `Failed to delete model ${model.name}`;
      // A native tree deletion can make partial progress before reporting an
      // error. Re-read the inventory so the row does not keep showing files
      // that were already removed, while preserving the mutation failure.
      await this.refresh();
      const refreshError = this.error;
      this.error = refreshError
        ? `${deletionError} The model list could not be refreshed: ${refreshError}`
        : deletionError;
      return false;
    } finally {
      this.isDeleting = false;
    }
  }
}

export const localModelsStore = new LocalModelsStore();
