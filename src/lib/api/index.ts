import { nativeApi } from './native';
import { mockApi } from './mock';

export type ApiBridgeMode = 'native' | 'preview';

/**
 * `bridge-appeared-late` — the page started serving preview data and the
 * native bridge showed up afterwards (WebView2 can inject it after the module
 * graph has already evaluated).
 *
 * `native-bridge-missing` — the tab is running inside a Tauri webview but the
 * bridge never became available, so commands fall back to preview data.
 */
export type ApiBridgeNotice = 'bridge-appeared-late' | 'native-bridge-missing';

export interface ApiBridgeSnapshot {
  mode: ApiBridgeMode;
  notice: ApiBridgeNotice | null;
  /** User-facing explanation, `null` while the bridge behaves as expected. */
  message: string | null;
}

const NOTICE_MESSAGES: Record<ApiBridgeNotice, string> = {
  'bridge-appeared-late':
    'The native Tauri bridge appeared after the preview data layer had already answered a command. Zenith is using native commands from now on.',
  'native-bridge-missing':
    'This window is running inside the Tauri webview but the native bridge is unavailable. Zenith is showing preview data instead of live results.',
};

/**
 * Tauri injects `__TAURI_INTERNALS__` into the page. WebView2 can inject it
 * after the frontend module graph has evaluated, so the check happens on every
 * call instead of being frozen into a module-level constant.
 */
function nativeBridgePresent(): boolean {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
}

let mode: ApiBridgeMode | null = null;
let sawNativeBridge = false;
let notice: ApiBridgeNotice | null = null;
const listeners = new Set<(snapshot: ApiBridgeSnapshot) => void>();

export function isTauri(): boolean {
  return nativeBridgePresent();
}

export function apiBridgeSnapshot(): ApiBridgeSnapshot {
  return {
    mode: mode ?? (nativeBridgePresent() ? 'native' : 'preview'),
    notice,
    message: notice ? NOTICE_MESSAGES[notice] : null,
  };
}

export function subscribeApiBridge(
  listener: (snapshot: ApiBridgeSnapshot) => void
): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

function publish(): void {
  const snapshot = apiBridgeSnapshot();
  for (const listener of listeners) listener(snapshot);
}

function announce(next: ApiBridgeNotice): void {
  if (notice === next) return;
  notice = next;
  console.warn(`[zenith] ${NOTICE_MESSAGES[next]}`);
}

function resolveBridgeMode(): ApiBridgeMode {
  let next: ApiBridgeMode;
  if (nativeBridgePresent()) {
    next = 'native';
    // A preview answer already went out, so this transition has to be visible.
    if (mode === 'preview') announce('bridge-appeared-late');
    sawNativeBridge = true;
  } else {
    next = 'preview';
    // A native session whose bridge never arrives still serves the Tauri
    // origin; saying so beats quietly presenting preview data as real.
    const location = typeof window === 'undefined' ? null : window.location;
    const tauriOrigin =
      !!location && (location.protocol === 'tauri:' || location.hostname === 'tauri.localhost');
    if (tauriOrigin || sawNativeBridge) announce('native-bridge-missing');
  }

  if (mode !== next) {
    mode = next;
    publish();
  }
  return next;
}

/**
 * Resolves the implementation for a single call. Prefer this over importing a
 * frozen `nativeApi`/`mockApi` pair so a late bridge cannot be missed.
 */
export function selectApiImpl<T>(nativeImpl: T, previewImpl: T): T {
  return resolveBridgeMode() === 'native' ? nativeImpl : previewImpl;
}

/**
 * Builds an object that re-resolves the bridge on every property access, so a
 * surface never keeps serving preview mocks after the native bridge appears.
 */
export function dispatchApi<T extends object>(nativeImpl: T, previewImpl: T): T {
  return new Proxy({} as T, {
    get(_target, property) {
      const impl = selectApiImpl(nativeImpl, previewImpl);
      const value = Reflect.get(impl as object, property);
      return typeof value === 'function' ? value.bind(impl) : value;
    },
  });
}

export type ZenithApi = typeof nativeApi;

export const api: ZenithApi = dispatchApi(nativeApi, mockApi);

export { nativeApi } from './native';
export { mockApi } from './mock';
