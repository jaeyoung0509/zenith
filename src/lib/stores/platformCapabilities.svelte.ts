import type {
  PlatformCapabilities,
  PlatformFeatureCapability,
} from '../models/types';
import { tauriGetPlatformCapabilities } from '../utils/tauri';

export type PlatformCapabilitiesLoader = () => Promise<PlatformCapabilities>;

/**
 * Backend-owned native feature availability shared by the dashboard and the
 * persistent Quick Panel webview. A missing snapshot is treated as unavailable
 * by action gates so an IPC failure cannot accidentally enable a native action.
 */
export class PlatformCapabilitiesStore {
  capabilities = $state<PlatformCapabilities | null>(null);
  isLoading = $state(false);
  error = $state<string | null>(null);
  private loadPromise: Promise<void> | null = null;
  private loadGeneration = 0;

  constructor(private readonly loadCapabilities: PlatformCapabilitiesLoader = tauriGetPlatformCapabilities) {}

  async load(force = false): Promise<void> {
    if (this.loadPromise && !force) return this.loadPromise;
    const generation = ++this.loadGeneration;
    const promise = this.performLoad(generation);
    this.loadPromise = promise;
    try {
      await promise;
    } finally {
      if (this.loadPromise === promise) this.loadPromise = null;
    }
  }

  private async performLoad(generation: number): Promise<void> {
    this.isLoading = true;
    try {
      const capabilities = await this.loadCapabilities();
      if (generation !== this.loadGeneration) return;
      this.capabilities = capabilities;
      this.error = null;
    } catch (error) {
      if (generation !== this.loadGeneration) return;
      // Keep the null state: action gates fail closed while the UI can surface
      // this error through the store if a platform query cannot be completed.
      this.error = error instanceof Error ? error.message : String(error);
    } finally {
      if (generation === this.loadGeneration) this.isLoading = false;
    }
  }

  feature(name: keyof Omit<PlatformCapabilities, 'platform'>): PlatformFeatureCapability | null {
    return this.capabilities?.[name] ?? null;
  }

  isAvailable(name: keyof Omit<PlatformCapabilities, 'platform'>): boolean {
    const capability = this.feature(name);
    return capability?.status === 'available';
  }

  isInspectable(name: keyof Omit<PlatformCapabilities, 'platform'>): boolean {
    const status = this.feature(name)?.status;
    return status === 'available' || status === 'read_only';
  }

  reset(): void {
    this.capabilities = null;
    this.error = null;
  }
}

export const platformCapabilitiesStore = new PlatformCapabilitiesStore();
