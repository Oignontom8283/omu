import { useEffect, useRef, useState, type KeyboardEvent as ReactKeyboardEvent, type PointerEvent as ReactPointerEvent, type ReactNode } from 'react';
import { ON_THE_DAEMON_COMPUTER, saveTextFile, type api } from '../api.ts';
import { nameOf, portsOf, categoryOf } from '../graph.ts';
import { owningBlock } from '../grouping.ts';
import { useEditor } from '../context.ts';
import { useT } from '../i18n/index.tsx';
import { Button } from '../kit/controls.tsx';
import { formatBytes } from '../soundboard.ts';
import { PRODUCT_NAME, type Diagnostic, type EngineStatus, type GraphSpec, type LogLine, type NodeSpec, type SoundStatus } from '../types.ts';
import { differences, same, type Version } from '../versions.ts';
import { Icon, type IconName } from './Icon.tsx';
import { InlineValue, Params } from './NodeCard.tsx';
import { formatDateTime } from '../i18n/core.ts';

// The places around the graph, as in a code editor: a panel under it (problems, activity log, monitor), a panel beside it
// (the inspector of the selected block), a bar at the bottom with the counts and the buttons that open them. Their sizes and
// what is open are remembered.

export type DockState = { bottom: 'problems' | 'logs' | 'monitor' | 'versions' | null; right: 'inspector' | null; bottomHeight: number; rightWidth: number };

const KEY = 'omu-workbench';
const BOTTOM_PANELS: NonNullable<DockState['bottom']>[] = ['problems', 'logs', 'monitor', 'versions'];
const DEFAULT: DockState = { bottom: null, right: null, bottomHeight: 220, rightWidth: 300 };

/** What is open and how big, kept in this browser. */
export function loadDock(): DockState {
  try {
    const saved = JSON.parse(localStorage.getItem(KEY) ?? 'null') as Partial<Record<keyof DockState, unknown>> | null;
    const size = (v: unknown, fallback: number) => (typeof v === 'number' && Number.isFinite(v) && v >= 80 && v <= 4000 ? v : fallback);
    return {
      bottom: BOTTOM_PANELS.find((p) => p === saved?.bottom) ?? null,
      right: saved?.right === 'inspector' ? 'inspector' : null,
      bottomHeight: size(saved?.bottomHeight, DEFAULT.bottomHeight),
      rightWidth: size(saved?.rightWidth, DEFAULT.rightWidth),
    };
  } catch {
    return DEFAULT;
  }
}
export function saveDock(dock: DockState): void {
  try {
    localStorage.setItem(KEY, JSON.stringify(dock));
  } catch {
    /* storage may be blocked */
  }
}

/** What the sizes of the panels are at first, and what a double click on a bar gives back. */
export const USUAL_SIZE = { bottomHeight: DEFAULT.bottomHeight, rightWidth: DEFAULT.rightWidth };

/**
 * The bar between two areas, dragged (or moved with the arrow keys) to change the size of the panel that follows it: the panel is
 * `size` big, and dragging towards the graph makes it bigger. The size is worked out from where the drag began, never from the
 * last move, so the panel follows the pointer exactly; a double click gives the usual size back.
 */
export function Splitter({
  direction,
  size,
  min,
  max,
  usual,
  label,
  onSize,
}: {
  direction: 'horizontal' | 'vertical';
  size: number;
  min: number;
  max: number;
  usual: number;
  label: string;
  onSize: (size: number) => void;
}) {
  const horizontal = direction === 'horizontal';
  const clamp = (n: number) => Math.round(Math.min(max, Math.max(min, n)));
  const [dragging, setDragging] = useState(false);
  // the drag is followed on the whole window, so that it goes on over the graph and out of the page's frame
  const press = (e: ReactPointerEvent<HTMLDivElement>) => {
    if (e.button !== 0) return;
    e.preventDefault();
    const at = horizontal ? e.clientY : e.clientX;
    const from = size;
    const move = (ev: PointerEvent) => onSize(clamp(from - ((horizontal ? ev.clientY : ev.clientX) - at)));
    const stop = () => {
      window.removeEventListener('pointermove', move);
      window.removeEventListener('pointerup', stop);
      window.removeEventListener('pointercancel', stop);
      document.body.classList.remove('resizing');
      setDragging(false);
    };
    window.addEventListener('pointermove', move);
    window.addEventListener('pointerup', stop);
    window.addEventListener('pointercancel', stop);
    document.body.classList.add('resizing');
    setDragging(true);
  };
  const key = (e: ReactKeyboardEvent<HTMLDivElement>) => {
    const grow = horizontal ? 'ArrowUp' : 'ArrowLeft';
    const shrink = horizontal ? 'ArrowDown' : 'ArrowRight';
    const step = e.shiftKey ? 64 : 16;
    if (e.key === grow) onSize(clamp(size + step));
    else if (e.key === shrink) onSize(clamp(size - step));
    else if (e.key === 'Home') onSize(max);
    else if (e.key === 'End') onSize(min);
    else if (e.key === 'Enter') onSize(clamp(usual));
    else return;
    e.preventDefault();
  };
  return (
    <div
      className={`splitter ${direction}${dragging ? ' dragging' : ''}`}
      role="separator"
      aria-orientation={horizontal ? 'horizontal' : 'vertical'}
      aria-label={label}
      aria-valuenow={size}
      aria-valuemin={min}
      aria-valuemax={max}
      tabIndex={0}
      onPointerDown={press}
      onDoubleClick={() => onSize(clamp(usual))}
      onKeyDown={key}
    />
  );
}

const SEVERITY_ICON: Record<Diagnostic['severity'], IconName> = { error: 'error', warning: 'warning', info: 'info' };

/** What is wrong with the graph, as the daemon says it: each line leads to the block (and the connection point) it is about. */
export function ProblemsPanel({ diagnostics, nodes, onGo }: { diagnostics: Diagnostic[]; nodes: NodeSpec[]; onGo: (d: Diagnostic) => void }) {
  const t = useT();
  if (diagnostics.length === 0) return <p className="panel-empty">{t('problems.none')}</p>;
  const nameFor = (id: string) => {
    const spec = nodes.find((n) => n.id === owningBlock(id, nodes));
    return spec ? nameOf(spec) || t(`kind.${spec.type}`) : id;
  };
  return (
    <ul className="problems">
      {diagnostics.map((d, i) => (
        <li key={i} className={d.severity} onClick={() => d.node && onGo(d)} role="button" tabIndex={0} onKeyDown={(e) => e.key === 'Enter' && d.node && onGo(d)}>
          <Icon name={SEVERITY_ICON[d.severity]} />
          <div>
            <div className="message">
              {d.message} <code className="code-tag">{d.code}</code>
            </div>
            {(d.node || d.hint) && (
              <small>
                {d.node && (
                  <span className="where">
                    {nameFor(d.node)}
                    {d.port ? ` · ${d.port}` : ''}
                  </span>
                )}
                {d.hint && <span className="hint-text"> {d.hint}</span>}
              </small>
            )}
          </div>
        </li>
      ))}
    </ul>
  );
}

/** What the program has been doing lately: its latest log lines, following new ones as they come. */
export function LogsPanel({ daemon, notify }: { daemon: ReturnType<typeof api>; notify: (text: string) => void }) {
  const t = useT();
  const [lines, setLines] = useState<LogLine[]>([]);
  const last = useRef(0);
  const opens = useRef(daemon.opens());
  const box = useRef<HTMLPreElement>(null);
  useEffect(() => {
    let alive = true;
    const poll = () => {
      // the daemon began again (its numbering with it): the old lines are another run's, and the new ones start at 1
      if (daemon.opens() !== opens.current) {
        opens.current = daemon.opens();
        last.current = 0;
        setLines([]);
      }
      return daemon
        .logs(last.current || undefined)
        .then((fresh) => {
          if (!alive || fresh.length === 0) return;
          last.current = fresh[fresh.length - 1].seq;
          setLines((old) => [...old, ...fresh].slice(-500));
        })
        .catch(() => undefined);
    };
    void poll();
    const timer = window.setInterval(poll, 1500);
    return () => {
      alive = false;
      window.clearInterval(timer);
    };
  }, [daemon]);
  // follows the new lines while the person reads the end of the log; scrolling up to read something stops that
  const following = useRef(true);
  useEffect(() => {
    const el = box.current;
    if (el && following.current) el.scrollTop = el.scrollHeight;
  }, [lines]);
  const text = lines.map((l) => l.text).join('\n');
  const [copied, setCopied] = useState(false);
  const failed = (e: unknown) => notify(e instanceof Error ? e.message : String(e));
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(text);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1500);
    } catch (e) {
      failed(e);
    }
  };
  const save = () => {
    const day = new Date().toISOString().slice(0, 10);
    const product = PRODUCT_NAME.toLowerCase().replace(/[^a-z0-9]+/g, '-');
    void saveTextFile(`${product}-logs-${day}.log`, `${text}\n`).catch(failed);
  };
  return (
    <div className="logs">
      <div className="logs-bar">
        <Button icon={copied ? 'check' : 'copy'} disabled={lines.length === 0} onClick={() => void copy()}>
          {copied ? t('logs.copied') : t('logs.copy')}
        </Button>
        <Button icon="save" disabled={lines.length === 0} onClick={save}>
          {t('logs.export')}
        </Button>
        {ON_THE_DAEMON_COMPUTER && (
          <Button icon="folder" onClick={() => void daemon.openLogsFolder().catch(failed)}>
            {t('logs.openFolder')}
          </Button>
        )}
      </div>
      <pre ref={box} className="logs-panel" onScroll={(e) => (following.current = e.currentTarget.scrollHeight - e.currentTarget.scrollTop - e.currentTarget.clientHeight < 40)}>
        {lines.length === 0 ? t('app.logsEmpty') : text}
      </pre>
    </div>
  );
}

/** The state of the running graph, block by block: the level each one hears, the glitches, the value it makes. */
export function MonitorPanel({ status, nodes, daemon }: { status: EngineStatus | null; nodes: NodeSpec[]; daemon: ReturnType<typeof api> }) {
  const t = useT();
  // the memory the sounds of the soundboards use, read while the panel is open
  const [sounds, setSounds] = useState<SoundStatus | null>(null);
  useEffect(() => {
    const read = () => daemon.soundStatus().then(setSounds).catch(() => undefined);
    void read();
    const timer = setInterval(read, 1500);
    return () => clearInterval(timer);
  }, [daemon]);
  const memory = sounds?.active ? t('monitor.soundMemory', { used: formatBytes(sounds.cache_used_bytes), reserved: formatBytes(sounds.cache_allocated_bytes), limit: formatBytes(sounds.cache_limit_bytes), count: sounds.sounds_loaded }) : t('monitor.soundMemoryIdle');
  if (!status || !status.running) return <p className="panel-empty">{t('monitor.stopped')}</p>;
  return (
    <>
      <p className="monitor-head">
        {t('monitor.cycles', { cycles: status.cycles, late: status.late_cycles })} · {memory}
      </p>
      <table className="monitor">
        <thead>
          <tr>
            <th>{t('monitor.block')}</th>
            <th>{t('monitor.level')}</th>
            <th>{t('monitor.glitches')}</th>
            <th>{t('monitor.value')}</th>
          </tr>
        </thead>
        <tbody>
          {status.nodes.map((n) => {
            const spec = nodes.find((x) => x.id === n.id);
            const db = n.peak > 0 ? Math.max(-60, 20 * Math.log10(n.peak)) : -60;
            return (
              <tr key={n.id} className={n.state === 'failed' ? 'failed' : ''}>
                <td>{spec ? nameOf(spec) || t(`kind.${spec.type}`) : n.id}</td>
                <td>
                  <span className="monitor-bar">
                    <span style={{ width: `${((db + 60) / 60) * 100}%` }} />
                  </span>
                </td>
                <td>{n.underruns + n.overruns || ''}</td>
                <td className="code">{n.state === 'failed' ? n.message : n.state === 'waiting' ? t('status.waitingProgram') : (n.value ?? (n.active != null ? t(n.active ? 'status.yes' : 'status.no') : ''))}</td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </>
  );
}

/** The versions of the graph that were applied: what each one changes compared with what is in the editor now, and a way back to it. */
export function VersionsPanel({ versions, current, onRestore }: { versions: Version[]; current: GraphSpec; onRestore: (v: Version) => void }) {
  const t = useT();
  if (versions.length === 0) return <p className="panel-empty">{t('versions.none')}</p>;
  return (
    <ul className="versions">
      {versions.map((v, i) => {
        const d = differences(current, v.graph);
        return (
          <li key={v.at}>
            <span className="when">{formatDateTime(v.at)}</span>
            <span className="summary muted">
              {i === 0 && same(d) ? t('versions.current') : t('versions.diff', { added: d.removed.length, removed: d.added.length, changed: d.changed.length, links: d.links })}
            </span>
            <Button size="small" disabled={i === 0 && same(d)} onClick={() => onRestore(v)}>
              {t('versions.restore')}
            </Button>
          </li>
        );
      })}
    </ul>
  );
}

/** All the settings of the selected block in one place, with what it does. */
export function InspectorPanel({ spec }: { spec: NodeSpec | undefined }) {
  const t = useT();
  const { statusOf, openHelp } = useEditor();
  if (!spec) return <p className="panel-empty">{t('inspector.none')}</p>;
  const status = statusOf(spec.id);
  const inputs = portsOf(spec).filter((p) => p.inline);
  return (
    <div className="inspector">
      <h3>
        <span className={`swatch cat-${categoryOf(spec.type)}`} /> {t(`kind.${spec.type}`)}
      </h3>
      <p className="muted">{nameOf(spec) || t('node.unnamed')}</p>
      <p>{t(`kind.${spec.type}.hint`).replace(/\[([^\]]+)\]\([^)]*\)/g, '$1')}</p>
      {inputs.length > 0 && (
        <>
          <h4>{t('help.inputs')}</h4>
          <div className="inspector-fields">
            {inputs.map((port) => (
              <div key={port.name} className="inspector-field">
                <span>{t(port.label, { n: port.n ?? '' }).trim()}</span>
                <InlineValue spec={spec} port={port} />
              </div>
            ))}
          </div>
        </>
      )}
      <h4>{t('help.settings')}</h4>
      <div className="inspector-params">
        <Params spec={spec} />
      </div>
      {status?.value != null && (
        <>
          <h4>{t('monitor.value')}</h4>
          <code className="code">{status.value === '' ? t('status.empty') : status.value}</code>
        </>
      )}
      <p className="inspector-actions">
        <Button size="small" onClick={() => openHelp(spec.id, { x: window.innerWidth - 640, y: 120 })}>
          {t('block.help')}
        </Button>
      </p>
    </div>
  );
}

/** A tab of a dock, with its title and its close button. */
export function DockFrame({ title, onClose, children, side = 'bottom' }: { title: ReactNode; onClose: () => void; children: ReactNode; side?: 'bottom' | 'right' }) {
  const t = useT();
  return (
    <section className={`dock dock-${side}`}>
      <header>
        <span className="dock-title">{title}</span>
        <Button variant="flat" size="small" icon="close" title={t('app.close')} onClick={onClose} />
      </header>
      <div className="dock-body">{children}</div>
    </section>
  );
}

/** The bar under everything: how many problems, the buttons that open the panels, and what the engine is doing. */
export function StatusBar({
  dock,
  setDock,
  errors,
  warnings,
  status,
  connected,
  blocks,
  onGraph,
}: {
  dock: DockState;
  setDock: (d: DockState) => void;
  errors: number;
  warnings: number;
  status: EngineStatus | null;
  connected: boolean;
  blocks: number;
  /** The graph is on screen: the inspector shows the settings of its blocks, so it has nothing to show anywhere else. */
  onGraph: boolean;
}) {
  const t = useT();
  const toggleBottom = (panel: NonNullable<DockState['bottom']>) => setDock({ ...dock, bottom: dock.bottom === panel ? null : panel });
  return (
    <footer className="statusbar">
      <Button variant="flat" size="small" pressed={dock.bottom === 'problems'} onClick={() => toggleBottom('problems')} title={t('dock.problems')}>
        <Icon name="error" size={12} /> {errors}
        <Icon name="warning" size={12} /> {warnings}
      </Button>
      <Button variant="flat" size="small" pressed={dock.bottom === 'logs'} onClick={() => toggleBottom('logs')}>
        {t('app.logs')}
      </Button>
      <Button variant="flat" size="small" pressed={dock.bottom === 'versions'} onClick={() => toggleBottom('versions')}>
        {t('dock.versions')}
      </Button>
      <Button variant="flat" size="small" pressed={dock.bottom === 'monitor'} onClick={() => toggleBottom('monitor')}>
        {t('dock.monitor')}
      </Button>
      {onGraph && (
        <Button variant="flat" size="small" pressed={dock.right === 'inspector'} onClick={() => setDock({ ...dock, right: dock.right ? null : 'inspector' })}>
          {t('dock.inspector')}
        </Button>
      )}
      <span className="spacer" />
      <span className="muted">{t('status.blocks', { count: blocks })}</span>
      <span className={`status-dot ${connected ? (status?.running ? 'ok' : 'idle') : 'bad'}`} />
      <span>{!connected ? t('app.disconnected') : status?.running ? t('app.engineRunning') : t('app.engineStopped')}</span>
    </footer>
  );
}
