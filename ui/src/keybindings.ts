// The keyboard shortcuts of the editor: every command has a default, and the person can change them (kept in this browser, as
// a small JSON object like the keybindings file of a code editor: `{ "edit.undo": "Ctrl+Z", "search.blocks": "" }`; an empty
// text takes the shortcut away).

import { comboFromEvent } from './graph.ts';

export type CommandId =
  | 'edit.undo'
  | 'edit.redo'
  | 'edit.copy'
  | 'edit.paste'
  | 'edit.duplicate'
  | 'edit.selectAll'
  | 'search.blocks'
  | 'palette.open'
  | 'palette.openAlt'
  | 'palette.addBlock'
  | 'dock.problems'
  | 'dock.logs'
  | 'dock.monitor'
  | 'dock.inspector';

/** What each command is called (translation key) and its shortcut as shipped. */
export const COMMANDS: { id: CommandId; label: string; keys: string }[] = [
  { id: 'edit.undo', label: 'keys.undo', keys: 'Ctrl+Z' },
  { id: 'edit.redo', label: 'keys.redo', keys: 'Ctrl+Y' },
  { id: 'edit.copy', label: 'keys.copy', keys: 'Ctrl+C' },
  { id: 'edit.paste', label: 'keys.paste', keys: 'Ctrl+V' },
  { id: 'edit.duplicate', label: 'keys.duplicate', keys: 'Ctrl+D' },
  { id: 'edit.selectAll', label: 'keys.selectAll', keys: 'Ctrl+A' },
  { id: 'search.blocks', label: 'keys.searchBlocks', keys: 'Ctrl+F' },
  { id: 'palette.open', label: 'keys.palette', keys: 'F1' },
  { id: 'palette.openAlt', label: 'keys.palette', keys: 'Ctrl+Shift+P' },
  { id: 'palette.addBlock', label: 'keys.addBlock', keys: 'R' },
  { id: 'dock.problems', label: 'dock.problems', keys: 'Ctrl+Shift+M' },
  { id: 'dock.logs', label: 'app.logs', keys: 'Ctrl+Shift+U' },
  { id: 'dock.monitor', label: 'dock.monitor', keys: '' },
  { id: 'dock.inspector', label: 'dock.inspector', keys: 'Ctrl+Shift+I' },
];

export type Overrides = Partial<Record<CommandId, string>>;

const STORAGE = 'omu-keybindings';

/** What the person changed, from this browser. */
export function loadOverrides(): Overrides {
  try {
    const raw = JSON.parse(localStorage.getItem(STORAGE) ?? '{}') as Record<string, unknown>;
    return Object.fromEntries(Object.entries(raw).filter(([id, v]) => COMMANDS.some((c) => c.id === id) && typeof v === 'string')) as Overrides;
  } catch {
    return {};
  }
}

export function saveOverrides(overrides: Overrides): void {
  try {
    localStorage.setItem(STORAGE, JSON.stringify(overrides));
  } catch {
    /* storage may be blocked */
  }
}

/** The shortcut of each command now: the default unless the person changed it. */
export function bindings(overrides: Overrides): Record<CommandId, string> {
  return Object.fromEntries(COMMANDS.map((c) => [c.id, overrides[c.id] ?? c.keys])) as Record<CommandId, string>;
}

/** The commands whose shortcut is `combo` (more than one means a conflict). */
export function commandsFor(combo: string, map: Record<CommandId, string>): CommandId[] {
  return COMMANDS.filter((c) => map[c.id] !== '' && map[c.id].toLowerCase() === combo.toLowerCase()).map((c) => c.id);
}

/** The command a key press asks for, if any. */
export function commandOf(e: { key: string; code: string; ctrlKey: boolean; altKey: boolean; shiftKey: boolean; metaKey: boolean }, map: Record<CommandId, string>): CommandId | null {
  const combo = comboFromEvent(e);
  return combo ? (commandsFor(combo, map)[0] ?? null) : null;
}
