import { useEffect, useRef, useState, type ReactNode } from 'react';
import type { api } from '../api.ts';
import { useT } from '../i18n/index.tsx';
import type { LogLine } from '../types.ts';
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

/** What the program has been doing lately: its latest log lines, following new ones as they come. */
export function LogsWindow({ daemon, onClose }: { daemon: ReturnType<typeof api>; onClose: () => void }) {
  const t = useT();
  const [lines, setLines] = useState<LogLine[]>([]);
  const last = useRef(0);
  const box = useRef<HTMLPreElement>(null);

  useEffect(() => {
    let alive = true;
    const poll = () =>
      daemon
        .logs(last.current || undefined)
        .then((fresh) => {
          if (!alive || fresh.length === 0) return;
          last.current = fresh[fresh.length - 1].seq;
          setLines((old) => [...old, ...fresh].slice(-500));
        })
        .catch(() => undefined);
    void poll();
    const timer = window.setInterval(poll, 1500);
    return () => {
      alive = false;
      window.clearInterval(timer);
    };
  }, [daemon]);

  // stay at the bottom, where the new lines come, unless the person scrolled up to read
  useEffect(() => {
    const el = box.current;
    if (el && el.scrollHeight - el.scrollTop - el.clientHeight < 80) el.scrollTop = el.scrollHeight;
  }, [lines]);

  return (
    <ToolWindow title={t('app.logs')} at={{ x: Math.max(8, window.innerWidth - 760), y: Math.max(8, window.innerHeight - 380) }} onClose={onClose}>
      <pre ref={box} className="logs-body">
        {lines.length === 0 ? t('app.logsEmpty') : lines.map((l) => l.text).join('\n')}
      </pre>
    </ToolWindow>
  );
}
