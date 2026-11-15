// What the soundboard panel computes, kept apart from the screen so it can be tested.

import { newId } from './graph.ts';
import type { Board, GraphSpec, Pad, SoundInfo } from './types.ts';

/** The most a sound's name is shortened to when it names a pad. */
const PAD_NAME_MAX = 40;

/** `1:05` or `1:02:05`: the length of a sound. */
export function formatDuration(frames: number, rate = 48000): string {
  const total = Math.round(frames / rate);
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  const two = (n: number) => String(n).padStart(2, '0');
  return h > 0 ? `${h}:${two(m)}:${two(s)}` : `${m}:${two(s)}`;
}

/** `850 KB`, `12.4 MB`, `1.2 GB`. */
export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  const units = ['KB', 'MB', 'GB', 'TB'];
  let value = bytes / 1024;
  let i = 0;
  while (value >= 1024 && i < units.length - 1) {
    value /= 1024;
    i++;
  }
  return `${value >= 100 ? Math.round(value) : value.toFixed(1)} ${units[i]}`;
}

/** The name of a file without its folders and its extension: what a pad is called at first. */
export function nameFromFile(path: string): string {
  const file = path.split(/[\\/]/).pop() ?? path;
  const dot = file.lastIndexOf('.');
  const base = dot > 0 ? file.slice(0, dot) : file;
  const name = base.trim() || file.trim() || 'Sound';
  return name.length > PAD_NAME_MAX ? `${name.slice(0, PAD_NAME_MAX - 1)}…` : name;
}

/** A pad that plays `sound`, named after it. */
export function padFor(sound: SoundInfo): Pad {
  return { id: newId(), name: nameFromFile(sound.name), sound: sound.id, gain_db: 0, looped: false };
}

/** An empty pad. */
export function emptyPad(name: string): Pad {
  return { id: newId(), name, sound: null, gain_db: 0, looped: false };
}

/** Replaces one pad of a board. */
export function withPad(boards: Board[], boardId: string, padId: string, change: Partial<Pad>): Board[] {
  return boards.map((b) => (b.id !== boardId ? b : { ...b, pads: b.pads.map((p) => (p.id === padId ? { ...p, ...change } : p)) }));
}

/** Adds pads at the end of a board. */
export function withPadsAdded(boards: Board[], boardId: string, pads: Pad[]): Board[] {
  return boards.map((b) => (b.id !== boardId ? b : { ...b, pads: [...b.pads, ...pads] }));
}

/** Takes a pad away. */
export function withPadRemoved(boards: Board[], boardId: string, padId: string): Board[] {
  return boards.map((b) => (b.id !== boardId ? b : { ...b, pads: b.pads.filter((p) => p.id !== padId) }));
}

/** How many pads play a sound (the daemon leaves them without one when the sound is removed). */
export function padsUsing(boards: Board[], soundId: string): number {
  return boards.reduce((n, b) => n + b.pads.filter((p) => p.sound === soundId).length, 0);
}

/** The pad that moves from one place to another in a board (drag or arrows). */
export function withPadMoved(boards: Board[], boardId: string, from: number, to: number): Board[] {
  return boards.map((b) => {
    if (b.id !== boardId || from === to || from < 0 || to < 0 || from >= b.pads.length || to >= b.pads.length) return b;
    const pads = b.pads.slice();
    const [moved] = pads.splice(from, 1);
    pads.splice(to, 0, moved);
    return { ...b, pads };
  });
}

/** Whether the graph has a block for this soundboard (without one, its pads cannot play). */
export function boardInGraph(graph: GraphSpec, boardId: string): boolean {
  return graph.nodes.some((n) => n.type === 'soundboard' && n.board === boardId);
}

/** A name for a new soundboard that no other has: `Soundboard 1`, `Soundboard 2`... */
export function newBoardName(boards: Board[], base: string): string {
  const taken = new Set(boards.map((b) => b.name));
  for (let n = boards.length + 1; ; n++) {
    const name = `${base} ${n}`;
    if (!taken.has(name)) return name;
  }
}

/** The size above which sounds are stored compressed, typed in megabytes: a whole number, 0 for never. */
export function cleanCompressMb(text: string): number | null {
  if (text.trim() === '') return null;
  const n = Math.round(Number(text));
  return Number.isFinite(n) && n >= 0 ? Math.min(n, 1_000_000) : null;
}

/** Megabytes the way the setting is typed: a whole number, at least the daemon's smallest memory. */
export const MIN_CACHE_MB = 16;
export function cleanCacheMb(text: string): number | null {
  const n = Math.round(Number(text));
  return Number.isFinite(n) && n >= MIN_CACHE_MB ? Math.min(n, 1_000_000) : null;
}

/**
 * What typing a start or an end (seconds) changes in a pad: the start never goes past the end (when there is one), and
 * an end that is not after the start means "to the end of the sound" (0).
 */
export function partChange(pad: { start_ms?: number; end_ms?: number }, which: 'start_ms' | 'end_ms', seconds: string): { start_ms?: number; end_ms?: number } {
  const ms = Math.max(0, Math.round((Number(seconds) || 0) * 1000));
  const start = which === 'start_ms' ? ms : (pad.start_ms ?? 0);
  const end = which === 'end_ms' ? ms : (pad.end_ms ?? 0);
  return { start_ms: start, end_ms: end !== 0 && end <= start ? 0 : end };
}
