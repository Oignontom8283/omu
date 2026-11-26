// How a list chooses its rows, the way the file explorer and the code editors do: a click chooses one row, Ctrl adds or takes one
// away, Shift chooses everything from the anchor to the row, and the arrows move a cursor that the selection follows. Kept apart from
// the screen so it can be tested.

/** The chosen rows, the row the cursor is on, and the row a Shift range starts from. */
export type ListSelection = { selected: string[]; focus: string | null; anchor: string | null };

export const NOTHING: ListSelection = { selected: [], focus: null, anchor: null };

/** The rows from `a` to `b` (both included), in the order of the list. */
function range(keys: string[], a: string, b: string): string[] {
  const i = keys.indexOf(a);
  const j = keys.indexOf(b);
  if (i < 0 || j < 0) return [b];
  return keys.slice(Math.min(i, j), Math.max(i, j) + 1);
}

/** What a click on `key` chooses, with the keys that were held. */
export function clickRow(keys: string[], sel: ListSelection, key: string, mods: { ctrl?: boolean; shift?: boolean } = {}): ListSelection {
  if (mods.shift && sel.anchor && keys.includes(sel.anchor)) {
    const span = range(keys, sel.anchor, key);
    const selected = mods.ctrl ? [...new Set([...sel.selected, ...span])] : span;
    return { selected: keys.filter((k) => selected.includes(k)), focus: key, anchor: sel.anchor };
  }
  if (mods.ctrl) {
    const selected = sel.selected.includes(key) ? sel.selected.filter((k) => k !== key) : keys.filter((k) => k === key || sel.selected.includes(k));
    return { selected, focus: key, anchor: key };
  }
  return { selected: [key], focus: key, anchor: key };
}

/**
 * What moving the cursor by `step` rows does (`-Infinity` and `Infinity` go to the first and the last): without Shift the row reached is
 * the only one chosen; with Shift the range from the anchor grows or shrinks; with Ctrl only the cursor moves.
 */
export function moveCursor(keys: string[], sel: ListSelection, step: number, mods: { ctrl?: boolean; shift?: boolean } = {}): ListSelection {
  if (keys.length === 0) return NOTHING;
  const at = sel.focus ? keys.indexOf(sel.focus) : -1;
  const from = at < 0 ? (step > 0 ? -1 : keys.length) : at;
  const to = Math.max(0, Math.min(keys.length - 1, step === Infinity ? keys.length - 1 : step === -Infinity ? 0 : from + step));
  const key = keys[to];
  if (mods.shift) {
    const anchor = sel.anchor && keys.includes(sel.anchor) ? sel.anchor : key;
    return { selected: range(keys, anchor, key), focus: key, anchor };
  }
  if (mods.ctrl) return { ...sel, focus: key };
  return { selected: [key], focus: key, anchor: key };
}

/** Every row chosen (Ctrl+A). */
export const selectAll = (keys: string[], sel: ListSelection): ListSelection => ({ selected: keys.slice(), focus: sel.focus ?? keys[0] ?? null, anchor: sel.anchor ?? keys[0] ?? null });

/** The selection without the rows that are gone (filtered out, removed elsewhere). The cursor stays when its row is still there. */
export function keepExisting(keys: string[], sel: ListSelection): ListSelection {
  const has = new Set(keys);
  const selected = sel.selected.filter((k) => has.has(k));
  const focus = sel.focus && has.has(sel.focus) ? sel.focus : null;
  const anchor = sel.anchor && has.has(sel.anchor) ? sel.anchor : null;
  return selected.length === sel.selected.length && focus === sel.focus && anchor === sel.anchor ? sel : { selected, focus, anchor };
}

/** The selection made of these rows only (what another view of the same things chose), the cursor on the first. */
export const selectKeys = (selected: string[]): ListSelection => ({ selected, focus: selected[0] ?? null, anchor: selected[0] ?? null });

/** Where an item lands when it is moved `step` places (Alt+arrows), kept inside the list. */
export const movedIndex = (length: number, from: number, step: number): number => Math.max(0, Math.min(length - 1, from + step));
