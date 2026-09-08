import { beforeEach, describe, expect, it, vi } from 'vitest';
import { MemoryStore } from '../lib/stores/memory.svelte';
import { tauriGetDiskVolumes } from '../lib/utils/tauri';
import type { DiskVolume } from '../lib/models/types';

vi.mock('../lib/utils/tauri', () => ({
  tauriGetDiskVolumes: vi.fn(), tauriGetMemoryMetrics: vi.fn(), tauriTerminateProcessGroup: vi.fn(),
}));
beforeEach(() => vi.resetAllMocks());

const primary: DiskVolume = {
  name: 'Data', mount_point: 'D:\\', file_system: 'NTFS', disk_type: 'SSD',
  total_bytes: 1000, used_bytes: 400, available_bytes: 600, percent_used: 40,
  is_removable: false, is_primary: true,
};

describe('shared disk snapshot', () => {
  it('coalesces concurrent refreshes into one IPC and identical summary/volume values', async () => {
    let resolve!: (value: DiskVolume[]) => void;
    vi.mocked(tauriGetDiskVolumes).mockReturnValue(new Promise((done) => { resolve = done; }));
    const store = new MemoryStore();
    const first = store.refreshDisk();
    const second = store.refreshDisk();
    expect(first).toBe(second);
    expect(tauriGetDiskVolumes).toHaveBeenCalledTimes(1);
    resolve([{ ...primary, mount_point: 'C:\\', is_primary: false }, primary]);
    await second;
    expect(store.disk?.mount_point).toBe('D:\\');
    expect(store.disk?.used_bytes).toBe(store.volumes[1].used_bytes);
    expect(store.disk?.free_bytes).toBe(store.volumes[1].available_bytes);
    expect(store.isDiskLoading).toBe(false);
  });

  it('preserves the complete last snapshot on failure and allows retry', async () => {
    const store = new MemoryStore();
    vi.mocked(tauriGetDiskVolumes).mockResolvedValue([primary]);
    await store.refreshDisk();
    vi.mocked(tauriGetDiskVolumes).mockRejectedValue(new Error('Disk unavailable'));
    await store.refreshDisk();
    expect(store.disk?.used_bytes).toBe(400);
    expect(store.volumes).toEqual([primary]);
    vi.mocked(tauriGetDiskVolumes).mockResolvedValue([]);
    await store.refreshDisk();
    expect(store.disk).toBeNull();
    expect(store.volumes).toEqual([]);
  });
});
