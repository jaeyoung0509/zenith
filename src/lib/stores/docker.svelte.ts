import type { DockerStatus } from '../models/types';
import { tauriGetDockerStatus, tauriPruneDocker } from '../utils/tauri';

class DockerStore {
  status = $state<DockerStatus | null>(null);
  isLoading = $state(false);
  isPruning = $state(false);
  error = $state<string | null>(null);

  constructor() {
    this.refresh();
  }

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
