import { createContext, useContext } from 'react';
import type { AppInfo, DeviceInfo, NodeKind, NodeStatus } from './types.ts';

/** What every node card needs from the editor. */
export type EditorContext = {
  apps: AppInfo[];
  devices: DeviceInfo[];
  statusOf: (id: string) => NodeStatus | undefined;
  update: (id: string, kind: NodeKind) => void;
  /** Renames a node (its id appears in the graph's edges). Returns an error message when refused. */
  rename: (id: string, next: string) => string | null;
};

export const Editor = createContext<EditorContext | null>(null);

export function useEditor(): EditorContext {
  const ctx = useContext(Editor);
  if (!ctx) throw new Error('useEditor outside the editor');
  return ctx;
}
