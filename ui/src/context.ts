import { createContext, useContext } from 'react';
import type { AppInfo, DeviceInfo, NodeKind, NodeStatus, NodeType } from './types.ts';

/** What every block card needs from the editor. */
export type EditorContext = {
  apps: AppInfo[];
  devices: DeviceInfo[];
  statusOf: (id: string) => NodeStatus | undefined;
  /** Changes the settings of a block (its kind stays). */
  update: (id: string, kind: NodeKind) => void;
  /** Makes a block another kind of its family (the drop-down): settings reset, links follow where they still fit. */
  changeKind: (id: string, type: NodeType) => void;
  /** Renames a block (its name appears in the graph's edges). Returns the translation key of the error when refused. */
  rename: (id: string, next: string) => string | null;
};

export const Editor = createContext<EditorContext | null>(null);

export function useEditor(): EditorContext {
  const ctx = useContext(Editor);
  if (!ctx) throw new Error('useEditor outside the editor');
  return ctx;
}
