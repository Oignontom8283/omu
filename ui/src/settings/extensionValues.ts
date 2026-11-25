// The values of the settings of extensions. They are kept in the browser (one entry per setting), read back at the next visit, and an
// extension that listens sees a change as soon as it is made. Where the browser keeps nothing (a private window), they last as long as
// the page does.

import type { SettingValue } from './registry.ts';

const memory = new Map<string, SettingValue>();
const listeners = new Map<string, Set<(value: SettingValue) => void>>();
const anyListeners = new Set<() => void>();

const keyOf = (extension: string, key: string) => `omu.extension.${extension}.${key}`;

const storage = (): Storage | null => {
  try {
    return globalThis.localStorage ?? null;
  } catch {
    return null;
  }
};

const isValue = (v: unknown): v is SettingValue => typeof v === 'string' || typeof v === 'number' || typeof v === 'boolean';

/** The value of a setting of an extension: what was set, or `fallback`. */
export function readExtensionSetting(extension: string, key: string, fallback: SettingValue): SettingValue {
  const k = keyOf(extension, key);
  const known = memory.get(k);
  if (known !== undefined && typeof known === typeof fallback) return known;
  try {
    const raw = storage()?.getItem(k);
    if (raw !== null && raw !== undefined) {
      const value: unknown = JSON.parse(raw);
      if (isValue(value) && typeof value === typeof fallback) {
        memory.set(k, value);
        return value;
      }
    }
  } catch {
    // a value that cannot be read is as if there were none
  }
  return fallback;
}

/** Sets a setting of an extension and tells whoever listens. */
export function writeExtensionSetting(extension: string, key: string, value: SettingValue): void {
  const k = keyOf(extension, key);
  memory.set(k, value);
  try {
    storage()?.setItem(k, JSON.stringify(value));
  } catch {
    // the browser keeps nothing: the value lasts as long as the page
  }
  listeners.get(k)?.forEach((l) => l(value));
  anyListeners.forEach((l) => l());
}

/** Calls `listener` each time the setting changes. Returns what stops it. */
export function onExtensionSetting(extension: string, key: string, listener: (value: SettingValue) => void): () => void {
  const k = keyOf(extension, key);
  const set = listeners.get(k) ?? new Set();
  set.add(listener);
  listeners.set(k, set);
  return () => set.delete(listener);
}

/** Calls `listener` each time any setting of an extension changes (what the page redraws on). Returns what stops it. */
export function onAnyExtensionSetting(listener: () => void): () => void {
  anyListeners.add(listener);
  return () => anyListeners.delete(listener);
}

/** Forgets what is kept in memory (tests). */
export function resetExtensionValues(): void {
  memory.clear();
  listeners.clear();
}
