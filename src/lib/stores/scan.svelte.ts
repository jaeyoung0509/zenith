import type {
  Category,
  CleanEvent,
  CleanResult,
  DeletePlan,
  ScanEvent,
  ScanItem,
  ScanResult,
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
        if (this.selectedMap[item.id]) {
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

  selectedCount = $derived.by(() => {
    return Object.values(this.selectedMap).filter(Boolean).length;
  });

  constructor() {
    this.init();
  }

  async init() {
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
        // Auto-select only safe items with non-zero size
        newMap[item.id] = item.risk === 'safe' && (item.size.allocated ?? item.size.logical) > 0;
      }
    }
    this.selectedMap = newMap;
  }

  toggleItem(id: string) {
    this.selectedMap[id] = !this.selectedMap[id];
  }

  setItemSelected(id: string, selected: boolean) {
    this.selectedMap[id] = selected;
  }

  toggleCategory(category: Category, select: boolean) {
    if (!this.lastScan) return;
    const cat = this.lastScan.categories.find((c) => c.category === category);
    if (!cat) return;

    for (const item of cat.items) {
      this.selectedMap[item.id] = select;
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

  deselectAll() {
    for (const id of Object.keys(this.selectedMap)) {
      this.selectedMap[id] = false;
    }
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
    if (!this.lastScan || this.isCleaning) return null;

    // Collect selected items
    const selectedItems: ScanItem[] = [];
    for (const cat of this.lastScan.categories) {
      for (const item of cat.items) {
        if (this.selectedMap[item.id]) {
          selectedItems.push({
            ...item,
            is_selected: true,
          });
        }
      }
    }

    if (selectedItems.length === 0) {
      this.error = 'No items selected for cleaning';
      return null;
    }

    this.isCleaning = true;
    this.error = null;
    this.lastCleanResult = null;

    try {
      // 1. Create and verify safety plan
      const plan = await tauriCreatePlan(selectedItems);

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
