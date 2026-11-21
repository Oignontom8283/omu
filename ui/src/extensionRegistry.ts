// What extensions add to the interface: commands (found in the command palette) and panels (floating windows). Kept in one
// small store that the interface reads with `useSyncExternalStore`: an extension that registers something while the page is
// open sees it appear at once.

export type ExtensionCommand = {
  /** A name of its own (`my-extension.reverse`): another command with the same id replaces it. */
  id: string;
  title: string;
  run: () => void;
};

export type ExtensionPanel = {
  id: string;
  title: string;
  /** Fills the panel. May return a function that is called when the panel closes (to stop what it started). */
  mount: (container: HTMLElement) => void | (() => void);
};

type Snapshot = { commands: ExtensionCommand[]; panels: ExtensionPanel[] };

let snapshot: Snapshot = { commands: [], panels: [] };
const listeners = new Set<() => void>();

const emit = () => listeners.forEach((l) => l());
const withEntry = <T extends { id: string }>(list: T[], entry: T): T[] => [...list.filter((e) => e.id !== entry.id), entry];

/** Adds a command to the palette (or replaces the one with the same id). Returns what takes it away. */
export function registerCommand(command: ExtensionCommand): () => void {
  snapshot = { ...snapshot, commands: withEntry(snapshot.commands, command) };
  emit();
  return () => {
    snapshot = { ...snapshot, commands: snapshot.commands.filter((c) => c !== command) };
    emit();
  };
}

/** Adds a panel, opened from the palette ("Open <title>"). Returns what takes it away. */
export function registerPanel(panel: ExtensionPanel): () => void {
  snapshot = { ...snapshot, panels: withEntry(snapshot.panels, panel) };
  emit();
  return () => {
    snapshot = { ...snapshot, panels: snapshot.panels.filter((p) => p !== panel) };
    emit();
  };
}

export const subscribe = (listener: () => void): (() => void) => {
  listeners.add(listener);
  return () => listeners.delete(listener);
};

export const getSnapshot = (): Snapshot => snapshot;

/** Empties the store (tests). */
export function resetExtensions(): void {
  snapshot = { commands: [], panels: [] };
  emit();
}
