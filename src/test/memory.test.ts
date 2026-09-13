import { afterEach, describe, expect, it, vi } from 'vitest';
import { render } from 'svelte/server';
import { filterProcesses } from '../lib/utils/memory';
import type { MemoryMetrics, ProcessMemory } from '../lib/models/types';
import { MemoryStore, memoryStore } from '../lib/stores/memory.svelte';
import MemoryView from '../routes/dashboard/MemoryView.svelte';

afterEach(() => {
  vi.useRealTimers();
  memoryStore.memory = null;
});

describe('MemoryStore polling lifecycle', () => {
  it('polls only while at least one visible subscriber is active', async () => {
    vi.useFakeTimers();
    const store = new MemoryStore();
    const refresh = vi.spyOn(store, 'refreshMemory').mockResolvedValue(undefined);

    store.startPolling(1000);
    store.startPolling(1000);
    expect(refresh).toHaveBeenCalledTimes(1);

    store.stopPolling();
    await vi.advanceTimersByTimeAsync(1000);
    expect(refresh).toHaveBeenCalledTimes(2);

    store.stopPolling();
    await vi.advanceTimersByTimeAsync(3000);
    expect(refresh).toHaveBeenCalledTimes(2);
    expect(store.isPolling).toBe(false);
  });
});

describe('filterProcesses memory search utility', () => {
  const sampleProcesses: ProcessMemory[] = [
    {
      pid: 1042,
      pids: [1042, 1043, 1044, 1045],
      name: 'Docker Desktop',
      memory_bytes: 1024 * 1024 * 500,
      process_count: 4,
      can_terminate: true,
      termination_lease_id: 'mock-lease-test',
    },
    {
      pid: 2048,
      pids: [2048, 2049],
      name: 'Claude Code Helper',
      memory_bytes: 1024 * 1024 * 300,
      process_count: 2,
      can_terminate: true,
      termination_lease_id: 'mock-lease-test',
    },
    {
      pid: 5096,
      pids: [5096],
      name: 'Ollama Runner',
      memory_bytes: 1024 * 1024 * 1200,
      process_count: 1,
      can_terminate: true,
      termination_lease_id: 'mock-lease-test',
    },
    {
      pid: 88,
      pids: [88],
      name: 'kernel_task',
      memory_bytes: 1024 * 1024 * 800,
      process_count: 1,
      can_terminate: false,
      termination_lease_id: null,
    },
  ];

  it('returns all processes when query is empty or only whitespace', () => {
    expect(filterProcesses(sampleProcesses, '')).toEqual(sampleProcesses);
    expect(filterProcesses(sampleProcesses, '   ')).toEqual(sampleProcesses);
  });

  it('returns empty array when process list is empty', () => {
    expect(filterProcesses([], 'docker')).toEqual([]);
  });

  it('filters processes by case-insensitive name match', () => {
    const lowercase = filterProcesses(sampleProcesses, 'docker');
    expect(lowercase).toHaveLength(1);
    expect(lowercase[0].name).toBe('Docker Desktop');

    const uppercase = filterProcesses(sampleProcesses, 'DOCKER');
    expect(uppercase).toHaveLength(1);
    expect(uppercase[0].name).toBe('Docker Desktop');

    const mixed = filterProcesses(sampleProcesses, 'cLaUdE');
    expect(mixed).toHaveLength(1);
    expect(mixed[0].name).toBe('Claude Code Helper');
  });

  it('filters processes by exact and partial PID matching on representative and constituent PIDs', () => {
    const exactPid = filterProcesses(sampleProcesses, '2048');
    expect(exactPid).toHaveLength(1);
    expect(exactPid[0].name).toBe('Claude Code Helper');

    // Search by constituent child PID not equal to representative PID
    const constituentChildPid = filterProcesses(sampleProcesses, '1044');
    expect(constituentChildPid).toHaveLength(1);
    expect(constituentChildPid[0].name).toBe('Docker Desktop');

    const partialPid = filterProcesses(sampleProcesses, '42');
    expect(partialPid).toHaveLength(1);
    expect(partialPid[0].pid).toBe(1042);

    const commonDigit = filterProcesses(sampleProcesses, '0');
    expect(commonDigit.map((p) => p.pid)).toEqual([1042, 2048, 5096]);
  });

  it('returns empty array when no processes match query', () => {
    const noMatch = filterProcesses(sampleProcesses, 'nonexistent_app_xyz_9999');
    expect(noMatch).toEqual([]);
  });

  it('preserves search filter across simulated live polling array replacement', () => {
    const query = 'ollama';

    const tick1 = sampleProcesses;
    expect(filterProcesses(tick1, query)).toHaveLength(1);
    expect(filterProcesses(tick1, query)[0].name).toBe('Ollama Runner');

    // Simulate tick 2 where Ollama memory increased and a new process joined
    const tick2: ProcessMemory[] = [
      ...sampleProcesses.map((p) =>
        p.name === 'Ollama Runner' ? { ...p, memory_bytes: 1024 * 1024 * 1500 } : p
      ),
      {
        pid: 9999,
        pids: [9999],
        name: 'Ollama CLI',
        memory_bytes: 1024 * 1024 * 50,
        process_count: 1,
        can_terminate: true,
        termination_lease_id: 'mock-lease-ollama-cli',
      },
    ];

    const filteredTick2 = filterProcesses(tick2, query);
    expect(filteredTick2).toHaveLength(2);
    expect(filteredTick2.map((p) => p.name)).toEqual(['Ollama Runner', 'Ollama CLI']);
    expect(filteredTick2[0].memory_bytes).toBe(1024 * 1024 * 1500);
  });

  it('matches processes by parent name so a group can be found by what started it', () => {
    const processes: ProcessMemory[] = [
      {
        pid: 700,
        pids: [700, 701],
        name: 'rust-analyzer',
        memory_bytes: 1024 * 1024 * 400,
        process_count: 2,
        can_terminate: false,
        termination_lease_id: null,
        parent_process_names: ['Warp'],
        ownership: 'observed',
      },
      {
        pid: 800,
        pids: [800],
        name: 'node',
        memory_bytes: 1024 * 1024 * 200,
        process_count: 1,
        can_terminate: false,
        termination_lease_id: null,
        parent_process_names: ['Cursor'],
        ownership: 'observed',
      },
    ];

    expect(filterProcesses(processes, 'warp').map((p) => p.name)).toEqual(['rust-analyzer']);
    expect(filterProcesses(processes, 'CURS').map((p) => p.name)).toEqual(['node']);
    // Name and PID matching still work alongside the parent match.
    expect(filterProcesses(processes, 'rust').map((p) => p.name)).toEqual(['rust-analyzer']);
    expect(filterProcesses(processes, '800').map((p) => p.name)).toEqual(['node']);
  });
});

describe('MemoryView process provenance', () => {
  function metricsFixture(topProcesses: ProcessMemory[]): MemoryMetrics {
    return {
      total_bytes: 16 * 1024 * 1024 * 1024,
      used_bytes: 8 * 1024 * 1024 * 1024,
      available_bytes: 8 * 1024 * 1024 * 1024,
      free_bytes: 4 * 1024 * 1024 * 1024,
      compressed_bytes: 0,
      swap_used_bytes: 0,
      swap_total_bytes: 0,
      pressure: 'normal',
      top_processes: topProcesses,
      timestamp: 1,
    };
  }

  function processFixture(overrides: Partial<ProcessMemory> = {}): ProcessMemory {
    return {
      pid: 700,
      pids: [700, 701, 702],
      name: 'rust-analyzer',
      memory_bytes: 1024 * 1024 * 400,
      process_count: 3,
      can_terminate: false,
      termination_lease_id: null,
      parent_process_names: [],
      ownership: 'observed',
      ...overrides,
    };
  }

  it('labels the process list as an observation of the system snapshot', () => {
    memoryStore.memory = metricsFixture([processFixture()]);
    const { body } = render(MemoryView);

    expect(body).toContain('Zenith observes these processes');
    expect(body).not.toContain('Started by Zenith');
  });

  it('shows the snapshot parent of an observed process without claiming Zenith started it', () => {
    memoryStore.memory = metricsFixture([
      processFixture({ parent_process_names: ['Warp'], ownership: 'observed' }),
    ]);
    const { body } = render(MemoryView);

    expect(body).toContain('parent: Warp');
    expect(body).not.toContain('Started by Zenith');
  });

  it('shows the provenance chip for a process traced to Zenith', () => {
    memoryStore.memory = metricsFixture([
      processFixture({
        name: 'Node.js',
        parent_process_names: ['Zenith'],
        ownership: 'zenith_child',
      }),
    ]);
    const { body } = render(MemoryView);

    expect(body).toContain('Started by Zenith');
  });

  it('renders no parent attribution and no placeholder when the snapshot resolved none', () => {
    memoryStore.memory = metricsFixture([processFixture({ parent_process_names: [] })]);
    const { body } = render(MemoryView);

    expect(body).not.toContain('parent:');
    expect(body.toLowerCase()).not.toContain('unknown');
    expect(body).not.toContain('Started by Zenith');
  });

  it('joins multiple snapshot parents in the order the backend reported them', () => {
    memoryStore.memory = metricsFixture([
      processFixture({ parent_process_names: ['Warp', 'Zenith'], ownership: 'observed' }),
    ]);
    const { body } = render(MemoryView);

    expect(body).toContain('parent: Warp, Zenith');
  });
});
