import type {
  Category,
  CleanEvent,
  CleanResult,
  ScanEvent,
  ScanItem,
  ScanResult,
  ZenithSettings,
} from '../models/types';
import {
  refusalForPreview,
  tauriCancelScan,
  tauriCreatePlan,
  tauriExecuteClean,
  tauriGetLastScan,
  tauriQuickCleanSafe,
  tauriScan,
} from '../utils/tauri';
import { cleanableBytes, isActionable, isAutoCleanable, isCleanable, isProviderBacked } from '../utils/cleanup';

/** How the in-flight (or most recent) scan was started. Drives auto-refresh copy. */
export type ScanTrigger = 'auto' | 'manual';

export class ScanStore {
  isScanning = $state(false);
  isCleaning = $state(false);
  currentCategory = $state<Category | null>(null);
  currentScanningItem = $state<string | null>(null);
  /** The id the running scan reported, so Stop cancels the scan being watched. */
  scanId = $state<string | null>(null);
  /** The root the walk is reading, from `RootStarted`. */
  currentRoot = $state<{ name: string; path: string } | null>(null);
  /** Items the running scan has reported so far. */
  foundItemCount = $state(0);
  /** Whether a stop request is in flight; the scan's own result ends it. */
  isCancelling = $state(false);
  lastScan = $state<ScanResult | null>(null);
  cleanProgress = $state<{
    currentItem: string;
    index: number;
    total: number;
    percent: number;
  }>({
    currentItem: '',
    index: 0,
    total: 0,
    percent: 0,
  });
  lastCleanResult = $state<CleanResult | null>(null);
  error = $state<string | null>(null);
  lastScanTrigger = $state<ScanTrigger | null>(null);
  private clock = $state(Date.now());
  private invalidated = $state(false);
  private generation = 0;
  private scanRequest: Promise<ScanResult | null> | null = null;
  private freshnessSubscribers = 0;
  private revalidating = false;
  private stopFreshness: (() => void) | null = null;

  observeFreshness(): () => void {
    this.freshnessSubscribers++;
    if (this.freshnessSubscribers === 1) {
      let timer: ReturnType<typeof setInterval> | undefined;
      const activate = () => {
        if (timer !== undefined) clearInterval(timer);
        timer = undefined;
        if (document.visibilityState !== 'visible') return;
        this.updateFreshness();
        void this.init();
        // Visible-only auto-rescan (#128): when the TTL lapses while the
        // surface stays open, refresh automatically instead of leaving a
        // stale warning for a manual click. Hidden panels never scan.
        timer = setInterval(() => this.tickFreshness(), 1000);
      };
      window.addEventListener('focus', activate);
      document.addEventListener('visibilitychange', activate);
      activate();
      this.stopFreshness = () => {
        if (timer !== undefined) clearInterval(timer);
        window.removeEventListener('focus', activate);
        document.removeEventListener('visibilitychange', activate);
      };
    }
    let disposed = false;
    return () => {
      if (disposed) return;
      disposed = true;
      if (--this.freshnessSubscribers === 0) {
        this.stopFreshness?.();
        this.stopFreshness = null;
      }
    };
  }

  get freshness(): 'empty' | 'fresh' | 'partial' | 'unavailable' | 'stale' | 'refreshing' | 'failed' {
    if (this.isScanning) return 'refreshing';
    if (this.invalidated && this.error) return 'failed';
    if (!this.lastScan) return 'empty';
    if (this.invalidated || !this.freshAt(this.clock)) return 'stale';
    if (this.lastScan.quality === 'partial') return 'partial';
    return this.lastScan.quality === 'fresh' ? 'fresh' : 'unavailable';
  }

  get canClean(): boolean {
    return (this.freshness === 'fresh' || this.freshness === 'partial') && !this.isCleaning;
  }

  /**
   * How a stopped scan reads: it states that it was stopped, that the locations
   * it had not reached were not inspected, and that retrying is safe. A user who
   * pressed Stop must never read their own action as a failure or as a scan
   * that completed. `null` when the last scan was not cancelled.
   */
  get cancelledScanNotice(): string | null {
    const scan = this.lastScan;
    if (!scan?.cancelled) return null;
    const recorded = scan.incomplete_reasons ?? [];
    const gaps = recorded.length > 0 ? ` The scan recorded: ${recorded.join('; ')}.` : '';
    return `Scan stopped before it finished, so locations it had not reached were not inspected.${gaps} Nothing was removed, and scanning again is safe.`;
  }

  private freshAt(nowMs: number): boolean {
    if (!this.lastScan) return false;
    const age = Math.floor(nowMs / 1000) - this.lastScan.finished_at;
    return age >= 0 && age < this.lastScan.valid_for_seconds;
  }

  updateFreshness() {
    this.clock = Date.now();
    if (this.lastScan && !this.freshAt(this.clock)) this.invalidate();
  }

  private tickFreshness() {
    this.updateFreshness();
    this.maybeAutoRescan();
  }

  private maybeAutoRescan() {
    if (this.revalidating) return;
    if (typeof document !== 'undefined' && document.visibilityState !== 'visible') return;
    if (this.isScanning || this.isCleaning) return;
    // A failed scan stays manual: the user retries explicitly via Scan Again.
    if (this.freshness !== 'stale' && this.freshness !== 'empty') return;
    void this.revalidateOrRescan();
  }

  /**
   * Cross-window guard (#131): another visible surface may have just
   * finished a scan, which the backend shares via getLastScan. Adopt a
   * fresh backend result instead of running a duplicate full scan against
   * the serialized storage gate. Manual scans bypass this and always scan.
   */
  private async revalidateOrRescan() {
    this.revalidating = true;
    const generation = this.generation;
    const canContinue = () => generation === this.generation
      && !this.isScanning && !this.isCleaning
      && this.freshnessSubscribers > 0
      && (typeof document === 'undefined' || document.visibilityState === 'visible');
    try {
      try {
        const cached = await tauriGetLastScan();
        if (!canContinue()) return;
        if (cached && cached.scan_id !== this.lastScan?.scan_id) {
          this.acceptScan(cached);
        }
        this.updateFreshness();
      } catch {
        // A cache-fetch failure must not block a visible surface's rescan.
      }
      if (!canContinue()) return;
      if (this.freshness !== 'stale' && this.freshness !== 'empty') return;
      void this.runScan(undefined, 'auto');
    } finally {
      this.revalidating = false;
    }
  }

  private invalidate() {
    this.invalidated = true;
    if (Object.keys(this.selectedMap).length > 0) this.selectedMap = {};
  }

  private acceptScan(scan: ScanResult) {
    this.lastScan = scan;
    this.invalidated = false;
    // A successfully accepted scan supersedes any earlier message, so an
    // item-level gap never survives as a stale destructive banner. The durable
    // `quality`/`incomplete_reasons` state is the warning surface for a partial
    // result; `error` stays reserved for a refused or failed operation.
    this.error = null;
    this.syncSelectionFromScan(scan);
    this.updateFreshness();
  }

  // Selected item IDs mapped to item objects
  selectedMap = $state<Record<string, boolean>>({});

  // Consolidated single-pass selection summary
  selectionSummary = $derived.by(() => {
    let reclaimableBytes = 0;
    let safeSelectedBytes = 0;
    let rebuildSelectedBytes = 0;
    let manualSelectedBytes = 0;
    let manualSelectedCount = 0;
    let selectedCount = 0;

    if (this.lastScan) {
      for (const cat of this.lastScan.categories) {
        for (const item of cat.items) {
          if (this.selectedMap[item.id]) {
            selectedCount++;
            const bytes = cleanableBytes(item);
            if (item.risk === 'safe') {
              safeSelectedBytes += bytes;
              reclaimableBytes += bytes;
            } else if (item.risk === 'rebuild') {
              rebuildSelectedBytes += bytes;
              reclaimableBytes += bytes;
            } else if (item.risk === 'manual') {
              // A provider-backed manual item is reclaimed by its reviewed
              // provider, so it is reclaimable through the submission path and
              // must not read as a blocking manual selection. Manual items
              // without a provider stay outside the reclaimable total.
              if (isProviderBacked(item)) {
                reclaimableBytes += bytes;
              } else {
                manualSelectedBytes += bytes;
                manualSelectedCount++;
              }
            }
          }
        }
      }
    }

    return {
      reclaimableBytes,
      safeSelectedBytes,
      rebuildSelectedBytes,
      manualSelectedBytes,
      manualSelectedCount,
      selectedCount,
    };
  });

  // Computed / Derived getters delegating to single-pass summary
  get reclaimableBytes(): number {
    return this.selectionSummary.reclaimableBytes;
  }

  get safeSelectedBytes(): number {
    return this.selectionSummary.safeSelectedBytes;
  }

  get rebuildSelectedBytes(): number {
    return this.selectionSummary.rebuildSelectedBytes;
  }

  get manualSelectedBytes(): number {
    return this.selectionSummary.manualSelectedBytes;
  }

  get manualSelectedCount(): number {
    return this.selectionSummary.manualSelectedCount;
  }

  get selectedCount(): number {
    return this.selectionSummary.selectedCount;
  }

  private initPromise: Promise<void> | null = null;

  async init() {
    if (this.initPromise) return this.initPromise;
    this.initPromise = this.loadCachedScan();
    try {
      await this.initPromise;
    } finally {
      this.initPromise = null;
    }
  }

  private async loadCachedScan() {
    const generation = this.generation;
    try {
      const cached = await tauriGetLastScan();
      if (generation !== this.generation || this.isScanning || this.isCleaning) return;
      if (!cached) {
        this.invalidate();
      } else if (cached.scan_id !== this.lastScan?.scan_id) {
        this.acceptScan(cached);
      }
      this.updateFreshness();
    } catch (error) {
      if (generation !== this.generation || this.isScanning || this.isCleaning) return;
      this.invalidate();
      this.error = `Could not verify scan. Scan again before cleaning. ${String(error)}`;
    }
  }

  syncSelectionFromScan(scan: ScanResult) {
    const newMap: Record<string, boolean> = {};
    const scanAllowsSelection = scan.quality === 'fresh' || scan.quality === 'partial';
    for (const cat of scan.categories) {
      for (const item of cat.items) {
        // Auto-select only cleanable safe items, and only if the item's own
        // observation is fresh: a partial item stays manually selectable.
        newMap[item.id] = scanAllowsSelection && isAutoCleanable(item);
      }
    }
    this.selectedMap = newMap;
  }

  private findItem(id: string): ScanItem | undefined {
    if (!this.lastScan) return undefined;
    for (const cat of this.lastScan.categories) {
      const found = cat.items.find((i) => i.id === id);
      if (found) return found;
    }
    return undefined;
  }

  toggleItem(id: string) {
    const item = this.findItem(id);
    if (!this.canClean || !item || !isCleanable(item)) return;
    this.selectedMap[id] = !this.selectedMap[id];
  }

  setItemSelected(id: string, selected: boolean) {
    const item = this.findItem(id);
    if (!this.canClean || !item || (selected && !isCleanable(item))) return;
    this.selectedMap[id] = selected;
  }

  toggleCategory(category: Category, select: boolean) {
    if (!this.lastScan || !this.canClean) return;
    const cat = this.lastScan.categories.find((c) => c.category === category);
    if (!cat) return;

    for (const item of cat.items) {
      if (select) {
        if (isCleanable(item)) this.selectedMap[item.id] = true;
      } else if (isActionable(item)) {
        this.selectedMap[item.id] = false;
      }
    }
  }

  quickCleanCategoryEnabled(category: Category, settings: ZenithSettings): boolean {
    switch (category) {
      case 'ai':
        return settings.clean_ai_tools;
      case 'developer':
        return settings.clean_developer_tools;
      case 'container':
        return settings.clean_docker;
      case 'model':
        return settings.clean_local_models;
      case 'system':
        return true;
      default:
        return false;
    }
  }

  isQuickCleanEligible(category: Category, item: ScanItem, settings: ZenithSettings): boolean {
    return isAutoCleanable(item)
      && cleanableBytes(item) > 0
      && this.quickCleanCategoryEnabled(category, settings);
  }

  quickCleanableBytes(settings: ZenithSettings): number {
    if (!this.lastScan || this.lastScan.quality === 'unavailable') return 0;
    let total = 0;
    for (const category of this.lastScan.categories) {
      for (const item of category.items) {
        if (this.isQuickCleanEligible(category.category, item, settings)) {
          total += cleanableBytes(item);
        }
      }
    }
    return total;
  }

  selectAllSafe() {
    if (!this.lastScan || !this.canClean) return;
    for (const cat of this.lastScan.categories) {
      for (const item of cat.items) {
        this.selectedMap[item.id] = isAutoCleanable(item);
      }
    }
  }

  selectQuickCleanDefaults(settings: ZenithSettings) {
    if (!this.lastScan || this.lastScan.quality === 'unavailable') return;
    const next: Record<string, boolean> = {};
    for (const category of this.lastScan.categories) {
      for (const item of category.items) {
        next[item.id] = this.isQuickCleanEligible(category.category, item, settings);
      }
    }
    this.selectedMap = next;
  }

  deselectAll() {
    for (const id of Object.keys(this.selectedMap)) {
      this.selectedMap[id] = false;
    }
  }

  isStale() {
    return this.invalidated || !this.freshAt(Date.now());
  }

  runScan(categories?: Category[], trigger: ScanTrigger = 'manual'): Promise<ScanResult | null> {
    if (this.scanRequest) return this.scanRequest;
    this.scanRequest = this.performScan(categories, trigger).finally(() => {
      this.scanRequest = null;
    });
    return this.scanRequest;
  }

  private async performScan(categories?: Category[], trigger: ScanTrigger = 'manual'): Promise<ScanResult | null> {
    this.generation++;
    this.lastScanTrigger = trigger;
    this.invalidate();
    this.isScanning = true;
    this.error = null;
    // A scan is only cancellable through the id it reports, so the previous
    // scan's id must never survive into the next one.
    this.scanId = null;
    this.currentRoot = null;
    this.foundItemCount = 0;
    this.isCancelling = false;

    try {
      const result = await tauriScan((event: ScanEvent) => {
        switch (event.type) {
          case 'Started':
            this.scanId = event.scan_id;
            this.currentCategory = null;
            this.currentRoot = null;
            this.foundItemCount = 0;
            break;
          case 'CategoryStarted':
            this.currentCategory = event.category;
            break;
          case 'RootStarted':
            this.currentRoot = { name: event.name, path: event.root };
            break;
          case 'ItemFound':
            this.currentScanningItem = event.item.name;
            this.foundItemCount++;
            break;
          case 'CategoryFinished':
            break;
          case 'Finished':
            this.currentCategory = null;
            this.currentScanningItem = null;
            this.currentRoot = null;
            break;
        }
      }, categories);

      this.acceptScan(result);
      return result;
    } catch (e: any) {
      this.error = e?.toString() || 'Scan failed';
      return null;
    } finally {
      this.isScanning = false;
      this.isCancelling = false;
      this.scanId = null;
      this.currentCategory = null;
      this.currentScanningItem = null;
      this.currentRoot = null;
    }
  }

  /**
   * Stop the scan that is running. The request names the id the scan itself
   * reported, and the scan's own result states what it managed to observe, so
   * this never clears the scanning state: the interface must not claim a scan
   * ended before its result says how.
   */
  async cancelScan(): Promise<void> {
    const scanId = this.scanId;
    if (!this.isScanning || !scanId || this.isCancelling) return;
    this.isCancelling = true;
    // The stopped scan's partial result supersedes what is selected now, so no
    // selection made from the interrupted scan survives the stop.
    this.invalidate();
    try {
      await tauriCancelScan(scanId);
    } catch (cause: unknown) {
      this.isCancelling = false;
      this.error = `Could not stop the scan: ${cause instanceof Error ? cause.message : String(cause)}`;
    }
  }

  async quickCleanSafe(): Promise<CleanResult | null> {
    if (this.isCleaning || this.isScanning) return null;
    const refusal = refusalForPreview('Cleaning');
    if (refusal) {
      this.error = refusal;
      return null;
    }
    this.updateFreshness();
    if (!this.canClean) {
      this.error = 'Scan results are out of date. Scan again and review the new results before cleaning.';
      return null;
    }

    this.isCleaning = true;
    this.error = null;
    this.lastCleanResult = null;

    try {
      const result = await tauriQuickCleanSafe((event: CleanEvent) => {
        switch (event.type) {
          case 'Started':
            this.cleanProgress = {
              currentItem: 'Starting cleanup...',
              index: 0,
              total: event.total_targets,
              percent: 0,
            };
            break;
          case 'ItemStarted':
            this.cleanProgress = {
              currentItem: event.name,
              index: event.index,
              total: event.total,
              percent: Math.round((event.index / event.total) * 100),
            };
            break;
          case 'ItemFinished':
            break;
          case 'Finished':
            this.lastCleanResult = event.result;
            break;
          case 'Error':
            this.error = event.message;
            break;
        }
      });

      this.lastCleanResult = result;
      this.invalidate();

      // Re-scan after clean to refresh metrics
      await this.runScan();

      return result;
    } catch (e: any) {
      this.invalidate();
      this.error = `${e?.toString() || 'Clean failed'} Scan again and review the results before retrying.`;
      return null;
    } finally {
      this.isCleaning = false;
    }
  }

  async cleanSelected(): Promise<CleanResult | null> {
    if (!this.lastScan) return null;
    return this.cleanItems(this.lastScan.categories.flatMap((category) => category.items));
  }

  async cleanItems(items: ScanItem[], confirmed = false): Promise<CleanResult | null> {
    if (this.isCleaning || this.isScanning) return null;
    const refusal = refusalForPreview('Cleaning');
    if (refusal) {
      this.error = refusal;
      return null;
    }
    this.updateFreshness();
    if (!this.canClean) {
      this.error = 'Scan results are out of date. Scan again and review the new results before cleaning.';
      return null;
    }
    const selectedItems = items
      .filter((item) => this.selectedMap[item.id] && isActionable(item))
      .map((item) => ({ ...item, is_selected: true }));

    if (selectedItems.length === 0) {
      this.error = 'No eligible safe or rebuild items selected for cleaning';
      return null;
    }

    this.isCleaning = true;
    this.error = null;
    this.lastCleanResult = null;

    try {
      // 1. Create and verify safety plan
      if (!this.lastScan) throw new Error('Scan result is no longer available');
      const plan = await tauriCreatePlan(this.lastScan.scan_id, selectedItems);

      if (this.isStale()) throw new Error('Scan expired. Scan again before cleaning.');

      // 2. Execute clean
      if (plan.requires_confirmation && !confirmed) {
        throw new Error('This cleanup requires explicit confirmation. Review the selected action before cleaning.');
      }

      const result = await tauriExecuteClean(plan, confirmed, (event: CleanEvent) => {
        switch (event.type) {
          case 'Started':
            this.cleanProgress = {
              currentItem: 'Starting cleanup...',
              index: 0,
              total: event.total_targets,
              percent: 0,
            };
            break;
          case 'ItemStarted':
            this.cleanProgress = {
              currentItem: event.name,
              index: event.index,
              total: event.total,
              percent: Math.round((event.index / event.total) * 100),
            };
            break;
          case 'ItemFinished':
            break;
          case 'Finished':
            this.lastCleanResult = event.result;
            break;
          case 'Error':
            this.error = event.message;
            break;
        }
      });

      this.lastCleanResult = result;
      this.invalidate();

      // Re-scan after clean to refresh metrics
      await this.runScan();

      return result;
    } catch (e: any) {
      this.invalidate();
      this.error = `${e?.toString() || 'Clean failed'} Scan again and review the results before retrying.`;
      return null;
    } finally {
      this.isCleaning = false;
    }
  }
}

export const scanStore = new ScanStore();
