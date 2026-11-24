import { useCallback, useEffect, useRef, useState } from 'react';
import { IN_DESKTOP_WINDOW, ON_THE_DAEMON_COMPUTER, type api } from '../api.ts';
import type { ImportJobSpec, SoundImports } from '../imports.tsx';
import { newId } from '../graph.ts';
import { useT } from '../i18n/index.tsx';
import { Button, TextInput } from '../kit/controls.tsx';
import { ask } from '../kit/dialogs.tsx';
import {
  newBoardName,
  orphanSounds,
  padFor,

  withPad,
  withPadRemoved,
  withPadsAdded,
} from '../soundboard.ts';
import { useSoundLibrary } from '../soundLibrary.ts';
import type { Board, Pad, SoundInfo, SoundLibrary as Library } from '../types.ts';
import { ContextMenu, type MenuItem } from './ContextMenu.tsx';
import { FloatingWindow } from './FloatingWindow.tsx';
import { Icon } from './Icon.tsx';
import { ImportCard, PadCard } from './PadCard.tsx';
import { PadSettings } from './PadSettings.tsx';
import { SoundsManager } from './SoundsManager.tsx';
import { useWindowStack } from './windows.ts';

type Daemon = ReturnType<typeof api>;

type Props = {
  daemon: Daemon;
  library: Library;
  setLibrary: (l: Library) => void;
  /** The board to show first. */
  initialBoard?: string;
  notify: (text: string) => void;
  /** The imports of the program: a sound that is dropped here is imported by them. */
  imports: SoundImports;
  /** The sounds the blocks of the graph play: removing a soundboard or a pad never takes them away. */
  graphSounds?: Set<string>;
};

const message = (e: unknown): string => (e instanceof Error ? e.message : String(e));

/** The page of the soundboards: a tab per soundboard, and under it a card for each of its sounds. */
export function Soundboards({ daemon, library, setLibrary, initialBoard, notify, imports, graphSounds }: Props) {
  const t = useT();
  const [boardId, setBoardId] = useState<string | undefined>(initialBoard ?? library.boards[0]?.id);
  const [playing, setPlaying] = useState<string[]>([]);
  const [over, setOver] = useState(false);
  const [menu, setMenu] = useState<{ x: number; y: number; items: MenuItem[] } | null>(null);
  const [editingTab, setEditingTab] = useState<string | null>(null);
  const [query, setQuery] = useState('');
  const windows = useWindowStack<string>();
  const { status, compressing, compressSound } = useSoundLibrary({ daemon, setLibrary, notify });

  // the latest values, for the work that outlives a render (an import takes a while)
  const libraryRef = useRef(library);
  libraryRef.current = library;
  const boardRef = useRef(boardId);
  boardRef.current = boardId;
  const graphSoundsRef = useRef<Set<string>>(graphSounds ?? new Set());
  graphSoundsRef.current = graphSounds ?? new Set();

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
  // a sound nothing else uses goes with the soundboard or the pad that held it (a block's file is never taken away)
  const dropOrphans = async (pads: Pad[], boards: Board[]) => {
    const gone = orphanSounds(pads, boards, graphSoundsRef.current);
    if (gone.length === 0) return;
    for (const id of gone) await daemon.removeSound(id).catch((e) => notify(message(e)));
    await reload();
  };
  const removeBoard = async (target: Board, e?: { shiftKey?: boolean }) => {
    if (!(await ask({ title: t('dialog.removeBoard.title', { name: target.name }), message: t('dialog.removeBoard.message') }, e))) return;
    if (status?.active_boards.includes(target.id)) await daemon.stopPad(target.id).catch(() => undefined);
    const boards = libraryRef.current.boards.filter((b) => b.id !== target.id);
    await save(boards);
    await dropOrphans(target.pads, boards);
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
  // a sound that comes in on a soundboard: it is added to it as a pad once it is stored
  // the soundboard the sounds go to: the one on screen, or a new one when there is none
  const ensureBoard = async (): Promise<string> => {
    const open = boardRef.current;
    if (open && libraryRef.current.boards.some((b) => b.id === open)) return open;
    const created: Board = { id: newId(), name: newBoardName(libraryRef.current.boards, t('soundboards.boardBase')), pads: [] };
    await save([...libraryRef.current.boards, created]);
    setBoardId(created.id);
    return created.id;
  };
  const importAll = async (jobs: ImportJobSpec[]) => {
    if (jobs.length === 0) return;
    const board = await ensureBoard();
    await imports.start(board, jobs, (sound) => finishImport(sound, board));
  };
  const addFiles = (files: File[]) => void importAll(imports.fromFiles(files));
  const addPaths = (paths: string[]) => void importAll(imports.fromPaths(paths));
  const addPathsRef = useRef(addPaths);
  addPathsRef.current = addPaths;
  const chooseFiles = async () => {
    const board = await ensureBoard();
    imports.choose(board, (sound) => finishImport(sound, board));
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
  const removePad = async (pad: Pad, e?: { shiftKey?: boolean }) => {
    if (!current) return;
    if (!(await ask({ title: t('dialog.removePad.title', { name: pad.name || t('soundboards.unnamedPad') }), message: t('dialog.removePad.message') }, e))) return;
    void daemon.stopPad(current, pad.id).catch(() => undefined);
    const boards = withPadRemoved(libraryRef.current.boards, current, pad.id);
    await save(boards);
    await dropOrphans([pad], boards);
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

  const problemsOf = (pad: Pad) => (status?.hotkey_problems ?? []).filter((h) => h.pad === pad.id).map((h) => h.message);
  const mine = imports.entries.filter((i) => i.owner === current);
  const shownPads = (board?.pads ?? []).filter((pad) => {
    const q = query.trim().toLowerCase();
    return !q || pad.name.toLowerCase().includes(q) || (soundOf(pad)?.name.toLowerCase().includes(q) ?? false);
  });
  // a window of a sound that is gone (its card was removed) goes with it
  useEffect(() => {
    windows.prune((key) => key === 'library' || library.boards.some((b) => b.pads.some((p) => `pad:${p.id}` === key)));
  }, [library, windows]);
  // the windows open at the middle of the screen, a little off from each other
  const centred = (width: number, n: number) => ({ x: Math.max(4, (window.innerWidth - width) / 2 + n * 24), y: Math.max(4, window.innerHeight / 2 - 260 + n * 24) });
  const openSettings = (pad: Pad) => windows.open(`pad:${pad.id}`, centred(400, windows.list.length));
  const showSoundFile = (id: string | null | undefined) => {
    if (id) daemon.revealSound(id).catch((e) => notify(message(e)));
  };
  const menuAt = (e: React.MouseEvent, items: MenuItem[]) => {
    e.preventDefault();
    e.stopPropagation();
    setMenu({ x: e.clientX, y: e.clientY, items });
  };
  const padMenu = (e: React.MouseEvent, pad: Pad) => {
    const isPlaying = playing.includes(pad.id);
    const canShow = ON_THE_DAEMON_COMPUTER && !!soundOf(pad);
    menuAt(e, [
      isPlaying ? { label: t('soundboards.stopPad'), run: () => stop(pad) } : { label: t('soundboards.play'), run: () => void play(pad) },
      { label: t('soundboards.settingsMenu'), run: () => openSettings(pad) },
      ...(canShow ? [{ label: t('soundboards.showFileShort'), run: () => showSoundFile(pad.sound) }] : []),
      ...(pad.key ? [{ label: t('soundboards.clearKey'), run: () => change(pad, { key: null }) }] : []),
      { label: t('soundboards.deleteMenu'), danger: true, run: (ev: React.MouseEvent) => void removePad(pad, ev) },
    ]);
  };
  const boardMenu = (e: React.MouseEvent, b: Board) =>
    menuAt(e, [
      { label: t('soundboards.renameBoard'), run: () => setEditingTab(b.id) },
      { label: t('soundboards.deleteMenu'), danger: true, run: (ev: React.MouseEvent) => void removeBoard(b, ev) },
    ]);

  return (
    <aside
      className={`boards soundboards-page${over ? ' over' : ''}`}
      aria-label={t('soundboards.title')}
      onContextMenu={(e) => menuAt(e, [{ label: t('soundboards.addSounds'), run: chooseFiles }, { label: t('soundboards.newBoard'), run: () => void addBoard() }])}
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
            return (
              <div key={b.id} className={`board-tab${on ? ' on' : ''}`} role="presentation" onContextMenu={(e) => boardMenu(e, b)}>
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
                    {b.name}
                  </button>
                )}
                {on && editingTab !== b.id && (
                  <button type="button" className="tab-x" title={t('soundboards.deleteBoard')} aria-label={t('soundboards.deleteBoard')} onClick={(e) => void removeBoard(b, e)}>
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
        <Button variant="flat" size="small" icon="list" title={t('soundboards.libraryTitle')} onClick={() => windows.open('library', centred(780, windows.list.length))}>
          {t('soundboards.library', { count: library.sounds.length })}
        </Button>
      </nav>

      <div className="boards-page">
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
                  onChange={(patch) => change(pad, patch)}
                  onPlay={() => void play(pad)}
                  onStop={() => stop(pad)}
                  onSettings={() => openSettings(pad)}
                  onRemove={(e) => void removePad(pad, e)}
                  onContextMenu={(e) => padMenu(e, pad)}
                />
              ))}
              {mine.map((i) => (
                <ImportCard key={i.job} job={imports.progress[i.job]} label={i.label} error={i.error} onDismiss={() => imports.dismiss(i.job)} />
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

      {menu && <ContextMenu x={menu.x} y={menu.y} items={menu.items} onClose={() => setMenu(null)} />}

      {windows.list.map((w, order) => {
        if (w.key === 'library') {
          return (
            <FloatingWindow key={w.key} className="lib-window cat-input" title={t('soundboards.libraryTitle')} x={w.x} y={w.y} order={order} onMove={(x, y) => windows.move(w.key, x, y)} onFocus={() => windows.front(w.key)} onClose={() => windows.close(w.key)}>
              <SoundsManager daemon={daemon} library={library} setLibrary={setLibrary} imports={imports} notify={notify} />
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
              onReveal={() => showSoundFile(pad.sound)}
            />
          </FloatingWindow>
        );
      })}
    </aside>
  );
}
