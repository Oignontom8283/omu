import { useCallback, useEffect, useRef, useState } from 'react';
import { IN_DESKTOP_WINDOW, RpcError, pickSoundFiles, uploadSound, type api } from '../api.ts';
import { MODULE_NEEDED } from '../modules.ts';
import { newId } from '../graph.ts';
import { useT } from '../i18n/index.tsx';
import { Button, TextInput } from '../kit/controls.tsx';
import {
  cleanCacheMb,
  cleanCompressMb,
  MIN_CACHE_MB,
  newBoardName,
  padFor,
  padsUsing,
  withPad,
  withPadRemoved,
  withPadsAdded,
} from '../soundboard.ts';
import type { Board, ImportJob, Pad, Settings, SoundInfo, SoundLibrary as Library, SoundStatus } from '../types.ts';
import { FloatingWindow } from './FloatingWindow.tsx';
import { Icon } from './Icon.tsx';
import { ImportCard, PadCard } from './PadCard.tsx';
import { PadSettings } from './PadSettings.tsx';
import { SoundLibrary } from './SoundLibrary.tsx';
import { useWindowStack } from './windows.ts';

type Daemon = ReturnType<typeof api>;

type Props = {
  daemon: Daemon;
  token: string;
  library: Library;
  setLibrary: (l: Library) => void;
  /** The board to show first. */
  initialBoard?: string;
  notify: (text: string) => void;
  /** Opens the panel of the optional modules. */
  onOpenModules: () => void;
};

/** A sound on its way in: it has a card of its own until it is a pad. */
type Importing = { job: string; board: string; label: string; error?: string };

const message = (e: unknown): string => (e instanceof Error ? e.message : String(e));

/** The page of the soundboards: a tab per soundboard, and under it a card for each of its sounds. */
export function Soundboards({ daemon, token, library, setLibrary, initialBoard, notify, onOpenModules }: Props) {
  const t = useT();
  const [boardId, setBoardId] = useState<string | undefined>(initialBoard ?? library.boards[0]?.id);
  const [playing, setPlaying] = useState<string[]>([]);
  const [status, setStatus] = useState<SoundStatus | null>(null);
  const [settings, setSettings] = useState<Settings | null>(null);
  const [cacheText, setCacheText] = useState('');
  const [compressText, setCompressText] = useState('');
  const [compressing, setCompressing] = useState<string | null>(null);
  const [importing, setImporting] = useState<Importing[]>([]);
  const [progress, setProgress] = useState<Record<string, ImportJob>>({});
  const [over, setOver] = useState(false);
  const [confirm, setConfirm] = useState<string | null>(null);
  const [editingTab, setEditingTab] = useState<string | null>(null);
  const [query, setQuery] = useState('');
  const windows = useWindowStack<string>();
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
  // how far the imports are, asked of the daemon while there are some
  const running = importing.some((i) => !i.error);
  useEffect(() => {
    if (!running) {
      setProgress({});
      return;
    }
    const read = () =>
      daemon
        .imports()
        .then((jobs) => setProgress(Object.fromEntries(jobs.map((j) => [j.job, j]))))
        .catch(() => undefined);
    void read();
    const timer = setInterval(read, 300);
    return () => clearInterval(timer);
  }, [daemon, running]);
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

  // ---- soundboards (the tabs)
  const addBoard = async () => {
    const created: Board = { id: newId(), name: newBoardName(libraryRef.current.boards, t('soundboards.boardBase')), pads: [] };
    await save([...libraryRef.current.boards, created]);
    setBoardId(created.id);
    setEditingTab(created.id);
  };
  const renameBoard = (id: string, name: string) => void save(libraryRef.current.boards.map((b) => (b.id === id ? { ...b, name: name.trim() || b.name } : b)));
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

  // ---- importing: a sound comes in as a card with a bar, then it is a pad of the soundboard it was dropped on
  const finishImport = async (sound: SoundInfo, target: string) => {
    let boards = libraryRef.current.boards;
    if (!boards.some((b) => b.id === target)) return;
    boards = withPadsAdded(boards, target, [padFor(sound)]);
    // the daemon knows the new sound already: take it, with the boards
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
  const importAll = async (jobs: { label: string; run: (job: string) => Promise<SoundInfo> }[]) => {
    if (jobs.length === 0) return;
    // dropped on no soundboard at all: the first one is made
    let boards = libraryRef.current.boards;
    let target = boardRef.current && boards.some((b) => b.id === boardRef.current) ? boardRef.current : undefined;
    if (!target) {
      const created: Board = { id: newId(), name: newBoardName(boards, t('soundboards.boardBase')), pads: [] };
      boards = [...boards, created];
      target = created.id;
      await save(boards);
      setBoardId(target);
    }
    const board = target;
    const queued = jobs.map((j) => ({ ...j, id: newId() }));
    setImporting((l) => [...l, ...queued.map((j): Importing => ({ job: j.id, board, label: j.label }))]);
    for (const job of queued) {
      try {
        await finishImport(await job.run(job.id), board);
        setImporting((l) => l.filter((i) => i.job !== job.id));
      } catch (e) {
        if (e instanceof RpcError && e.code === MODULE_NEEDED) {
          setNeedModule(typeof e.data === 'string' ? e.data : 'ffmpeg');
          setImporting((l) => l.filter((i) => i.job !== job.id));
        } else {
          const why = e instanceof RpcError || e instanceof Error ? e.message : String(e);
          setImporting((l) => l.map((i) => (i.job === job.id ? { ...i, error: why } : i)));
        }
      }
    }
  };
  const addFiles = (files: File[]) => void importAll(files.map((f) => ({ label: f.name, run: (job) => uploadSound(token, f, job) })));
  const addPaths = (paths: string[]) => void importAll(paths.filter((p) => p.trim()).map((p) => ({ label: p.trim().split(/[\\/]/).pop() ?? p, run: (job) => daemon.importSound(p.trim(), job) })));
  const addPathsRef = useRef(addPaths);
  addPathsRef.current = addPaths;
  const chooseFiles = () => {
    if (IN_DESKTOP_WINDOW) void pickSoundFiles().then(addPaths).catch((e) => notify(message(e)));
    else fileInput.current?.click();
  };

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
  const soundOf = (pad: Pad) => library.sounds.find((s) => s.id === pad.sound);
  // a pad is changed on its own soundboard, whichever one is on screen (its window may stay open when the tab changes)
  const change = (pad: Pad, patch: Partial<Pad>) => {
    const owner = libraryRef.current.boards.find((b) => b.pads.some((p) => p.id === pad.id));
    if (owner) void save(withPad(libraryRef.current.boards, owner.id, pad.id, patch));
  };
  const removePad = async (pad: Pad) => {
    if (!current) return;
    if (confirm !== `pad:${pad.id}`) {
      setConfirm(`pad:${pad.id}`);
      return;
    }
    setConfirm(null);
    void daemon.stopPad(current, pad.id).catch(() => undefined);
    const boards = withPadRemoved(libraryRef.current.boards, current, pad.id);
    await save(boards);
    // a sound that no pad plays any more is not kept
    if (pad.sound && padsUsing(boards, pad.sound) === 0) {
      await daemon.removeSound(pad.sound).catch((e) => notify(message(e)));
      await reload();
    }
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
    try {
      // never defaults over what the daemon holds: when the settings were not read earlier, they are read now
      const now = settings ?? (await daemon.settings());
      const next: Settings = { ...now, sound_cache_mb: mb, compress_above_mb: packMb };
      await daemon.setSettings(next);
      setSettings(next);
      setCacheText(String(mb));
      setCompressText(String(packMb));
    } catch (e) {
      notify(message(e));
    }
  };

  const problemsOf = (pad: Pad) => (status?.hotkey_problems ?? []).filter((h) => h.pad === pad.id).map((h) => h.message);
  const mine = importing.filter((i) => i.board === current);
  const shownPads = (board?.pads ?? []).filter((pad) => {
    const q = query.trim().toLowerCase();
    return !q || pad.name.toLowerCase().includes(q) || (soundOf(pad)?.name.toLowerCase().includes(q) ?? false);
  });
  // a window of a sound that is gone (its card was removed) goes with it
  useEffect(() => {
    windows.prune((key) => key === 'library' || library.boards.some((b) => b.pads.some((p) => `pad:${p.id}` === key)));
  }, [library, windows]);
  const place = (e: React.MouseEvent) => ({ x: e.clientX + 12, y: e.clientY + 12 });

  return (
    <aside
      className={`boards soundboards-page${over ? ' over' : ''}`}
      aria-label={t('soundboards.title')}
      onDragOver={(e) => {
        if (IN_DESKTOP_WINDOW) return;
        e.preventDefault();
        setOver(true);
      }}
      onDragLeave={(e) => {
        if (e.currentTarget === e.target) setOver(false);
      }}
      onDrop={(e) => {
        if (IN_DESKTOP_WINDOW) return;
        e.preventDefault();
        setOver(false);
        addFiles(Array.from(e.dataTransfer.files));
      }}
    >
      <nav className="board-tabs" aria-label={t('soundboards.board')}>
        <div className="board-tab-list" role="tablist">
          {library.boards.map((b) => {
            const on = b.id === current;
            const asking = confirm === `board:${b.id}`;
            return (
              <div key={b.id} className={`board-tab${on ? ' on' : ''}${asking ? ' asking' : ''}`} role="presentation">
                {editingTab === b.id ? (
                  <input
                    className="board-tab-input"
                    autoFocus
                    defaultValue={b.name}
                    aria-label={t('soundboards.rename')}
                    onFocus={(e) => e.currentTarget.select()}
                    onBlur={(e) => {
                      renameBoard(b.id, e.currentTarget.value);
                      setEditingTab(null);
                    }}
                    onKeyDown={(e) => {
                      if (e.key === 'Enter') e.currentTarget.blur();
                      if (e.key === 'Escape') setEditingTab(null);
                    }}
                  />
                ) : (
                  <button type="button" role="tab" aria-selected={on} title={t('soundboards.tabHint')} onClick={() => setBoardId(b.id)} onDoubleClick={() => setEditingTab(b.id)}>
                    {asking ? t('soundboards.deleteBoardAsk', { name: b.name }) : b.name}
                  </button>
                )}
                {on && editingTab !== b.id && (
                  <button type="button" className="tab-x" title={asking ? t('soundboards.deleteConfirm') : t('soundboards.deleteBoard')} aria-label={t('soundboards.deleteBoard')} onClick={() => void removeBoard()}>
                    <Icon name="trash" size={12} />
                  </button>
                )}
              </div>
            );
          })}
          <Button variant="flat" size="small" icon="plus" className="board-tab-add" title={t('soundboards.newBoard')} onClick={() => void addBoard()} />
        </div>
        <span className="spacer" />
        {playing.length > 0 && (
          <Button size="small" icon="stop" onClick={() => stop()}>
            {t('soundboards.stopAll')}
          </Button>
        )}
        <Button variant="flat" size="small" icon="list" title={t('soundboards.libraryTitle')} onClick={(e) => windows.open('library', { x: e.clientX - 760, y: e.clientY + 16 })}>
          {t('soundboards.library', { count: library.sounds.length })}
        </Button>
      </nav>

      <div className="boards-page">
        {needModule && (
          <div className="banner warn" role="alert">
            {t('modules.needed', { name: needModule === 'ffmpeg' ? 'FFmpeg' : needModule })}{' '}
            <Button
              size="small"
              variant="primary"
              onClick={() => {
                const id = needModule;
                setNeedModule(null);
                // the question was the consent: the module is installed, and its panel shows the download
                daemon.installModule(id).catch((e) => notify(message(e)));
                onOpenModules();
              }}
            >
              {t('modules.neededInstall')}
            </Button>{' '}
            <Button size="small" onClick={() => setNeedModule(null)}>
              {t('modules.neededLater')}
            </Button>
          </div>
        )}

        {!board && (
          <div className="boards-empty">
            <p>{t('soundboards.none')}</p>
            <Button variant="primary" icon="plus" onClick={() => void addBoard()}>
              {t('soundboards.newBoard')}
            </Button>
          </div>
        )}

        {board && (
          <>
            <div className="boards-toolbar">
              <Button variant="primary" icon="plus" onClick={chooseFiles}>
                {t('soundboards.addSounds')}
              </Button>
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
              {board.pads.length > 0 && <TextInput code={false} className="boards-filter" placeholder={t('soundboards.filter')} value={query} onChange={setQuery} />}
              <span className="muted boards-hint">{IN_DESKTOP_WINDOW ? t('soundboards.dropHintDesktop') : t('soundboards.dropHint')}</span>
            </div>

            <div className="pad-cards">
              {shownPads.map((pad) => (
                <PadCard
                  key={pad.id}
                  pad={pad}
                  sound={soundOf(pad)}
                  playing={playing.includes(pad.id)}
                  canPlay={active}
                  problems={problemsOf(pad)}
                  confirming={confirm === `pad:${pad.id}`}
                  onChange={(patch) => change(pad, patch)}
                  onPlay={() => void play(pad)}
                  onStop={() => stop(pad)}
                  onSettings={(e) => windows.open(`pad:${pad.id}`, place(e))}
                  onRemove={() => void removePad(pad)}
                />
              ))}
              {mine.map((i) => (
                <ImportCard key={i.job} job={progress[i.job]} label={i.label} waiting={!progress[i.job]} error={i.error} onDismiss={() => setImporting((l) => l.filter((x) => x.job !== i.job))} />
              ))}
              {query.trim() === '' && (
                <button type="button" className="pad-add nodrag" onClick={chooseFiles} aria-label={t('soundboards.addSounds')}>
                  <Icon name="plus" size={20} />
                </button>
              )}
            </div>
            {query.trim() !== '' && shownPads.length === 0 && <p className="muted">{t('soundboards.noMatch')}</p>}
          </>
        )}
      </div>

      {windows.list.map((w, order) => {
        if (w.key === 'library') {
          return (
            <FloatingWindow key={w.key} className="lib-window cat-input" title={t('soundboards.libraryTitle')} x={w.x} y={w.y} order={order} onMove={(x, y) => windows.move(w.key, x, y)} onFocus={() => windows.front(w.key)} onClose={() => windows.close(w.key)}>
              <SoundLibrary
                library={library}
                status={status}
                settings={settings}
                cacheText={cacheText}
                compressText={compressText}
                confirm={confirm}
                compressing={compressing}
                onCacheText={setCacheText}
                onCompressText={setCompressText}
                onSaveSettings={() => void saveCache()}
                onRemove={(id) => {
                  const s = library.sounds.find((x) => x.id === id);
                  if (s) void removeSound(s);
                }}
                onCompress={(id) => {
                  const s = library.sounds.find((x) => x.id === id);
                  if (s) void compressSound(s);
                }}
              />
            </FloatingWindow>
          );
        }
        const pad = library.boards.flatMap((b) => b.pads).find((p) => `pad:${p.id}` === w.key);
        if (!pad) return null;
        return (
          <FloatingWindow key={w.key} className="pad-window cat-input" title={pad.name || t('soundboards.unnamedPad')} x={w.x} y={w.y} order={order} onMove={(x, y) => windows.move(w.key, x, y)} onFocus={() => windows.front(w.key)} onClose={() => windows.close(w.key)}>
            <PadSettings
              pad={pad}
              sound={soundOf(pad)}
              problems={problemsOf(pad)}
              compressing={compressing === pad.sound}
              onChange={(patch) => change(pad, patch)}
              onCompress={() => {
                const s = soundOf(pad);
                if (s) void compressSound(s);
              }}
            />
          </FloatingWindow>
        );
      })}
    </aside>
  );
}
