import { describe, expect, it } from 'vitest';
import { filterProcesses } from '../lib/utils/memory';
import type { ProcessMemory } from '../lib/models/types';

describe('filterProcesses memory search utility', () => {
  const sampleProcesses: ProcessMemory[] = [
    {
      pid: 1042,
      name: 'Docker Desktop',
      memory_bytes: 1024 * 1024 * 500,
      process_count: 4,
      can_terminate: true,
    },
    {
      pid: 2048,
      name: 'Claude Code Helper',
      memory_bytes: 1024 * 1024 * 300,
      process_count: 2,
      can_terminate: true,
    },
    {
      pid: 5096,
      name: 'Ollama Runner',
      memory_bytes: 1024 * 1024 * 1200,
      process_count: 1,
      can_terminate: true,
    },
    {
      pid: 88,
      name: 'kernel_task',
      memory_bytes: 1024 * 1024 * 800,
      process_count: 1,
      can_terminate: false,
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

  it('filters processes by exact and partial PID matching', () => {
    const exactPid = filterProcesses(sampleProcesses, '2048');
    expect(exactPid).toHaveLength(1);
    expect(exactPid[0].name).toBe('Claude Code Helper');

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
        name: 'Ollama CLI',
        memory_bytes: 1024 * 1024 * 50,
        process_count: 1,
        can_terminate: true,
      },
    ];

    const filteredTick2 = filterProcesses(tick2, query);
    expect(filteredTick2).toHaveLength(2);
    expect(filteredTick2.map((p) => p.name)).toEqual(['Ollama Runner', 'Ollama CLI']);
    expect(filteredTick2[0].memory_bytes).toBe(1024 * 1024 * 1500);
  });
});
