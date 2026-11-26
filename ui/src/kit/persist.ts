import { useCallback, useState } from 'react';

/**
 * A piece of state the page keeps in this browser (the size of a pane, the columns of a table, the view chosen): read once, checked by
 * `clean` (what was saved may be from another version, or broken), written at every change. The storage may be blocked: the page then
 * works the same and simply forgets.
 */
export function usePersistent<T>(key: string, fallback: T, clean: (saved: unknown) => T | undefined = (v) => v as T): [T, (next: T | ((old: T) => T)) => void] {
  const [value, setValue] = useState<T>(() => {
    try {
      const raw = localStorage.getItem(key);
      return raw === null ? fallback : (clean(JSON.parse(raw)) ?? fallback);
    } catch {
      return fallback;
    }
  });
  const set = useCallback(
    (next: T | ((old: T) => T)) =>
      setValue((old) => {
        const v = typeof next === 'function' ? (next as (old: T) => T)(old) : next;
        try {
          localStorage.setItem(key, JSON.stringify(v));
        } catch {
          /* storage may be blocked */
        }
        return v;
      }),
    [key],
  );
  return [value, set];
}

/** A size saved by an older page, kept when it is a sensible number of pixels. */
export const cleanSize = (min: number, max: number) => (v: unknown): number | undefined => (typeof v === 'number' && Number.isFinite(v) && v >= min && v <= max ? v : undefined);

/** A flag saved by an older page. */
export const cleanFlag = (v: unknown): boolean | undefined => (typeof v === 'boolean' ? v : undefined);

/** One of a few words. */
export const cleanChoice =
  <T extends string>(...allowed: T[]) =>
  (v: unknown): T | undefined =>
    allowed.find((a) => a === v);
