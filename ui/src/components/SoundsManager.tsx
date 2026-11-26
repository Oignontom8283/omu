import { useMemo, useState } from 'react';
import { ON_THE_DAEMON_COMPUTER, type api } from '../api.ts';
import type { SoundImports } from '../imports.tsx';
import { useT } from '../i18n/index.tsx';
import { Button } from '../kit/controls.tsx';
import { DataTable, type Column } from '../kit/DataTable.tsx';
import { FilterBox, Highlight, Toolbar, foldText } from '../kit/Panes.tsx';
import { NOTHING, type ListSelection } from '../kit/selectionModel.ts';
import { formatBytes, formatDuration, padsUsing } from '../soundboard.ts';
import { useSoundLibrary } from '../soundLibrary.ts';
import type { SoundInfo, SoundLibrary as Library } from '../types.ts';
import { ContextMenu, SEPARATOR, type MenuItem } from './ContextMenu.tsx';
import { ImportCard } from './PadCard.tsx';

type Row = { s: SoundInfo; uses: number };

/**
 * Every sound that was imported, as a table of an IDE: filtered, sorted by any column, several chosen at once (Ctrl, Shift, Ctrl+A), with
 * a menu on the chosen ones (add them to the soundboard on screen, compress, show the file, delete) and how the memory is used. It is the
 * library pane of the soundboards and a page of the settings: the sounds belong to the program, not to one soundboard.
 */
export function SoundsManager({
  daemon,
  library,
  setLibrary,
  imports,
  notify,
  onAddToBoard,
  boardName,
}: {
  daemon: ReturnType<typeof api>;
  library: Library;
  setLibrary: (l: Library) => void;
  imports: SoundImports;
  notify: (text: string) => void;
  /** Adds sounds to the soundboard on screen (none outside the soundboards). */
  onAddToBoard?: (sounds: SoundInfo[]) => void;
  boardName?: string;
}) {
  const t = useT();
  const lib = useSoundLibrary({ daemon, setLibrary, notify });
  const [query, setQuery] = useState('');
  const [selection, setSelection] = useState<ListSelection>(NOTHING);
  const [menu, setMenu] = useState<{ x: number; y: number; items: MenuItem[] } | null>(null);
  const mine = imports.entries.filter((e) => e.owner === 'library');

  const rows = useMemo(() => {
    const q = foldText(query.trim());
    return library.sounds.filter((s) => !q || foldText(s.name).includes(q)).map((s): Row => ({ s, uses: padsUsing(library.boards, s.id) }));
  }, [library, query]);
  const total = library.sounds.reduce((n, s) => n + s.bytes, 0);
  const status = lib.status;
  const reveal = (s: SoundInfo) => void daemon.revealSound(s.id).catch((e) => notify(e instanceof Error ? e.message : String(e)));
  const add = (list: Row[]) => onAddToBoard?.(list.map((r) => r.s));

  const menuFor = (at: { x: number; y: number }, list: Row[]) =>
    setMenu({
      ...at,
      items:
        list.length === 0
          ? [{ label: t('soundboards.addSounds'), run: () => imports.choose('library', () => lib.reload()) }]
          : [
              ...(onAddToBoard ? [{ label: boardName ? t('library.addTo', { name: boardName }) : t('library.addToBoard'), keys: 'Enter', run: () => add(list) }, SEPARATOR] : []),
              { label: t('soundboards.compress'), disabled: lib.compressing !== null || list.every((r) => r.s.compressed), run: () => void (async () => { for (const r of list) if (!r.s.compressed) await lib.compressSound(r.s); })() },
              ...(ON_THE_DAEMON_COMPUTER ? [{ label: t('soundboards.showFileShort'), disabled: list.length !== 1, run: () => reveal(list[0].s) }] : []),
              SEPARATOR,
              { label: t('soundboards.deleteMenu'), danger: true, keys: 'Del', run: (e) => void lib.removeSounds(list.map((r) => r.s), e as { shiftKey?: boolean }) },
            ],
    });

  const columns: Column<Row>[] = [
    { id: 'name', label: t('soundboards.col.name'), width: 280, fixed: true, sort: (r) => r.s.name, render: (r) => <span className="ellipsis" title={r.s.name}><Highlight text={r.s.name} query={query} /></span> },
    { id: 'duration', label: t('soundboards.col.duration'), width: 80, align: 'end', sort: (r) => r.s.frames, render: (r) => formatDuration(r.s.frames) },
    { id: 'size', label: t('soundboards.col.size'), width: 90, align: 'end', sort: (r) => r.s.bytes, render: (r) => formatBytes(r.s.bytes) },
    { id: 'channels', label: t('library.col.channels'), width: 80, align: 'end', hidden: true, sort: (r) => r.s.channels, render: (r) => r.s.channels },
    { id: 'format', label: t('soundboards.col.format'), width: 100, sort: (r) => (r.s.compressed ? 1 : 0), render: (r) => <span className="muted-cell">{r.s.compressed ? t('soundboards.compressed') : t('soundboards.raw')}</span> },
    { id: 'uses', label: t('soundboards.col.uses'), width: 70, align: 'end', sort: (r) => r.uses, render: (r) => r.uses || <span className="muted-cell">0</span> },
  ];

  return (
    <div className="sounds-manager">
      <Toolbar label={t('soundboards.libraryTitle')} className="inner">
        <Button size="small" icon="plus" onClick={() => imports.choose('library', () => lib.reload())}>
          {t('soundboards.addSounds')}
        </Button>
        <FilterBox value={query} onChange={setQuery} placeholder={t('soundboards.filter')} count={String(rows.length)} />
        <span className="tool-text">{t('soundboards.libraryTotal', { count: library.sounds.length, size: formatBytes(total) })}</span>
        <span className="spacer" />
        <span className={`tool-text${status && status.late_frames > 0 ? ' warn' : ''}`} title={status && status.late_frames > 0 ? t('soundboards.late') : undefined}>
          {status?.active ? t('soundboards.memoryUsed', { used: formatBytes(status.cache_used_bytes), limit: formatBytes(status.cache_limit_bytes), count: status.sounds_loaded }) : t('library.memoryIdle')}
          {status && status.streams > 0 ? ` · ${t('soundboards.streams', { count: status.streams })}` : ''}
        </span>
      </Toolbar>
      {mine.length > 0 && (
        <div className="imports-strip">
          {mine.map((i) => (
            <ImportCard key={i.job} job={imports.progress[i.job]} label={i.label} error={i.error} onDismiss={() => imports.dismiss(i.job)} />
          ))}
        </div>
      )}
      <DataTable
        id="library"
        label={t('soundboards.libraryTitle')}
        rows={rows}
        rowKey={(r) => r.s.id}
        columns={columns}
        selection={selection}
        onSelection={setSelection}
        onActivate={onAddToBoard ? (r) => add([r]) : undefined}
        onKey={(e, list) => {
          if (e.key === 'Delete') {
            void lib.removeSounds(list.map((r) => r.s), e);
            return true;
          }
          return false;
        }}
        onMenu={menuFor}
        empty={library.sounds.length === 0 ? t('soundboards.libraryEmpty') : t('soundboards.noMatch')}
        initialSort={{ by: 'name', down: false }}
      />
      {menu && <ContextMenu x={menu.x} y={menu.y} items={menu.items} onClose={() => setMenu(null)} />}
    </div>
  );
}
