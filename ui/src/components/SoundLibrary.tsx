import { useMemo, useState } from 'react';
import { ON_THE_DAEMON_COMPUTER } from '../api.ts';
import { useT } from '../i18n/index.tsx';
import { Button, NumberField, TextInput } from '../kit/controls.tsx';
import { VirtualRows } from '../kit/VirtualRows.tsx';
import { formatBytes, formatDuration, MIN_CACHE_MB, padsUsing } from '../soundboard.ts';
import type { Settings, SoundLibrary as Library, SoundStatus } from '../types.ts';

type Column = 'name' | 'duration' | 'size' | 'format' | 'uses';
const ROW_HEIGHT = 30;

/** The sounds the program holds, as a table that is sorted and filtered, and the memory soundboards may use. */
export function SoundLibrary({
  library,
  status,
  settings,
  cacheText,
  compressText,
  compressing,
  memory = true,
  onCacheText,
  onCompressText,
  onSaveSettings,
  onRemove,
  onCompress,
  onReveal,
}: {
  library: Library;
  status: SoundStatus | null;
  settings: Settings | null;
  cacheText: string;
  compressText: string;
  compressing: string | null;
  /** Whether the memory limits are edited under the table (the settings page has them as settings of their own). */
  memory?: boolean;
  onCacheText: (text: string) => void;
  onCompressText: (text: string) => void;
  onSaveSettings: () => void;
  onRemove: (id: string, e: React.MouseEvent) => void;
  onCompress: (id: string) => void;
  onReveal: (id: string) => void;
}) {
  const t = useT();
  const [query, setQuery] = useState('');
  const [sort, setSort] = useState<{ by: Column; down: boolean }>({ by: 'name', down: false });
  const rows = useMemo(() => {
    const q = query.trim().toLowerCase();
    const uses = new Map(library.sounds.map((s) => [s.id, padsUsing(library.boards, s.id)]));
    const key = (s: (typeof library.sounds)[number]): string | number => {
      switch (sort.by) {
        case 'duration':
          return s.frames;
        case 'size':
          return s.bytes;
        case 'format':
          return s.compressed ? 1 : 0;
        case 'uses':
          return uses.get(s.id) ?? 0;
        default:
          return s.name.toLowerCase();
      }
    };
    const kept = library.sounds.filter((s) => !q || s.name.toLowerCase().includes(q));
    kept.sort((a, b) => {
      const [x, y] = [key(a), key(b)];
      const order = typeof x === 'string' ? x.localeCompare(String(y), undefined, { numeric: true }) : x - Number(y);
      return sort.down ? -order : order;
    });
    return kept.map((s) => ({ sound: s, uses: uses.get(s.id) ?? 0 }));
  }, [library, query, sort]);
  const total = library.sounds.reduce((n, s) => n + s.bytes, 0);
  const header = (by: Column, label: string, right = false) => (
    <button type="button" role="columnheader" className={`lib-head${right ? ' right' : ''}`} onClick={() => setSort((s) => ({ by, down: s.by === by ? !s.down : false }))}>
      {label}
      {sort.by === by ? <span aria-hidden>{sort.down ? ' ▼' : ' ▲'}</span> : null}
    </button>
  );
  return (
    <div className="library">
      <div className="lib-bar">
        <TextInput code={false} className="grow" placeholder={t('soundboards.filter')} value={query} onChange={setQuery} />
        <small className="muted">{t('soundboards.libraryTotal', { count: library.sounds.length, size: formatBytes(total) })}</small>
      </div>
      <div className="lib-table" role="table" aria-rowcount={rows.length + 1}>
        <div className="lib-row lib-header" role="row">
          {header('name', t('soundboards.col.name'))}
          {header('duration', t('soundboards.col.duration'), true)}
          {header('size', t('soundboards.col.size'), true)}
          {header('format', t('soundboards.col.format'))}
          {header('uses', t('soundboards.col.uses'), true)}
          <span role="columnheader" aria-label={t('soundboards.col.actions')} />
        </div>
        {rows.length === 0 ? (
          <p className="muted lib-empty">{library.sounds.length === 0 ? t('soundboards.libraryEmpty') : t('soundboards.noMatch')}</p>
        ) : (
          <VirtualRows
            className="lib-body"
            count={rows.length}
            rowHeight={ROW_HEIGHT}
            label={t('soundboards.libraryTitle')}
            render={(i) => {
              const { sound: s, uses } = rows[i];
              return (
                <div className="lib-row" role="presentation">
                  <span className="lib-name" role="cell" title={s.name}>
                    {s.name}
                  </span>
                  <span className="right" role="cell">
                    {formatDuration(s.frames)}
                  </span>
                  <span className="right" role="cell">
                    {formatBytes(s.bytes)}
                  </span>
                  <span role="cell">{s.compressed ? t('soundboards.compressed') : t('soundboards.raw')}</span>
                  <span className="right" role="cell">
                    {uses}
                  </span>
                  <span className="lib-actions" role="cell">
                    {ON_THE_DAEMON_COMPUTER && <Button size="small" variant="flat" icon="folder" title={t('soundboards.showFileShort')} onClick={() => onReveal(s.id)} />}
                    {!s.compressed && <Button size="small" variant="flat" icon="stack" disabled={compressing !== null} title={t('soundboards.compressNow')} onClick={() => onCompress(s.id)} />}
                    <Button size="small" variant="flat" icon="trash" title={t('soundboards.removeSound')} onClick={(e) => onRemove(s.id, e)} />
                  </span>
                </div>
              );
            }}
          />
        )}
      </div>

      <section className="lib-memory">
        {memory && <h4>{t('soundboards.memory')}</h4>}
        <p className="muted">
          {status?.active
            ? t('soundboards.memoryUsed', { used: formatBytes(status.cache_used_bytes), limit: formatBytes(status.cache_limit_bytes), loaded: status.sounds_loaded })
            : t('soundboards.memoryIdle')}
          {status && status.streams > 0 ? ` · ${t('soundboards.streams', { count: status.streams })}` : ''}
        </p>
        {status && status.late_frames > 0 && <p className="warn">{t('soundboards.late')}</p>}
        {memory && (
          <>
            <div className="srow">
              <span>{t('soundboards.cacheLimit')}</span>
              <NumberField whole min={MIN_CACHE_MB} step={16} value={Number(cacheText) || 0} onChange={(n) => onCacheText(String(n))} />
            </div>
            <div className="srow">
              <span>{t('soundboards.compressAbove')}</span>
              <NumberField whole min={0} step={1} value={Number(compressText) || 0} onChange={(n) => onCompressText(String(n))} />
            </div>
            <div className="lib-save">
              <small className="muted">{t('soundboards.compressHint')}</small>
              <Button icon="save" onClick={onSaveSettings} disabled={settings !== null && String(settings.sound_cache_mb) === cacheText && String(settings.compress_above_mb) === compressText}>
                {t('soundboards.cacheSave')}
              </Button>
            </div>
          </>
        )}
      </section>
    </div>
  );
}
