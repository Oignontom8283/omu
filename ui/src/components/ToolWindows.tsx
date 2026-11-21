import { useEffect, useRef, useState, type ReactNode } from 'react';
import type { ExtensionPanel } from '../extensionRegistry.ts';
import { useT } from '../i18n/index.tsx';
import { FloatingWindow } from './FloatingWindow.tsx';

/** A floating window that keeps its own place: opened where it was last put, moved by its title. */
function ToolWindow({ title, at, onClose, children }: { title: string; at: { x: number; y: number }; onClose: () => void; children: ReactNode }) {
  const [place, setPlace] = useState(at);
  return (
    <FloatingWindow title={title} x={place.x} y={place.y} order={60} onMove={(x, y) => setPlace({ x, y })} onFocus={() => undefined} onClose={onClose}>
      {children}
    </FloatingWindow>
  );
}

/** The graph as the daemon reads it (JSON), to look at or copy. */
export function JsonWindow({ graph, onClose }: { graph: unknown; onClose: () => void }) {
  const t = useT();
  return (
    <ToolWindow title={t('app.json')} at={{ x: Math.max(8, window.innerWidth - 520), y: 130 }} onClose={onClose}>
      <textarea className="json-body" readOnly spellCheck={false} value={JSON.stringify(graph, null, 2)} />
    </ToolWindow>
  );
}

/** A panel an extension added: a window whose content the extension fills, and empties when the window closes. */
export function ExtensionWindow({ panel, onClose }: { panel: ExtensionPanel; onClose: () => void }) {
  const host = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!host.current) return;
    // an extension that fails must not take the interface down with it
    try {
      const unmount = panel.mount(host.current);
      return typeof unmount === 'function' ? unmount : undefined;
    } catch (e) {
      host.current.textContent = e instanceof Error ? e.message : String(e);
      return undefined;
    }
  }, [panel]);
  return (
    <ToolWindow title={panel.title} at={{ x: Math.max(8, window.innerWidth - 560), y: 150 }} onClose={onClose}>
      <div ref={host} className="extension-panel" />
    </ToolWindow>
  );
}
