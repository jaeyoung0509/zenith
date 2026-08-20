import { nativeApi } from './native';
import { mockApi } from './mock';

export function isTauri(): boolean {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
}

export const api = isTauri() ? nativeApi : mockApi;
export type ZenithApi = typeof nativeApi;

export { nativeApi } from './native';
export { mockApi } from './mock';
