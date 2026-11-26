import { useCallback, useEffect, useMemo, useRef, useState, type KeyboardEvent } from 'react';
import type { api } from '../api.ts';
import { useT } from '../i18n/index.tsx';
import { Button } from '../kit/controls.tsx';
import { isInteractive } from '../kit/DataTable.tsx';
import { ask } from '../kit/dialogs.tsx';
import { FilterBox, Highlight, Toolbar, foldText } from '../kit/Panes.tsx';
import { cleanChoice, usePersistent } from '../kit/persist.ts';
import { moduleDocument, moduleStatus, moduleText } from '../modules.ts';
import { formatBytes } from '../soundboard.ts';
import type { ModuleInfo } from '../types.ts';
import { ContextMenu, SEPARATOR, type MenuItem } from './ContextMenu.tsx';
import { Icon } from './Icon.tsx';
import { useInfoWindows } from './InfoWindows.tsx';
import { Markdown } from './Markdown.tsx';

type Props = {
  daemon: ReturnType<typeof api>;
  notify: (text: string) => void;
};

type Show = 'all' | 'on' | 'off';
type Row = { m: ModuleInfo; name: string; description: string; state: ReturnType<typeof moduleStatus> };

const message = (e: unknown): string => (e instanceof Error ? e.message : String(e));
const DOT: Record<Row['state'], string> = { on: 'on', ready: 'ready', missing: '', installing: 'busy' };

/**
 * The optional modules (components that are not part of the program and that people switch on; nothing is downloaded unless they ask), as
 * a list of blocks in the middle of the page: each one says what it does and where it stands, with its action at its right; a click (or
 * Enter) opens it on its documentation. A search and a choice of which ones to show sit above; the arrows go from one to the next, Delete
 * removes a downloaded copy, and a right click gives the rest.
 */
export function Modules({ daemon, notify }: Props) {
  const t = useT();
  const [modules, setModules] = useState<ModuleInfo[] | null>(null);
  const [busy, setBusy] = useState(false);
  const [query, setQuery] = useState('');
  const [show, setShow] = usePersistent<Show>('omu-modules-show', 'all', cleanChoice<Show>('all', 'on', 'off'));
  const [open, setOpen] = useState<string[]>([]);
  const [menu, setMenu] = useState<{ x: number; y: number; items: MenuItem[] } | null>(null);
  const [focusFilter, setFocusFilter] = useState(0);
  const list = useRef<HTMLDivElement>(null);

  const refresh = useCallback(() => {
    daemon.modules().then(setModules).catch(() => undefined);
  }, [daemon]);

  // a download is followed closely; otherwise the list is looked at now and then
  const downloading = modules?.some((m) => m.installing) ?? false;
  useEffect(() => {
    refresh();
    const timer = setInterval(refresh, downloading ? 500 : 3000);
    return () => clearInterval(timer);
  }, [refresh, downloading]);

  const run = async (work: () => Promise<unknown>) => {
    setBusy(true);
    try {
      await work();
    } catch (e) {
      notify(message(e));
    } finally {
      setBusy(false);
      refresh();
    }
  };

  const all: Row[] = useMemo(() => (modules ?? []).map((m) => ({ m, name: moduleText(m, 'name', t), description: moduleText(m, 'description', t), state: moduleStatus(m) })), [modules, t]);
  const rows = all.filter((r) => {
    if (show === 'on' && r.state !== 'on') return false;
    if (show === 'off' && r.state === 'on') return false;
    const q = foldText(query.trim());
    return !q || foldText(`${r.name} ${r.description} ${r.m.id}`).includes(q);
  });

  const canOn = (r: Row) => !r.m.installing && !r.m.enabled && r.m.supported !== false && (r.m.available || r.m.downloadable);
  const canOff = (r: Row) => r.m.enabled && !r.m.installing;
  const canRemove = (r: Row) => r.m.source === 'local' && !r.m.installing;
  const switchOn = (r: Row) => void run(() => daemon.installModule(r.m.id));
  const switchOff = (r: Row) => void run(() => daemon.enableModule(r.m.id, false));
  const cancel = (r: Row) => void run(() => daemon.cancelModule(r.m.id));
  const remove = async (r: Row, e?: { shiftKey?: boolean }) => {
    if (!canRemove(r)) return;
    if (!(await ask({ title: t('dialog.removeModule.title', { name: r.name }), message: t('dialog.removeModule.message') }, e))) return;
    await run(() => daemon.removeModule(r.m.id));
  };
  const onLabel = (r: Row) => (!r.m.available && r.m.downloadable ? t('modules.install', { mb: r.m.download_mb }) : t('modules.switchOn'));
  const stateText = (r: Row) => (r.m.supported === false ? t('modules.state.unsupported') : t(`modules.state.${r.state}`));
  const toggleOpen = (id: string) => setOpen((o) => (o.includes(id) ? o.filter((x) => x !== id) : [...o, id]));

  const info = useInfoWindows((key) => {
    const m = modules?.find((x) => `module:${x.id}` === key);
    return m ? { source: moduleDocument(m, t) } : null;
  }, {});

  const menuFor = (at: { x: number; y: number }, r: Row) =>
    setMenu({
      ...at,
      items: [
        { label: onLabel(r), disabled: busy || !canOn(r), run: () => switchOn(r) },
        { label: t('modules.switchOff'), disabled: busy || !canOff(r), run: () => switchOff(r) },
        ...(r.m.installing ? [{ label: t('modules.cancel'), disabled: busy, run: () => cancel(r) }] : []),
        SEPARATOR,
        { label: open.includes(r.m.id) ? t('modules.less') : t('modules.more'), keys: 'Enter', run: () => toggleOpen(r.m.id) },
        { label: t('modules.openDoc'), run: () => info.open(`module:${r.m.id}`, { x: at.x + 12, y: at.y }) },
        SEPARATOR,
        { label: t('modules.remove'), danger: true, keys: 'Del', disabled: busy || !canRemove(r), run: (e) => void remove(r, e as { shiftKey?: boolean }) },
      ],
    });

  const focusCard = (index: number) => list.current?.querySelectorAll<HTMLElement>('.mod-card')[index]?.focus();
  const cardKeys = (e: KeyboardEvent<HTMLElement>, r: Row, index: number) => {
    if (e.target !== e.currentTarget) return;
    if (e.key === 'ArrowDown') focusCard(Math.min(rows.length - 1, index + 1));
    else if (e.key === 'ArrowUp') focusCard(Math.max(0, index - 1));
    else if (e.key === 'Home') focusCard(0);
    else if (e.key === 'End') focusCard(rows.length - 1);
    else if (e.key === 'Enter' || e.key === ' ') toggleOpen(r.m.id);
    else if (e.key === 'Delete') void remove(r, e);
    else if (e.key === 'ContextMenu' || (e.key === 'F10' && e.shiftKey)) {
      const b = e.currentTarget.getBoundingClientRect();
      menuFor({ x: b.left + 40, y: b.top + 30 }, r);
    } else return;
    e.preventDefault();
  };

  const pageKeys = (e: KeyboardEvent) => {
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'f') {
      e.preventDefault();
      setFocusFilter((n) => n + 1);
    } else if (e.key === 'F5') {
      e.preventDefault();
      refresh();
    }
  };

  return (
    <div className="ide-page modules-page" aria-label={t('modules.title')} onKeyDown={pageKeys}>
      <Toolbar label={t('modules.title')}>
        <FilterBox value={query} onChange={setQuery} placeholder={t('modules.filter')} count={query.trim() ? String(rows.length) : undefined} focusKey={focusFilter} />
        <div className="segmented" role="group" aria-label={t('modules.show')}>
          {(['all', 'on', 'off'] as Show[]).map((s) => (
            <Button key={s} size="small" pressed={show === s} onClick={() => setShow(s)}>
              {t(`modules.show.${s}`)}
            </Button>
          ))}
        </div>
        <span className="spacer" />
        <span className="tool-text">{t('modules.count', { on: all.filter((r) => r.state === 'on').length, count: all.length })}</span>
        <Button size="small" icon="refresh" title={`${t('modules.refresh')} (F5)`} onClick={refresh} />
      </Toolbar>
      <div className="mod-scroll">
        <div ref={list} className="mod-list" role="list" aria-label={t('modules.title')}>
          {rows.length === 0 && <p className="pane-empty">{modules === null ? t('modules.loading') : all.length === 0 ? t('modules.none') : t('modules.noMatch')}</p>}
          {rows.map((r, index) => {
            const m = r.m;
            const isOpen = open.includes(m.id);
            const facts = [m.available ? t(m.source === 'local' ? 'modules.source.local' : 'modules.source.system') : null, m.version, !m.available && m.downloadable && m.supported !== false ? t('modules.sizeMb', { mb: m.download_mb }) : null].filter(Boolean);
            return (
              <article
                key={m.id}
                className={`mod-card ${r.state}${m.error ? ' failed' : ''}${isOpen ? ' open' : ''}`}
                role="listitem"
                tabIndex={0}
                aria-expanded={isOpen}
                aria-label={r.name}
                onClick={(e) => {
                  if (!isInteractive(e.target) && !(e.target as HTMLElement).closest('.mod-card-doc')) toggleOpen(m.id);
                }}
                onKeyDown={(e) => cardKeys(e, r, index)}
                onContextMenu={(e) => {
                  if ((e.target as HTMLElement).closest('.mod-card-doc')) return;
                  e.preventDefault();
                  menuFor({ x: e.clientX, y: e.clientY }, r);
                }}
              >
                <div className="mod-card-main">
                  <span className="mod-chevron">
                    <Icon name="chevron" size={14} />
                  </span>
                  <div className="mod-card-text">
                    <div className="mod-card-title">
                      <strong>
                        <Highlight text={r.name} query={query} />
                      </strong>
                      <span className={`mod-badge ${m.error ? 'bad' : r.state}${m.supported === false ? ' unsupported' : ''}`}>
                        <span className={`state-dot ${m.error ? 'bad' : DOT[r.state]}`} />
                        {stateText(r)}
                      </span>
                    </div>
                    <p className="mod-card-desc">
                      <Highlight text={r.description} query={query} />
                    </p>
                    {facts.length > 0 && <p className="mod-card-facts">{facts.join(' · ')}</p>}
                  </div>
                  <div className="mod-card-actions">
                    {m.installing ? (
                      <Button size="small" icon="close" disabled={busy} onClick={() => cancel(r)}>
                        {t('modules.cancel')}
                      </Button>
                    ) : m.enabled ? (
                      <Button size="small" icon="power" disabled={busy} onClick={() => switchOff(r)}>
                        {t('modules.switchOff')}
                      </Button>
                    ) : (
                      canOn(r) && (
                        <Button size="small" variant="primary" icon={m.available ? 'power' : 'download'} disabled={busy} onClick={() => switchOn(r)}>
                          {onLabel(r)}
                        </Button>
                      )
                    )}
                    <Button variant="flat" size="small" icon="more" title={t('settings.more')} onClick={(e) => menuFor({ x: e.clientX, y: e.clientY }, r)} />
                  </div>
                </div>
                {m.installing && (
                  <div className="mod-download" role="status">
                    <progress max={m.installing.total || undefined} value={m.installing.total ? m.installing.done : undefined} />
                    <small>{m.installing.total ? `${formatBytes(m.installing.done)} / ${formatBytes(m.installing.total)}` : formatBytes(m.installing.done)}</small>
                  </div>
                )}
                {m.error && (
                  <p className="mod-card-note warn" role="alert">
                    {m.error}
                  </p>
                )}
                {m.supported === false && <p className="mod-card-note warn">{t('modules.notSupported')}</p>}
                {isOpen && (
                  <div className="mod-card-doc">
                    {!m.available && !m.installing && m.supported !== false && <p>{m.downloadable ? t('modules.downloadNote', { mb: m.download_mb }) : t('modules.noDownload')}</p>}
                    {/* the head of the block already has the name and what it does */}
                    <Markdown source={moduleDocument(m, t).replace(/^# .*\n\n.*\n\n/, '')} links={{}} />
                    {canRemove(r) && (
                      <Button size="small" icon="trash" disabled={busy} title={t('dialog.shiftHint')} onClick={(e) => void remove(r, e)}>
                        {t('modules.remove')}
                      </Button>
                    )}
                  </div>
                )}
              </article>
            );
          })}
        </div>
      </div>
      {menu && <ContextMenu x={menu.x} y={menu.y} items={menu.items} onClose={() => setMenu(null)} />}
      {info.windows}
    </div>
  );
}
