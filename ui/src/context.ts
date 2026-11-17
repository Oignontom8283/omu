import { createContext, useContext } from 'react';
import type { AppInfo, DeviceInfo, NodeKind, NodeStatus, SoundLibrary } from './types.ts';

/** What every block card needs from the editor. */
export type EditorContext = {
  apps: AppInfo[];
  devices: DeviceInfo[];
  /** The soundboards and sounds the daemon keeps (a soundboard block picks one of the boards). */
  library: SoundLibrary;
  /** Opens the soundboards panel, on this board when one is given. */
  openSoundboards: (board?: string) => void;
  /** Opens the panel of the optional modules. */
  openModules: () => void;
  statusOf: (id: string) => NodeStatus | undefined;
  /** Changes the settings of a block (its kind stays). */
  update: (id: string, kind: NodeKind) => void;
  /** Makes a block another kind of its family (the drop-down): settings reset, links follow where they still fit. */
  /** Gives a block the name people see (free text; empty text means no name). Its identifier never changes. */
  setDisplay: (id: string, text: string) => void;
  /** Shows `count` input points on a block that sums its inputs (never fewer than it has links on). */
  setInputs: (id: string, count: number) => void;
};

export const Editor = createContext<EditorContext | null>(null);

export function useEditor(): EditorContext {
  const ctx = useContext(Editor);
  if (!ctx) throw new Error('useEditor outside the editor');
  return ctx;
}
