import type {
  DevelopmentListener,
  ReleaseDevelopmentListenerResult,
  ReleaseMode,
} from '../../models/types';

export function createDevelopmentPortsMock() {
  let listeners = initialListeners();

  return {
    async list(): Promise<DevelopmentListener[]> {
      return listeners.map((listener) => ({ ...listener }));
    },

    async release(
      id: string,
      mode: ReleaseMode
    ): Promise<ReleaseDevelopmentListenerResult> {
      const target = listeners.find((listener) => listener.id === id);
      if (!target) {
        throw new Error('Listener snapshot expired; refresh and try again.');
      }
      if (!target.can_release) {
        throw new Error('This listener is protected and cannot be released.');
      }

      if (target.port === 3000 && mode === 'graceful') {
        const freshListener: DevelopmentListener = {
          ...target,
          id: `mock-lease-next-3000-force-${Date.now()}`,
        };
        listeners = listeners.map((listener) =>
          listener.id === id ? freshListener : listener
        );
        return {
          port: target.port,
          outcome: 'still_listening',
          listener: { ...freshListener },
        };
      }

      listeners = listeners.filter((listener) => listener.id !== id);
      return {
        port: target.port,
        outcome: 'released',
        listener: null,
      };
    },
  };
}

function initialListeners(): DevelopmentListener[] {
  const now = Math.floor(Date.now() / 1000);
  return [
    {
      id: 'mock-lease-vite-5173',
      port: 5173,
      protocol: 'tcp',
      bind_address: '127.0.0.1',
      exposure: 'loopback',
      pid: 32892,
      server_name: 'Vite',
      project_name: 'clean1',
      working_directory: '~/Myproject/clean1',
      started_at: now - 17040,
      can_release: true,
      blocked_reason: null,
    },
    {
      id: 'mock-lease-next-3000',
      port: 3000,
      protocol: 'tcp',
      bind_address: '0.0.0.0',
      exposure: 'all_interfaces',
      pid: 40001,
      server_name: 'Next.js',
      project_name: 'web-dashboard',
      working_directory: '~/work/web-dashboard',
      started_at: now - 1080,
      can_release: true,
      blocked_reason: null,
    },
    {
      id: 'mock-lease-pg-5432',
      port: 5432,
      protocol: 'tcp',
      bind_address: '127.0.0.1',
      exposure: 'loopback',
      pid: 5432,
      server_name: 'postgres',
      project_name: null,
      working_directory: null,
      started_at: now - 86400,
      can_release: false,
      blocked_reason: 'Protected system, terminal, database, or container process',
    },
    {
      id: 'mock-lease-agent-browser-58937',
      port: 58937,
      protocol: 'tcp',
      bind_address: '127.0.0.1',
      exposure: 'loopback',
      pid: 88725,
      server_name: 'agent-browser',
      project_name: 'clean1',
      working_directory: '~/Myproject/clean1',
      started_at: now - 120000,
      can_release: true,
      blocked_reason: null,
    },
    {
      id: 'mock-lease-chrome-testing-62850',
      port: 62850,
      protocol: 'tcp',
      bind_address: '127.0.0.1',
      exposure: 'loopback',
      pid: 24450,
      server_name: 'Chrome for Testing',
      project_name: 'clean1',
      working_directory: '~/Myproject/clean1',
      started_at: now - 24000,
      can_release: true,
      blocked_reason: null,
    },
    {
      id: 'mock-lease-custom-8080',
      port: 8080,
      protocol: 'tcp',
      bind_address: '192.168.1.100',
      exposure: 'network',
      pid: 7777,
      server_name: 'worker-service',
      project_name: 'backend-services',
      working_directory: '~/backend-services',
      started_at: now - 7200,
      can_release: false,
      blocked_reason: 'Not recognized as a development server',
    },
  ];
}
