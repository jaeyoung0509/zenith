import type { DockerStatus } from '../models/types';
import { refusalForPreview, tauriGetDockerStatus, tauriPruneDocker } from '../utils/tauri';

class DockerStore {
  status = $state<DockerStatus | null>(null);
  isLoading = $state(false);
  isPruning = $state(false);
  error = $state<string | null>(null);

  async refresh() {
    this.isLoading = true;
    this.error = null;
    try {
      this.status = await tauriGetDockerStatus();
    } catch (e: any) {
      this.error = e?.toString() || 'Failed to fetch Docker status';
    } finally {
      this.isLoading = false;
    }
  }

  async pruneTarget(signatureId: string): Promise<number> {
    const refusal = refusalForPreview('Pruning Docker data');
    if (refusal) {
      this.error = refusal;
      return 0;
    }
    this.isPruning = true;
    this.error = null;
    try {
      const reclaimed = await tauriPruneDocker(signatureId);
      await this.refresh();
      return reclaimed;
    } catch (e: any) {
      this.error = e?.toString() || 'Failed to prune Docker target';
      return 0;
    } finally {
      this.isPruning = false;
    }
  }
}

export const dockerStore = new DockerStore();
