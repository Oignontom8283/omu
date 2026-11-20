import { useState, type ReactNode } from 'react';
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
