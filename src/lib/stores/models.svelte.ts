import type { LocalModelItem } from '../models/types';
import { refusalForPreview, tauriDeleteLocalModel, tauriGetLocalModels } from '../utils/tauri';

class LocalModelsStore {
  models = $state<LocalModelItem[]>([]);
  isLoading = $state(false);
  isDeleting = $state(false);
  error = $state<string | null>(null);

  totalBytes = $derived.by(() => {
    return this.models.reduce((acc, m) => acc + m.size_bytes, 0);
  });

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
