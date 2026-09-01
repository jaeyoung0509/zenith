import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  DevelopmentPortsStore,
  filterDevelopmentListeners,
  replaceDevelopmentListenerLease,
} from '../lib/stores/developmentPorts.svelte';
import { formatProcessAge } from '../lib/utils/format';
import { mockApi } from '../lib/api/mock';
import type { DevelopmentListener } from '../lib/models/types';
import fs from 'node:fs';
import path from 'node:path';

afterEach(() => {
  vi.useRealTimers();
});

describe('DevelopmentPortsStore polling lifecycle', () => {
  it('deduplicates subscribers and stops at zero subscribers', async () => {
    vi.useFakeTimers();
    const store = new DevelopmentPortsStore();
    const refresh = vi.spyOn(store, 'refresh').mockResolvedValue(undefined);

    store.startPolling(1000);
    store.startPolling(1000);
    expect(refresh).toHaveBeenCalledTimes(1);

    store.stopPolling();
    await vi.advanceTimersByTimeAsync(1000);
    expect(refresh).toHaveBeenCalledTimes(2);

    store.stopPolling();
    await vi.advanceTimersByTimeAsync(3000);
    expect(refresh).toHaveBeenCalledTimes(2);
  });
});

describe('filterDevelopmentListeners utility', () => {
  const sampleListeners: DevelopmentListener[] = [
    {
      id: 'lease-1',
      port: 5173,
      protocol: 'tcp',
      bind_address: '127.0.0.1',
      exposure: 'loopback',
      pid: 32892,
      server_name: 'Vite',
      project_name: 'clean1',
      working_directory: '~/Myproject/clean1',
      started_at: 1700000000,
      can_release: true,
      blocked_reason: null,
    },
    {
      id: 'lease-2',
      port: 3000,
      protocol: 'tcp',
      bind_address: '0.0.0.0',
      exposure: 'all_interfaces',
      pid: 40001,
      server_name: 'Next.js',
      project_name: 'web-dashboard',
      working_directory: '~/work/web-dashboard',
      started_at: 1700001000,
      can_release: true,
      blocked_reason: null,
    },
    {
      id: 'lease-3',
      port: 5432,
      protocol: 'tcp',
      bind_address: '127.0.0.1',
      exposure: 'loopback',
      pid: 5432,
      server_name: 'postgres',
      project_name: null,
      working_directory: null,
      started_at: 1699900000,
      can_release: false,
      blocked_reason: 'Protected system, terminal, database, or container process',
    },
  ];

  it('returns all listeners when query is empty or only whitespace', () => {
    expect(filterDevelopmentListeners(sampleListeners, '')).toEqual(sampleListeners);
    expect(filterDevelopmentListeners(sampleListeners, '   ')).toEqual(sampleListeners);
  });

  it('filters listeners by port number', () => {
    const result = filterDevelopmentListeners(sampleListeners, '5173');
    expect(result).toHaveLength(1);
    expect(result[0].server_name).toBe('Vite');

    const partial = filterDevelopmentListeners(sampleListeners, '54');
    expect(partial).toHaveLength(1);
    expect(partial[0].server_name).toBe('postgres');
  });

  it('filters listeners by server name case-insensitively', () => {
    expect(filterDevelopmentListeners(sampleListeners, 'vite')).toHaveLength(1);
    expect(filterDevelopmentListeners(sampleListeners, 'VITE')).toHaveLength(1);
    expect(filterDevelopmentListeners(sampleListeners, 'next')).toHaveLength(1);
  });

  it('filters listeners by project name', () => {
    const result = filterDevelopmentListeners(sampleListeners, 'clean1');
    expect(result).toHaveLength(1);
    expect(result[0].port).toBe(5173);

    const dashboard = filterDevelopmentListeners(sampleListeners, 'web-dashboard');
    expect(dashboard).toHaveLength(1);
    expect(dashboard[0].port).toBe(3000);
  });

  it('filters listeners by PID', () => {
    const result = filterDevelopmentListeners(sampleListeners, '32892');
    expect(result).toHaveLength(1);
    expect(result[0].server_name).toBe('Vite');
  });

  it('returns empty array when no listeners match query', () => {
    expect(filterDevelopmentListeners(sampleListeners, 'nonexistent-app-9999')).toEqual([]);
  });

  it('replaces only the consumed endpoint lease when ports are shared', () => {
    const samePort = { ...sampleListeners[1], id: 'lease-same-port', port: 5173 };
    const replacement = { ...sampleListeners[0], id: 'force-authorized-lease' };

    expect(
      replaceDevelopmentListenerLease(
        [sampleListeners[0], samePort],
        sampleListeners[0].id,
        replacement
      )
    ).toEqual([replacement, samePort]);
  });
});

describe('formatProcessAge utility', () => {
  it('handles null and undefined timestamps gracefully', () => {
    expect(formatProcessAge(null)).toBe('—');
    expect(formatProcessAge(undefined)).toBe('—');
  });

  it('formats seconds, minutes, and hours', () => {
    const now = Math.floor(Date.now() / 1000);
    expect(formatProcessAge(now - 30)).toBe('30s');
    expect(formatProcessAge(now - 120)).toBe('2m');
    expect(formatProcessAge(now - 3600 * 4 - 60 * 44)).toBe('4h 44m');
  });
});

describe('mockApi development port operations', () => {
  it('returns initial mock listeners containing releasable and protected servers', async () => {
    const listeners = await mockApi.listDevelopmentListeners();
    expect(listeners.length).toBeGreaterThanOrEqual(3);

    const vite = listeners.find((l) => l.port === 5173);
    expect(vite).toBeDefined();
    expect(vite?.can_release).toBe(true);
    expect(vite?.exposure).toBe('loopback');

    const next = listeners.find((l) => l.port === 3000);
    expect(next).toBeDefined();
    expect(next?.exposure).toBe('all_interfaces');

    const pg = listeners.find((l) => l.port === 5432);
    expect(pg).toBeDefined();
    expect(pg?.can_release).toBe(false);
    expect(pg?.blocked_reason).toContain('Protected');

    const agentBrowser = listeners.find((l) => l.server_name === 'agent-browser');
    expect(agentBrowser?.can_release).toBe(true);

    const chromeTesting = listeners.find((l) => l.server_name === 'Chrome for Testing');
    expect(chromeTesting?.can_release).toBe(true);
  });

  it('graceful release of Vite returns released', async () => {
    const listeners = await mockApi.listDevelopmentListeners();
    const vite = listeners.find((l) => l.port === 5173)!;

    const res = await mockApi.releaseDevelopmentListener(vite.id, 'graceful');
    expect(res.outcome).toBe('released');
    expect(res.port).toBe(5173);

    const updated = await mockApi.listDevelopmentListeners();
    expect(updated.find((l) => l.port === 5173)).toBeUndefined();
  });

  it('graceful release on port 3000 triggers still_listening and force release succeeds', async () => {
    const listeners = await mockApi.listDevelopmentListeners();
    const next = listeners.find((l) => l.port === 3000)!;

    const resGraceful = await mockApi.releaseDevelopmentListener(next.id, 'graceful');
    expect(resGraceful.outcome).toBe('still_listening');
    expect(resGraceful.listener).toBeDefined();
    expect(resGraceful.listener?.id).not.toBe(next.id); // Fresh lease!

    const resForce = await mockApi.releaseDevelopmentListener(
      resGraceful.listener!.id,
      'force'
    );
    expect(resForce.outcome).toBe('released');
    expect(resForce.port).toBe(3000);
  });

  it('attempting to release a protected listener throws error', async () => {
    const listeners = await mockApi.listDevelopmentListeners();
    const pg = listeners.find((l) => l.port === 5432)!;

    await expect(mockApi.releaseDevelopmentListener(pg.id, 'graceful')).rejects.toThrow(
      'protected'
    );
  });
});

describe('capabilities security contract', () => {
  it('quick-panel capability never gains release development listener permissions', () => {
    const quickCapPath = path.resolve(
      __dirname,
      '../../src-tauri/capabilities/quick.json'
    );
    const content = fs.readFileSync(quickCapPath, 'utf-8');
    const quickJson = JSON.parse(content);

    expect(quickJson.permissions).not.toContain('allow-release-development-listener');
    expect(quickJson.permissions).not.toContain('allow-list-development-listeners');
  });

  it('main capability includes development listener inspection and release permissions', () => {
    const mainCapPath = path.resolve(
      __dirname,
      '../../src-tauri/capabilities/main.json'
    );
    const content = fs.readFileSync(mainCapPath, 'utf-8');
    const mainJson = JSON.parse(content);

    expect(mainJson.permissions).toContain('allow-list-development-listeners');
    expect(mainJson.permissions).toContain('allow-release-development-listener');
  });
});
