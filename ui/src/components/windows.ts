import { useCallback, useState } from 'react';

/** A floating window of a page: what it is about, where it is. */
export type Floating<K extends string = string> = { key: K; x: number; y: number };

/**
 * The floating windows a page opens, the last one in front (the same windows as the information ones of the editor: see
 * `FloatingWindow`). Opening a window that is already open brings it to the front.
 */
export function useWindowStack<K extends string>() {
  const [list, setList] = useState<Floating<K>[]>([]);
  const open = useCallback((key: K, at: { x: number; y: number }) => {
    const inside = (x: number, y: number) => ({
      x: Math.max(4, Math.min(window.innerWidth - 440, x)),
      y: Math.max(4, Math.min(window.innerHeight - 200, y)),
    });
    setList((l) => [...l.filter((w) => w.key !== key), l.find((w) => w.key === key) ?? { key, ...inside(at.x, at.y) }]);
  }, []);
  const close = useCallback((key: K) => setList((l) => l.filter((w) => w.key !== key)), []);
  const move = useCallback((key: K, x: number, y: number) => setList((l) => l.map((w) => (w.key === key ? { ...w, x, y } : w))), []);
  const front = useCallback((key: K) => setList((l) => (l[l.length - 1]?.key === key ? l : [...l.filter((w) => w.key !== key), ...l.filter((w) => w.key === key)])), []);
  /** Closes the windows `keep` says are gone (a sound that was removed...). */
  const prune = useCallback((keep: (key: K) => boolean) => setList((l) => (l.every((w) => keep(w.key)) ? l : l.filter((w) => keep(w.key)))), []);
  return { list, open, close, move, front, prune };
}
