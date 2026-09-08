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
  tauriCreatePlan,
  tauriExecuteClean,
  tauriGetLastScan,
  tauriScan,
} from '../utils/tauri';

export class ScanStore {
  isScanning = $state(false);
  isCleaning = $state(false);
  currentCategory = $state<Category | null>(null);
  currentScanningItem = $state<string | null>(null);
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
  private clock = $state(Date.now());
  private invalidated = $state(false);
  private generation = 0;
  private scanRequest: Promise<ScanResult | null> | null = null;
  private freshnessSubscribers = 0;
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

  get freshness(): 'empty' | 'fresh' | 'stale' | 'refreshing' | 'failed' {
    if (this.isScanning) return 'refreshing';
    if (this.invalidated && this.error) return 'failed';
    if (!this.lastScan) return 'empty';
    return this.invalidated || !this.freshAt(this.clock) ? 'stale' : 'fresh';
  }

  get canClean(): boolean {
    return this.freshness === 'fresh' && !this.isCleaning;
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
    if (typeof document !== 'undefined' && document.visibilityState !== 'visible') return;
    if (this.isScanning || this.isCleaning) return;
    // A failed scan stays manual: the user retries explicitly via Scan Again.
    if (this.freshness !== 'stale' && this.freshness !== 'empty') return;
    void this.runScan();
  }

  private invalidate() {
    this.invalidated = true;
    if (Object.keys(this.selectedMap).length > 0) this.selectedMap = {};
  }

  private acceptScan(scan: ScanResult) {
    this.lastScan = scan;
    this.invalidated = false;
    this.syncSelectionFromScan(scan);
    this.updateFreshness();
  }

  // Selected item IDs mapped to item objects
  selectedMap = $state<Record<string, boolean>>({});

  // Computed / Derived values
  get reclaimableBytes(): number {
    if (!this.lastScan) return 0;
    let total = 0;
    for (const cat of this.lastScan.categories) {
      for (const item of cat.items) {
        if (item.risk !== 'manual' && this.selectedMap[item.id]) {
          total += item.size.allocated ?? item.size.logical;
        }
      }
    }
    return total;
  }

  get safeSelectedBytes(): number {
    if (!this.lastScan) return 0;
    let total = 0;
    for (const cat of this.lastScan.categories) {
      for (const item of cat.items) {
        if (item.risk === 'safe' && this.selectedMap[item.id]) {
          total += item.size.allocated ?? item.size.logical;
        }
      }
    }
    return total;
  }

  get rebuildSelectedBytes(): number {
    if (!this.lastScan) return 0;
    let total = 0;
    for (const cat of this.lastScan.categories) {
      for (const item of cat.items) {
        if (item.risk === 'rebuild' && this.selectedMap[item.id]) {
          total += item.size.allocated ?? item.size.logical;
        }
      }
    }
    return total;
  }

  get manualSelectedBytes(): number {
    if (!this.lastScan) return 0;
    let total = 0;
    for (const cat of this.lastScan.categories) {
      for (const item of cat.items) {
        if (item.risk === 'manual' && this.selectedMap[item.id]) {
          total += item.size.allocated ?? item.size.logical;
        }
      }
    }
    return total;
  }

  get selectedCount(): number {
    return Object.values(this.selectedMap).filter(Boolean).length;
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
    for (const cat of scan.categories) {
      for (const item of cat.items) {
        // Auto-select only safe items with non-zero size (never manual)
        newMap[item.id] = item.risk === 'safe' && (item.size.allocated ?? item.size.logical) > 0;
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
    if (!item || item.risk === 'manual') return;
    this.selectedMap[id] = !this.selectedMap[id];
  }

  setItemSelected(id: string, selected: boolean) {
    const item = this.findItem(id);
    if (!item || item.risk === 'manual') return;
    this.selectedMap[id] = selected;
  }

  toggleCategory(category: Category, select: boolean) {
    if (!this.lastScan) return;
    const cat = this.lastScan.categories.find((c) => c.category === category);
    if (!cat) return;

    for (const item of cat.items) {
      if (item.risk !== 'manual') {
        this.selectedMap[item.id] = select;
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
    const bytes = item.size.allocated ?? item.size.logical;
    return item.risk === 'safe' && bytes > 0 && this.quickCleanCategoryEnabled(category, settings);
  }

  quickCleanableBytes(settings: ZenithSettings): number {
    if (!this.lastScan) return 0;
    let total = 0;
    for (const category of this.lastScan.categories) {
      for (const item of category.items) {
        if (this.isQuickCleanEligible(category.category, item, settings)) {
          total += item.size.allocated ?? item.size.logical;
        }
      }
    }
    return total;
  }

  selectAllSafe() {
    if (!this.lastScan) return;
    for (const cat of this.lastScan.categories) {
      for (const item of cat.items) {
        this.selectedMap[item.id] = item.risk === 'safe' && (item.size.allocated ?? item.size.logical) > 0;
      }
    }
  }

  selectQuickCleanDefaults(settings: ZenithSettings) {
    if (!this.lastScan) return;
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

  runScan(categories?: Category[]): Promise<ScanResult | null> {
    if (this.scanRequest) return this.scanRequest;
    this.scanRequest = this.performScan(categories).finally(() => {
      this.scanRequest = null;
    });
    return this.scanRequest;
  }

  private async performScan(categories?: Category[]): Promise<ScanResult | null> {
    this.generation++;
    this.invalidate();
    this.isScanning = true;
    this.error = null;

    try {
      const result = await tauriScan((event: ScanEvent) => {
        switch (event.type) {
          case 'Started':
            this.currentCategory = null;
            break;
          case 'CategoryStarted':
            this.currentCategory = event.category;
            break;
          case 'ItemFound':
            this.currentScanningItem = event.item.name;
            break;
          case 'CategoryFinished':
            break;
          case 'Finished':
            this.currentCategory = null;
            this.currentScanningItem = null;
            break;
          case 'Error':
            this.error = event.message;
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
      this.currentCategory = null;
      this.currentScanningItem = null;
    }
  }

  async cleanSelected(): Promise<CleanResult | null> {
    if (!this.lastScan) return null;
    return this.cleanItems(this.lastScan.categories.flatMap((category) => category.items));
  }

  async cleanItems(items: ScanItem[]): Promise<CleanResult | null> {
    if (this.isCleaning || this.isScanning) return null;
    this.updateFreshness();
    if (!this.canClean) {
      this.error = 'Scan results are out of date. Scan again and review the new results before cleaning.';
      return null;
    }
    const selectedItems = items
      .filter((item) => this.selectedMap[item.id] && item.risk !== 'manual')
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
      const result = await tauriExecuteClean(plan, (event: CleanEvent) => {
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
