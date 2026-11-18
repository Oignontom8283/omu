import { useCallback, useEffect, useRef, useState } from 'react';
import { IN_DESKTOP_WINDOW, RpcError, pickSoundFiles, uploadSound, type api } from '../api.ts';
import { MODULE_NEEDED } from '../modules.ts';
import { newId } from '../graph.ts';
import { useT } from '../i18n/index.tsx';
import { ShortcutField } from './ShortcutField.tsx';
import { partChange } from '../soundboard.ts';
import {
  boardInGraph,
  cleanCacheMb,
  cleanCompressMb,
  emptyPad,
  formatBytes,
  formatDuration,
  MIN_CACHE_MB,
  newBoardName,
  padFor,
  padsUsing,
  withPad,
  withPadRemoved,
  withPadsAdded,
} from '../soundboard.ts';
import type { Board, GraphSpec, Pad, Settings, SoundInfo, SoundLibrary, SoundStatus } from '../types.ts';

type Daemon = ReturnType<typeof api>;

type Props = {
  daemon: Daemon;
  token: string;
  library: SoundLibrary;
  setLibrary: (l: SoundLibrary) => void;
  /** The graph the daemon is running, and the one on screen (a block may be there and not applied yet). */
  applied: GraphSpec;
  shown: GraphSpec;
  /** The board to show first. */
  initialBoard?: string;
  /** Adds a soundboard block for this board to the editor. */
  onAddBlock: (board: string) => void;
  notify: (text: string) => void;
  /** Opens the panel of the optional modules. */
  onOpenModules: () => void;
};

const message = (e: unknown): string => (e instanceof Error ? e.message : String(e));

/** The panel of the soundboards: boards, their pads, the sounds, and the memory they may use. */
export function Soundboards({ daemon, token, library, setLibrary, applied, shown, initialBoard, onAddBlock, notify, onOpenModules }: Props) {
  const t = useT();
  const [boardId, setBoardId] = useState<string | undefined>(initialBoard ?? library.boards[0]?.id);
  const [playing, setPlaying] = useState<string[]>([]);
  const [status, setStatus] = useState<SoundStatus | null>(null);
  const [settings, setSettings] = useState<Settings | null>(null);
  const [cacheText, setCacheText] = useState('');
  const [compressText, setCompressText] = useState('');
  const [compressing, setCompressing] = useState<string | null>(null);
  const [importing, setImporting] = useState<string[]>([]);
  const [over, setOver] = useState(false);
  const [path, setPath] = useState('');
  const [confirm, setConfirm] = useState<string | null>(null);
  // a file that needs an optional module (its id), until the question is answered
  const [needModule, setNeedModule] = useState<string | null>(null);
  const fileInput = useRef<HTMLInputElement>(null);

  // the latest values, for the work that outlives a render (an import takes a while)
  const libraryRef = useRef(library);
  libraryRef.current = library;
  const boardRef = useRef(boardId);
  boardRef.current = boardId;

  const board = library.boards.find((b) => b.id === boardId) ?? library.boards[0];
  const current = board?.id;
  // a board that was deleted elsewhere, or the first one that appears
  useEffect(() => {
    if (boardId !== undefined && !library.boards.some((b) => b.id === boardId)) setBoardId(library.boards[0]?.id);
    if (boardId === undefined && library.boards.length > 0) setBoardId(library.boards[0].id);
  }, [library, boardId]);

  // what is playing, and how the memory is used
  const active = status?.active_boards.includes(current ?? '') ?? false;
  useEffect(() => {
    if (!current || !active) {
      setPlaying([]);
      return;
    }
    const timer = setInterval(() => {
      daemon.playing(current).then(setPlaying).catch(() => undefined);
    }, 250);
    return () => clearInterval(timer);
  }, [daemon, current, active]);
  useEffect(() => {
    const read = () => daemon.soundStatus().then(setStatus).catch(() => undefined);
    void read();
    const timer = setInterval(read, 1500);
    return () => clearInterval(timer);
  }, [daemon]);
  useEffect(() => {
    daemon
      .settings()
      .then((s) => {
        setSettings(s);
        setCacheText(String(s.sound_cache_mb));
        setCompressText(String(s.compress_above_mb));
      })
      .catch(() => undefined);
  }, [daemon]);
  // a second click within 4 s confirms a removal
  useEffect(() => {
    if (!confirm) return;
    const timer = window.setTimeout(() => setConfirm(null), 4000);
    return () => window.clearTimeout(timer);
  }, [confirm]);

  const reload = useCallback(async () => {
    try {
      const fresh = await daemon.soundboards();
      libraryRef.current = fresh;
      setLibrary(fresh);
    } catch (e) {
      notify(message(e));
    }
  }, [daemon, setLibrary, notify]);

  /** Saves the boards (shown at once; the daemon's answer corrects it if it refuses). */
  const save = useCallback(
    async (boards: Board[]) => {
      libraryRef.current = { ...libraryRef.current, boards };
      setLibrary(libraryRef.current);
      try {
        await daemon.setSoundboards(boards);
      } catch (e) {
        notify(message(e));
        void reload();
      }
    },
    [daemon, setLibrary, notify, reload],
  );

  const addBoard = async () => {
    const created: Board = { id: newId(), name: newBoardName(libraryRef.current.boards, t('soundboards.boardBase')), pads: [] };
    await save([...libraryRef.current.boards, created]);
    setBoardId(created.id);
  };
  const renameBoard = (name: string) => void save(libraryRef.current.boards.map((b) => (b.id === current ? { ...b, name } : b)));
  const removeBoard = async () => {
    if (!current) return;
    if (confirm !== `board:${current}`) {
      setConfirm(`board:${current}`);
      return;
    }
    setConfirm(null);
    if (active) await daemon.stopPad(current).catch(() => undefined);
    await save(libraryRef.current.boards.filter((b) => b.id !== current));
  };

  // ---- importing: sounds come in, and each one becomes a pad of the board on screen
  const finishImport = async (sounds: SoundInfo[]) => {
    if (sounds.length === 0) return;
    let boards = libraryRef.current.boards;
    let target = boardRef.current && boards.some((b) => b.id === boardRef.current) ? boardRef.current : undefined;
    if (!target) {
      const created: Board = { id: newId(), name: newBoardName(boards, t('soundboards.boardBase')), pads: [] };
      boards = [...boards, created];
      target = created.id;
      setBoardId(target);
    }
    boards = withPadsAdded(boards, target, sounds.map(padFor));
    // the daemon knows the new sounds already: take them, with the boards
    try {
      const fresh = await daemon.soundboards();
      libraryRef.current = { ...fresh, boards };
      setLibrary(libraryRef.current);
      await daemon.setSoundboards(boards);
    } catch (e) {
      notify(message(e));
      void reload();
    }
  };
  const importAll = async (jobs: { label: string; run: () => Promise<SoundInfo> }[]) => {
    const done: SoundInfo[] = [];
    setImporting((l) => [...l, ...jobs.map((j) => j.label)]);
    for (const job of jobs) {
      try {
        done.push(await job.run());
      } catch (e) {
        if (e instanceof RpcError && e.code === MODULE_NEEDED) setNeedModule(typeof e.data === 'string' ? e.data : 'ffmpeg');
        else notify(`${job.label}: ${e instanceof RpcError || e instanceof Error ? e.message : String(e)}`);
      } finally {
        setImporting((l) => {
          const i = l.indexOf(job.label);
          return i < 0 ? l : [...l.slice(0, i), ...l.slice(i + 1)];
        });
      }
    }
    await finishImport(done);
  };
  const addFiles = (files: File[]) => void importAll(files.map((f) => ({ label: f.name, run: () => uploadSound(token, f) })));
  const addPaths = (paths: string[]) => void importAll(paths.filter((p) => p.trim()).map((p) => ({ label: p, run: () => daemon.importSound(p.trim()) })));
  const addPathsRef = useRef(addPaths);
  addPathsRef.current = addPaths;

  // the desktop window receives the paths of files dropped on it; the browser, the files themselves
  useEffect(() => {
    if (!IN_DESKTOP_WINDOW) return;
    let off: (() => void) | undefined;
    let gone = false;
    void import('@tauri-apps/api/webview').then(async ({ getCurrentWebview }) => {
      const stopListening = await getCurrentWebview().onDragDropEvent((e) => {
        const p = e.payload;
        if (p.type === 'over' || p.type === 'enter') setOver(true);
        else if (p.type === 'drop') {
          setOver(false);
          addPathsRef.current(p.paths);
        } else setOver(false);
      });
      if (gone) stopListening();
      else off = stopListening;
    });
    return () => {
      gone = true;
      off?.();
    };
  }, []);

  // ---- pads
  const change = (pad: Pad, patch: Partial<Pad>) => current && void save(withPad(libraryRef.current.boards, current, pad.id, patch));
  const removePad = (pad: Pad) => {
    if (!current) return;
    void daemon.stopPad(current, pad.id).catch(() => undefined);
    void save(withPadRemoved(libraryRef.current.boards, current, pad.id));
  };
  const play = async (pad: Pad) => {
    if (!current) return;
    try {
      await daemon.playPad(current, pad.id);
      setPlaying((l) => (l.includes(pad.id) ? l : [...l, pad.id]));
    } catch (e) {
      notify(message(e));
    }
  };
  const stop = (pad?: Pad) => {
    if (!current) return;
    daemon
      .stopPad(current, pad?.id)
      .then(() => (pad ? undefined : setPlaying([])))
      .catch((e) => notify(message(e)));
  };

  const removeSound = async (s: SoundInfo) => {
    if (confirm !== `sound:${s.id}`) {
      setConfirm(`sound:${s.id}`);
      return;
    }
    setConfirm(null);
    try {
      await daemon.removeSound(s.id);
    } catch (e) {
      notify(message(e));
    }
    await reload();
  };

  const compressSound = async (s: SoundInfo) => {
    setCompressing(s.id);
    try {
      await daemon.compressSound(s.id);
    } catch (e) {
      notify(`${s.name}: ${message(e)}`);
    } finally {
      setCompressing(null);
    }
    await reload();
  };

  const saveCache = async () => {
    const mb = cleanCacheMb(cacheText);
    if (mb === null) {
      notify(t('soundboards.cacheInvalid', { min: MIN_CACHE_MB }));
      return;
    }
    const packMb = cleanCompressMb(compressText);
    if (packMb === null) {
      notify(t('soundboards.compressInvalid'));
      return;
    }
    const next: Settings = { show_tray_icon: true, start_at_logon: false, ...settings, sound_cache_mb: mb, compress_above_mb: packMb };
    try {
      await daemon.setSettings(next);
      setSettings(next);
      setCacheText(String(mb));
      setCompressText(String(packMb));
    } catch (e) {
      notify(message(e));
    }
  };

  const inApplied = current ? boardInGraph(applied, current) : false;
  const inShown = current ? boardInGraph(shown, current) : false;
  const soundName = (id: string | null | undefined) => library.sounds.find((s) => s.id === id)?.name;

  return (
    <aside
      className={`boards${over ? ' over' : ''}`}
      aria-label={t('soundboards.title')}
      onDragOver={(e) => {
        if (IN_DESKTOP_WINDOW) return;
        e.preventDefault();
        setOver(true);
      }}
      onDragLeave={() => setOver(false)}
      onDrop={(e) => {
        if (IN_DESKTOP_WINDOW) return;
        e.preventDefault();
        setOver(false);
        addFiles(Array.from(e.dataTransfer.files));
      }}
    >
      <header>
        <h2>{t('soundboards.title')}</h2>
      </header>

      <div className="row">
        <select aria-label={t('soundboards.board')} value={current ?? ''} onChange={(e) => setBoardId(e.target.value)} disabled={library.boards.length === 0}>
          {library.boards.length === 0 && <option value="">{t('soundboards.none')}</option>}
          {library.boards.map((b) => (
            <option key={b.id} value={b.id}>
              {b.name}
            </option>
          ))}
        </select>
        <button onClick={() => void addBoard()}>{t('soundboards.newBoard')}</button>
        {board && (
          <>
            <input className="grow" aria-label={t('soundboards.rename')} value={board.name} onChange={(e) => renameBoard(e.target.value)} />
            <button className={confirm === `board:${board.id}` ? 'danger' : ''} onClick={() => void removeBoard()}>
              {confirm === `board:${board.id}` ? t('soundboards.deleteConfirm') : t('soundboards.deleteBoard')}
            </button>
          </>
        )}
      </div>

      {board && !inApplied && (
        <div className="banner info">
          {inShown ? t('soundboards.notAppliedYet') : t('soundboards.notInGraph')}{' '}
          {!inShown && (
            <button className="inline" onClick={() => onAddBlock(board.id)}>
              {t('soundboards.addBlock')}
            </button>
          )}
        </div>
      )}

      {board && (
        <>
          <div className="row">
            {IN_DESKTOP_WINDOW ? (
              <>
                <button className="primary" onClick={() => void pickSoundFiles().then((paths) => addPaths(paths)).catch((e) => notify(message(e)))}>
                  {t('soundboards.addSounds')}
                </button>
                <input
                  className="grow"
                  value={path}
                  placeholder={t('soundboards.pathPlaceholder')}
                  onChange={(e) => setPath(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === 'Enter' && path.trim()) {
                      addPaths([path]);
                      setPath('');
                    }
                  }}
                />
                <button
                  disabled={!path.trim()}
                  onClick={() => {
                    addPaths([path]);
                    setPath('');
                  }}
                >
                  {t('soundboards.addSound')}
                </button>
              </>
            ) : (
              <>
                <button className="primary" onClick={() => fileInput.current?.click()}>
                  {t('soundboards.addSounds')}
                </button>
                <input
                  ref={fileInput}
                  type="file"
                  hidden
                  multiple
                  accept=".wav,.mp3,.flac,.ogg,.oga,.aac,.m4a,.mp4,.m4v,.mov,.mkv,.webm,audio/*,video/*"
                  onChange={(e) => {
                    addFiles(Array.from(e.target.files ?? []));
                    e.target.value = '';
                  }}
                />
              </>
            )}
            <button onClick={() => current && void save(withPadsAdded(libraryRef.current.boards, current, [emptyPad(t('soundboards.padBase'))]))}>{t('soundboards.addPad')}</button>
            <span className="spacer" />
            <button onClick={() => stop()} disabled={playing.length === 0}>
              {t('soundboards.stopAll')}
            </button>
          </div>
          <p className="muted drop-hint">{IN_DESKTOP_WINDOW ? t('soundboards.dropHintDesktop') : t('soundboards.dropHint')}</p>
        </>
      )}
      {needModule && (
        <div className="banner warn" role="alert">
          {t('modules.needed', { name: needModule === 'ffmpeg' ? 'FFmpeg' : needModule })}{' '}
          <button
            className="inline primary"
            onClick={() => {
              const id = needModule;
              setNeedModule(null);
              // the question was the consent: the module is installed, and its panel shows the download
              daemon.installModule(id).catch((e) => notify(message(e)));
              onOpenModules();
            }}
          >
            {t('modules.neededInstall')}
          </button>
          <button className="inline" onClick={() => setNeedModule(null)}>
            {t('modules.neededLater')}
          </button>
        </div>
      )}
      {importing.length > 0 && (
        <div className="banner info" role="status">
          {t('soundboards.importing', { count: importing.length })}: {importing[0]}
        </div>
      )}

      {board && board.pads.length === 0 && importing.length === 0 && <p className="muted">{t('soundboards.noPads')}</p>}
      <div className="pads">
        {board?.pads.map((pad) => {
          const isPlaying = playing.includes(pad.id);
          return (
            <div key={pad.id} className={`pad${isPlaying ? ' playing' : ''}${pad.sound ? '' : ' empty'}`} style={pad.color ? ({ '--pad': pad.color } as React.CSSProperties) : undefined}>
              <div className="pad-top">
                <button className="pad-play" disabled={!pad.sound || !active} title={!active ? t('soundboards.cannotPlay') : (soundName(pad.sound) ?? '')} onClick={() => void play(pad)}>
                  <span aria-hidden="true">▶</span> {pad.name || t('soundboards.unnamedPad')}
                </button>
                {isPlaying && (
                  <button className="mini" title={t('soundboards.stopPad')} aria-label={t('soundboards.stopPad')} onClick={() => stop(pad)}>
                    ■
                  </button>
                )}
                <button className="mini" title={t('soundboards.removePad')} aria-label={t('soundboards.removePad')} onClick={() => removePad(pad)}>
                  ×
                </button>
              </div>
              <input aria-label={t('soundboards.padName')} value={pad.name} onChange={(e) => change(pad, { name: e.target.value })} />
              <select aria-label={t('soundboards.padSound')} value={pad.sound ?? ''} onChange={(e) => change(pad, { sound: e.target.value || null })}>
                <option value="">{t('soundboards.noSound')}</option>
                {library.sounds.map((s) => (
                  <option key={s.id} value={s.id}>
                    {s.name} ({formatDuration(s.frames)})
                  </option>
                ))}
              </select>
              <label className="field">
                <span>
                  {t('field.gain')}: {(pad.gain_db ?? 0) > 0 ? '+' : ''}
                  {pad.gain_db ?? 0} dB
                </span>
                <input type="range" min={-30} max={12} step={0.5} value={pad.gain_db ?? 0} onChange={(e) => change(pad, { gain_db: Number(e.target.value) })} />
              </label>
              <div className="field">
                <span>{t('soundboards.padKey')}</span>
                <ShortcutField value={pad.key ?? ''} onChange={(keys) => change(pad, { key: keys || null })} />
                {(status?.hotkey_problems ?? [])
                  .filter((h) => h.board === current && h.pad === pad.id)
                  .map((h) => (
                    <small key={h.message} className="warn">
                      {h.message}
                    </small>
                  ))}
                {pad.key && (
                  <label className="check" title={t('soundboards.holdHint')}>
                    <input type="checkbox" checked={pad.hold ?? false} onChange={(e) => change(pad, { hold: e.target.checked })} />
                    {t('soundboards.hold')}
                  </label>
                )}
                {pad.key && !active && <small className="muted">{t('soundboards.keyIdle')}</small>}
              </div>
              <div className="checks">
                <label className="check" title={t('soundboards.loopHint')}>
                  <input type="checkbox" checked={pad.looped ?? false} onChange={(e) => change(pad, { looped: e.target.checked })} />
                  {t('soundboards.loop')}
                </label>
                <label className="check" title={t('soundboards.exclusiveHint')}>
                  <input type="checkbox" checked={pad.exclusive ?? false} onChange={(e) => change(pad, { exclusive: e.target.checked })} />
                  {t('soundboards.exclusive')}
                </label>
              </div>
              <label className="field" title={t('soundboards.fadeHint')}>
                <span>
                  {t('soundboards.fade')}: {pad.fade_ms ? `${pad.fade_ms} ms` : t('soundboards.fadeShort')}
                </span>
                <input type="range" min={0} max={5000} step={50} value={pad.fade_ms ?? 0} onChange={(e) => change(pad, { fade_ms: Number(e.target.value) })} />
              </label>
              <div className="range-row" title={t('soundboards.partHint')}>
                <label>
                  <span>{t('soundboards.start')}</span>
                  <input type="number" min={0} step={0.1} value={(pad.start_ms ?? 0) / 1000} onChange={(e) => change(pad, partChange(pad, 'start_ms', e.target.value))} />
                </label>
                <label>
                  <span>{t('soundboards.end')}</span>
                  <input type="number" min={0} step={0.1} value={(pad.end_ms ?? 0) / 1000} onChange={(e) => change(pad, partChange(pad, 'end_ms', e.target.value))} />
                </label>
              </div>
              <div className="color-row">
                <span>{t('soundboards.color')}</span>
                <input type="color" aria-label={t('soundboards.color')} value={pad.color ?? '#0f9d8a'} onChange={(e) => change(pad, { color: e.target.value })} />
                {pad.color && (
                  <button type="button" className="plain" aria-label={t('soundboards.colorReset')} title={t('soundboards.colorReset')} onClick={() => change(pad, { color: null })}>
                    ×
                  </button>
                )}
              </div>
            </div>
          );
        })}
      </div>

      <details className="sounds">
        <summary>{t('soundboards.library', { count: library.sounds.length })}</summary>
        {library.sounds.length === 0 && <p className="muted">{t('soundboards.libraryEmpty')}</p>}
        <ul>
          {library.sounds.map((s) => {
            const uses = padsUsing(library.boards, s.id);
            return (
              <li key={s.id}>
                <span className="grow" title={s.name}>
                  {s.name}
                </span>
                <small className="muted">
                  {formatDuration(s.frames)} · {formatBytes(s.bytes)}
                  {s.compressed ? ` · ${t('soundboards.compressed')}` : ''} · {t('soundboards.usedBy', { count: uses })}
                </small>
                {!s.compressed && (
                  <button className="mini" disabled={compressing !== null} title={t('soundboards.compressNow')} onClick={() => void compressSound(s)}>
                    {compressing === s.id ? t('soundboards.compressing') : t('soundboards.compress')}
                  </button>
                )}
                <button className={`mini${confirm === `sound:${s.id}` ? ' danger' : ''}`} title={t('soundboards.removeSound')} aria-label={t('soundboards.removeSound')} onClick={() => void removeSound(s)}>
                  {confirm === `sound:${s.id}` ? t('soundboards.deleteConfirm') : '×'}
                </button>
              </li>
            );
          })}
        </ul>
      </details>

      <section className="memory">
        <h3>{t('soundboards.memory')}</h3>
        <p className="muted">
          {status?.active
            ? t('soundboards.memoryUsed', { used: formatBytes(status.cache_used_bytes), limit: formatBytes(status.cache_limit_bytes), loaded: status.sounds_loaded })
            : t('soundboards.memoryIdle')}
          {status && status.streams > 0 ? ` · ${t('soundboards.streams', { count: status.streams })}` : ''}
        </p>
        {status && status.late_frames > 0 && <p className="warn">{t('soundboards.late')}</p>}
        <div className="row">
          <label className="field grow">
            <span>{t('soundboards.cacheLimit')}</span>
            <input type="number" min={MIN_CACHE_MB} step={16} value={cacheText} onChange={(e) => setCacheText(e.target.value)} />
          </label>
        </div>
        <div className="row">
          <label className="field grow">
            <span>{t('soundboards.compressAbove')}</span>
            <input type="number" min={0} step={1} value={compressText} onChange={(e) => setCompressText(e.target.value)} />
          </label>
          <button onClick={() => void saveCache()} disabled={settings !== null && String(settings.sound_cache_mb) === cacheText && String(settings.compress_above_mb) === compressText}>
            {t('soundboards.cacheSave')}
          </button>
        </div>
        <small className="muted">{t('soundboards.compressHint')}</small>
        <small className="muted">{t('soundboards.cacheHint')}</small>
      </section>
    </aside>
  );
}
