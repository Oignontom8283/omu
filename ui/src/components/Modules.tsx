import { useCallback, useEffect, useMemo, useState, type KeyboardEvent } from 'react';
import type { api } from '../api.ts';
import { useT } from '../i18n/index.tsx';
import { Button } from '../kit/controls.tsx';
import { DataTable, type Column } from '../kit/DataTable.tsx';
import { ask } from '../kit/dialogs.tsx';
import { FilterBox, Highlight, Pane, SidePane, ToolSeparator, Toolbar, foldText } from '../kit/Panes.tsx';
import { cleanChoice, cleanFlag, usePersistent } from '../kit/persist.ts';
import { NOTHING, type ListSelection } from '../kit/selectionModel.ts';
import { moduleDocument, moduleStatus, moduleText } from '../modules.ts';
import { formatBytes } from '../soundboard.ts';
import type { ModuleInfo } from '../types.ts';
import { ContextMenu, SEPARATOR, type MenuItem } from './ContextMenu.tsx';
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
 * The optional modules (components that are not part of the program and that people switch on; nothing is downloaded unless they ask), laid
 * out like the extensions view of an IDE: a toolbar (search, which ones to show, the actions on the chosen ones), a table that is sorted and
 * driven with the keyboard, and a pane with the details and the documentation of the chosen module.
 */
export function Modules({ daemon, notify }: Props) {
  const t = useT();
  const [modules, setModules] = useState<ModuleInfo[] | null>(null);
  const [busy, setBusy] = useState(false);
  const [query, setQuery] = useState('');
  const [show, setShow] = usePersistent<Show>('omu-modules-show', 'all', cleanChoice<Show>('all', 'on', 'off'));
  const [details, setDetails] = usePersistent('omu-modules-details', true, cleanFlag);
  const [selection, setSelection] = useState<ListSelection>(NOTHING);
  const [menu, setMenu] = useState<{ x: number; y: number; items: MenuItem[] } | null>(null);
  const [focusFilter, setFocusFilter] = useState(0);

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
  const chosen = all.filter((r) => selection.selected.includes(r.m.id));
  // the details show something from the start: the first module is chosen once the list is known
  useEffect(() => {
    if (selection.focus === null && rows.length > 0) setSelection({ selected: [rows[0].m.id], focus: rows[0].m.id, anchor: rows[0].m.id });
  }, [rows, selection.focus]);
  const focused = all.find((r) => r.m.id === selection.focus) ?? chosen[0];

  // what can be done with some modules: each action applies to the ones it makes sense for
  const canOn = (r: Row) => !r.m.installing && !r.m.enabled && r.m.supported !== false && (r.m.available || r.m.downloadable);
  const canOff = (r: Row) => r.m.enabled && !r.m.installing;
  const canCancel = (r: Row) => !!r.m.installing;
  const canRemove = (r: Row) => r.m.source === 'local' && !r.m.installing;
  const switchOn = (list: Row[]) => void run(() => Promise.all(list.filter(canOn).map((r) => daemon.installModule(r.m.id))));
  const switchOff = (list: Row[]) => void run(() => Promise.all(list.filter(canOff).map((r) => daemon.enableModule(r.m.id, false))));
  const cancel = (list: Row[]) => void run(() => Promise.all(list.filter(canCancel).map((r) => daemon.cancelModule(r.m.id))));
  const remove = async (list: Row[], e?: { shiftKey?: boolean }) => {
    const targets = list.filter(canRemove);
    if (targets.length === 0) return;
    const name = targets.map((r) => r.name).join(', ');
    if (!(await ask({ title: t('dialog.removeModule.title', { name }), message: t('dialog.removeModule.message') }, e))) return;
    await run(() => Promise.all(targets.map((r) => daemon.removeModule(r.m.id))));
  };
  const onLabel = (list: Row[]) => (list.length === 1 && !list[0].m.available && list[0].m.downloadable ? t('modules.install', { mb: list[0].m.download_mb }) : t('modules.switchOn'));

  const info = useInfoWindows((key) => {
    const m = modules?.find((x) => `module:${x.id}` === key);
    return m ? { source: moduleDocument(m, t) } : null;
  }, {});

  const menuFor = (at: { x: number; y: number }, list: Row[]) => {
    setMenu({
      ...at,
      items:
        list.length === 0
          ? [{ label: t('modules.refresh'), keys: 'F5', run: refresh }]
          : [
              { label: onLabel(list), disabled: busy || !list.some(canOn), run: () => switchOn(list) },
              { label: t('modules.switchOff'), disabled: busy || !list.some(canOff), run: () => switchOff(list) },
              { label: t('modules.cancel'), disabled: busy || !list.some(canCancel), run: () => cancel(list) },
              SEPARATOR,
              { label: t('modules.openDoc'), run: () => list.forEach((r, i) => info.open(`module:${r.m.id}`, { x: at.x + 12 + i * 24, y: at.y + i * 24 })) },
              SEPARATOR,
              { label: t('modules.remove'), danger: true, keys: 'Del', disabled: busy || !list.some(canRemove), run: (e) => void remove(list, e as { shiftKey?: boolean }) },
            ],
    });
  };

  const stateText = (r: Row) => (r.m.supported === false ? t('modules.state.unsupported') : t(`modules.state.${r.state}`));
  const columns: Column<Row>[] = [
    {
      id: 'name',
      label: t('modules.col.name'),
      width: 200,
      fixed: true,
      sort: (r) => r.name,
      render: (r) => (
        <>
          <span className={`state-dot ${r.m.error ? 'bad' : DOT[r.state]}`} />
          <strong className="ellipsis">
            <Highlight text={r.name} query={query} />
          </strong>
        </>
      ),
    },
    { id: 'state', label: t('modules.col.state'), width: 150, sort: (r) => r.state, render: (r) => (r.m.installing ? <Progress m={r.m} /> : <span className="ellipsis">{stateText(r)}</span>) },
    { id: 'version', label: t('modules.col.version'), width: 110, sort: (r) => r.m.version ?? '', render: (r) => <span className="ellipsis">{r.m.version ?? '—'}</span> },
    { id: 'source', label: t('modules.col.source'), width: 150, sort: (r) => r.m.source ?? '', render: (r) => <span className="ellipsis muted-cell">{r.m.available ? t(r.m.source === 'local' ? 'modules.source.local' : 'modules.source.system') : '—'}</span> },
    { id: 'size', label: t('modules.col.download'), width: 90, align: 'end', sort: (r) => (r.m.downloadable ? r.m.download_mb : -1), render: (r) => (r.m.downloadable ? t('modules.sizeMb', { mb: r.m.download_mb }) : '—') },
    { id: 'description', label: t('modules.col.description'), width: 320, render: (r) => <span className="ellipsis muted-cell" title={r.description}><Highlight text={r.description} query={query} /></span> },
  ];

  const pageKeys = (e: KeyboardEvent) => {
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'f') {
      e.preventDefault();
      setFocusFilter((n) => n + 1);
    } else if (e.key === 'F5') {
      e.preventDefault();
      refresh();
    }
  };

  const one = chosen.length > 0 ? chosen : focused ? [focused] : [];
  return (
    <div className="ide-page modules-page" aria-label={t('modules.title')} onKeyDown={pageKeys}>
      <Toolbar label={t('modules.title')}>
        <FilterBox value={query} onChange={setQuery} placeholder={t('modules.filter')} count={String(rows.length)} focusKey={focusFilter} />
        <div className="segmented" role="group" aria-label={t('modules.show')}>
          {(['all', 'on', 'off'] as Show[]).map((s) => (
            <Button key={s} size="small" pressed={show === s} onClick={() => setShow(s)}>
              {t(`modules.show.${s}`)}
            </Button>
          ))}
        </div>
        <ToolSeparator />
        <Button size="small" variant="primary" icon="power" disabled={busy || !one.some(canOn)} onClick={() => switchOn(one)}>
          {onLabel(one)}
        </Button>
        <Button size="small" icon="power" disabled={busy || !one.some(canOff)} onClick={() => switchOff(one)}>
          {t('modules.switchOff')}
        </Button>
        {one.some(canCancel) && (
          <Button size="small" icon="close" disabled={busy} onClick={() => cancel(one)}>
            {t('modules.cancel')}
          </Button>
        )}
        <Button size="small" icon="trash" disabled={busy || !one.some(canRemove)} title={`${t('modules.remove')} — ${t('dialog.shiftHint')}`} onClick={(e) => void remove(one, e)} />
        <span className="spacer" />
        <span className="tool-text">{t('modules.count', { on: all.filter((r) => r.state === 'on').length, count: all.length })}</span>
        <Button size="small" icon="refresh" title={`${t('modules.refresh')} (F5)`} onClick={refresh} />
        <Button size="small" icon="sideRight" pressed={details} title={t('modules.details')} onClick={() => setDetails(!details)} />
      </Toolbar>
      <div className="ide-columns">
        <div className="ide-center">
          <DataTable
            id="modules"
            label={t('modules.title')}
            rows={rows}
            rowKey={(r) => r.m.id}
            columns={columns}
            selection={selection}
            onSelection={setSelection}
            onActivate={(r) => info.open(`module:${r.m.id}`, { x: window.innerWidth / 2 - 220, y: 140 })}
            onKey={(e, list) => {
              if (e.key === 'Delete') {
                void remove(list, e);
                return true;
              }
              return false;
            }}
            onMenu={menuFor}
            rowClass={(r) => (r.m.error ? 'failed' : r.state === 'on' ? 'on' : '')}
            empty={modules === null ? t('modules.loading') : all.length === 0 ? t('modules.none') : t('modules.noMatch')}
            rowHeight={30}
          />
        </div>
        {details && (
          <SidePane id="modules-details" side="right" usual={400} min={260} max={800}>
            <Pane title={t('modules.details')} actions={<Button variant="flat" size="small" icon="close" title={t('app.close')} onClick={() => setDetails(false)} />}>
              {focused ? (
                <ModuleDetails row={focused} busy={busy} onOn={() => switchOn([focused])} onOff={() => switchOff([focused])} onCancel={() => cancel([focused])} onRemove={(e) => void remove([focused], e)} onLabel={onLabel([focused])} stateText={stateText(focused)} />
              ) : (
                <p className="pane-empty">{t('modules.pick')}</p>
              )}
            </Pane>
          </SidePane>
        )}
      </div>
      {menu && <ContextMenu x={menu.x} y={menu.y} items={menu.items} onClose={() => setMenu(null)} />}
      {info.windows}
    </div>
  );
}

/** How far a download is: a bar, and the size received. */
function Progress({ m }: { m: ModuleInfo }) {
  const p = m.installing!;
  return (
    <span className="mod-progress" role="status">
      <progress max={p.total || undefined} value={p.total ? p.done : undefined} />
      <small>{p.total ? `${Math.round((p.done / p.total) * 100)} %` : formatBytes(p.done)}</small>
    </span>
  );
}

/** Everything about one module: where it stands, its actions, what went wrong, and its documentation. */
function ModuleDetails({ row, busy, onOn, onOff, onCancel, onRemove, onLabel, stateText }: { row: Row; busy: boolean; onOn: () => void; onOff: () => void; onCancel: () => void; onRemove: (e: React.MouseEvent) => void; onLabel: string; stateText: string }) {
  const t = useT();
  const m = row.m;
  return (
    <div className="mod-details">
      <header className="mod-head">
        <h2>{row.name}</h2>
        <span className={`mod-state ${row.state}`}>
          <span className={`state-dot ${m.error ? 'bad' : DOT[row.state]}`} /> {stateText}
        </span>
      </header>
      <div className="mod-actions">
        {m.installing ? (
          <Button size="small" icon="close" disabled={busy} onClick={onCancel}>
            {t('modules.cancel')}
          </Button>
        ) : m.enabled ? (
          <Button size="small" icon="power" disabled={busy} onClick={onOff}>
            {t('modules.switchOff')}
          </Button>
        ) : (
          m.supported !== false &&
          (m.available || m.downloadable) && (
            <Button size="small" variant="primary" icon={m.available ? 'power' : 'download'} disabled={busy} onClick={onOn}>
              {onLabel}
            </Button>
          )
        )}
        {m.source === 'local' && !m.installing && (
          <Button size="small" icon="trash" disabled={busy} title={t('dialog.shiftHint')} onClick={onRemove}>
            {t('modules.remove')}
          </Button>
        )}
      </div>
      {m.installing && (
        <div className="mod-download" role="status">
          <progress max={m.installing.total || undefined} value={m.installing.total ? m.installing.done : undefined} />
          <small className="muted">{m.installing.total ? `${formatBytes(m.installing.done)} / ${formatBytes(m.installing.total)}` : formatBytes(m.installing.done)}</small>
        </div>
      )}
      {m.error && (
        <p className="warn" role="alert">
          {m.error}
        </p>
      )}
      {m.supported === false && <p className="warn">{t('modules.notSupported')}</p>}
      {!m.available && !m.installing && m.supported !== false && <p className="muted">{m.downloadable ? t('modules.downloadNote', { mb: m.download_mb }) : t('modules.noDownload')}</p>}
      <div className="mod-doc">
        {/* the head of the pane already has the name and what it does */}
        <Markdown source={moduleDocument(m, t).replace(/^# .*\n\n.*\n\n/, '')} links={{}} />
      </div>
    </div>
  );
}
