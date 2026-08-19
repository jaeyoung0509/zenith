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

class ScanStore {
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

  // Selected item IDs mapped to item objects
  selectedMap = $state<Record<string, boolean>>({});

  // Computed / Derived values
  reclaimableBytes = $derived.by(() => {
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
  });

  safeSelectedBytes = $derived.by(() => {
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
  });

  rebuildSelectedBytes = $derived.by(() => {
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
  });

  manualSelectedBytes = $derived.by(() => {
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
  });

  selectedCount = $derived.by(() => {
    return Object.values(this.selectedMap).filter(Boolean).length;
  });

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
    try {
      const cached = await tauriGetLastScan();
      if (cached) {
        this.lastScan = cached;
        this.syncSelectionFromScan(cached);
      }
    } catch {
      // ignore
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
    if (item && item.risk === 'manual') return;
    this.selectedMap[id] = !this.selectedMap[id];
  }

  setItemSelected(id: string, selected: boolean) {
    const item = this.findItem(id);
    if (item && item.risk === 'manual') return;
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
    const enabledCategories: Partial<Record<Category, boolean>> = {
      ai: settings.clean_ai_tools,
      developer: settings.clean_developer_tools,
      container: settings.clean_docker,
      model: settings.clean_local_models,
      system: true,
    };
    for (const category of this.lastScan.categories) {
      for (const item of category.items) {
        const allowedRisk = item.risk === 'safe'
          || (settings.include_rebuild_caches && item.risk === 'rebuild');
        this.selectedMap[item.id] = Boolean(enabledCategories[category.category])
          && allowedRisk
          && (item.size.allocated ?? item.size.logical) > 0;
      }
    }
  }

  deselectAll() {
    for (const id of Object.keys(this.selectedMap)) {
      this.selectedMap[id] = false;
    }
  }

  isStale(maxAgeSeconds = 300) {
    if (!this.lastScan) return true;
    return Math.floor(Date.now() / 1000) - this.lastScan.finished_at >= maxAgeSeconds;
  }

  async runScan(categories?: Category[]): Promise<ScanResult | null> {
    if (this.isScanning) return null;
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
            this.lastScan = event.result;
            this.syncSelectionFromScan(event.result);
            this.currentCategory = null;
            this.currentScanningItem = null;
            break;
          case 'Error':
            this.error = event.message;
            break;
        }
      }, categories);

      this.lastScan = result;
      this.syncSelectionFromScan(result);
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
    if (this.isCleaning) return null;
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

      // Re-scan after clean to refresh metrics
      await this.runScan();

      return result;
    } catch (e: any) {
      this.error = e?.toString() || 'Clean failed';
      return null;
    } finally {
      this.isCleaning = false;
    }
  }
}

export const scanStore = new ScanStore();
