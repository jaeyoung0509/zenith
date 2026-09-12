import type { PlatformContext } from '../models/types';
import { tauriGetPlatformContext } from '../utils/tauri';

export type PlatformContextLoader = () => Promise<PlatformContext>;

/**
 * Platform vocabulary and window chrome ownership reported by the backend.
 *
 * The values are needed while the first paint of the main window is being laid
 * out, so consumers read `overlayTitleBar` and friends directly and only the
 * first request is allowed to touch IPC: `load()` is single-flight, and a
 * generation guard keeps a slower response from overwriting a newer one.
 * Nothing is started from the constructor, so an unmounted preview never
 * performs work on its own.
 */
export class PlatformContextStore {
  context = $state<PlatformContext | null>(null);
  isLoading = $state(false);
  error = $state<string | null>(null);
  private loadPromise: Promise<void> | null = null;
  private loadGeneration = 0;

  constructor(private readonly loadContext: PlatformContextLoader = tauriGetPlatformContext) {}

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
      const context = await this.loadContext();
      if (generation !== this.loadGeneration) return;
      this.context = context;
      this.error = null;
    } catch (error) {
      if (generation !== this.loadGeneration) return;
      // Fail closed: without a context the interface must not claim overlay
      // chrome or macOS nouns it cannot verify. The error stays readable so a
      // surface can offer a retry.
      this.error = error instanceof Error ? error.message : String(error);
    } finally {
      if (generation === this.loadGeneration) this.isLoading = false;
    }
  }

  /** True only when the backend confirmed the overlay title bar. */
  get overlayTitleBar(): boolean {
    return this.context?.overlay_title_bar === true;
  }

  /** True only when the backend confirmed the OS-drawn caption bar. */
  get nativeCaptionBar(): boolean {
    return this.context?.native_caption_bar === true;
  }

  get logDirectory(): string | null {
    return this.context?.log_directory ?? null;
  }

  get revealLabel(): string {
    return this.context?.reveal_label ?? 'file manager';
  }

  get trashLabel(): string {
    return this.context?.trash_label ?? 'the recoverable-delete location';
  }

  get appDataLabel(): string {
    return this.context?.app_data_label ?? 'application data';
  }

  /** Null when unknown: a heading cannot fall back to another platform's noun. */
  get quickPanelSurfaceLabel(): string | null {
    return this.context?.quick_panel_surface_label ?? null;
  }

  get containerRuntimeHint(): string | null {
    return this.context?.container_runtime_hint ?? null;
  }

  get appIdentityLabel(): string {
    return this.context?.app_identity_label ?? 'application identity';
  }

  get primaryAccelerator(): 'meta' | 'ctrl' {
    return this.context?.primary_accelerator === 'ctrl' ? 'ctrl' : 'meta';
  }

  reset(): void {
    this.context = null;
    this.error = null;
  }
}

export const platformContextStore = new PlatformContextStore();
