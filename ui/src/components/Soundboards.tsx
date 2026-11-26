import { useCallback, useEffect, useMemo, useRef, useState, type CSSProperties, type KeyboardEvent } from 'react';
import { IN_DESKTOP_WINDOW, ON_THE_DAEMON_COMPUTER, type api } from '../api.ts';
import type { ImportJobSpec, SoundImports } from '../imports.tsx';
import { newId } from '../graph.ts';
import { useT } from '../i18n/index.tsx';
import { Button, Slider, Switch } from '../kit/controls.tsx';
import { DataTable, isInteractive, type Column } from '../kit/DataTable.tsx';
import { ask } from '../kit/dialogs.tsx';
import { FilterBox, Highlight, Pane, SidePane, ToolSeparator, Toolbar, foldText } from '../kit/Panes.tsx';
import { cleanChoice, cleanFlag, cleanSize, usePersistent } from '../kit/persist.ts';
import { clickRow, moveCursor, movedIndex, NOTHING, selectAll, selectKeys, type ListSelection } from '../kit/selectionModel.ts';
import { TreeView, type TreeNode } from '../kit/TreeView.tsx';
import { formatBytes, formatDb, formatDuration, newBoardName, orphanSounds, padFor, usualPadSettings, withPad, withPadMoved, withPadRemoved, withPadsAdded } from '../soundboard.ts';
import { useSoundLibrary } from '../soundLibrary.ts';
import type { Board, Pad, SoundInfo, SoundLibrary as Library } from '../types.ts';
import { ContextMenu, SEPARATOR, type MenuItem } from './ContextMenu.tsx';
import { Icon } from './Icon.tsx';
import { ImportCard, PadCard } from './PadCard.tsx';
import { PadSettings } from './PadSettings.tsx';
import { SoundsManager } from './SoundsManager.tsx';

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

type View = 'grid' | 'table';
type Menu = { x: number; y: number; items: MenuItem[] };

const message = (e: unknown): string => (e instanceof Error ? e.message : String(e));
const BOARD = 'board:';
const PAD = 'pad:';

/**
 * The soundboards, laid out like an IDE. On the left the explorer: every soundboard as a folder of its sounds (renamed in place with F2,
 * reordered with Alt+arrows, removed with Delete). In the middle the soundboard on screen, as a grid of pads to play or as a table of
 * all their settings; pads are chosen with the mouse or the keyboard (Ctrl, Shift, Ctrl+A), Space plays them, Delete removes them. On the
 * right the inspector of what is chosen (one pad, several at once, or the soundboard). Under it the library of every imported sound.
 */
export function Soundboards({ daemon, library, setLibrary, initialBoard, notify, imports, graphSounds }: Props) {
  const t = useT();
  // the soundboard on screen: the one asked for, else the one of last time
  const [lastBoard, setLastBoard] = usePersistent<string | null>('omu-boards-current', null, (v) => (typeof v === 'string' ? v : undefined));
  const [boardId, setBoardIdNow] = useState<string | undefined>(initialBoard ?? (library.boards.some((b) => b.id === lastBoard) ? lastBoard! : library.boards[0]?.id));
  const setBoardId = (id: string | undefined) => {
    setBoardIdNow(id);
    if (id) setLastBoard(id);
  };
  const [selection, setSelection] = useState<ListSelection>(NOTHING);
  const [playing, setPlaying] = useState<string[]>([]);
  const [progress, setProgress] = useState<Record<string, number>>({});
  const [over, setOver] = useState(false);
  const [menu, setMenu] = useState<Menu | null>(null);
  const [renaming, setRenaming] = useState<string | null>(null);
  const [query, setQuery] = useState('');
  const [focusFilter, setFocusFilter] = useState(0);
  const [view, setView] = usePersistent<View>('omu-boards-view', 'grid', cleanChoice<View>('grid', 'table'));
  const [cardWidth, setCardWidth] = usePersistent('omu-boards-card', 240, cleanSize(160, 420));
  const [explorer, setExplorer] = usePersistent('omu-boards-explorer', true, cleanFlag);
  const [inspector, setInspector] = usePersistent('omu-boards-inspector', true, cleanFlag);
  const [libraryPane, setLibraryPane] = usePersistent('omu-boards-library', false, cleanFlag);
  const [expanded, setExpanded] = usePersistent<string[]>('omu-boards-expanded', [], (v) => (Array.isArray(v) ? v.filter((x): x is string => typeof x === 'string') : undefined));
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
  const pickBoard = (id: string) => {
    if (id !== current) setSelection(NOTHING);
    setBoardId(id);
  };

  // what is playing, and how far
  const active = status?.active_boards.includes(current ?? '') ?? false;
  useEffect(() => {
    if (!current || !active) {
      setPlaying([]);
      setProgress({});
      return;
    }
    const timer = setInterval(() => {
      daemon
        .progress(current)
        .then((list) => {
          setPlaying((before) => {
            const ids = list.map((p) => p.pad);
            return before.length === ids.length && before.every((id, i) => id === ids[i]) ? before : ids;
          });
          setProgress(Object.fromEntries(list.map((p) => [p.pad, p.fraction])));
        })
        .catch(() => undefined);
    }, 120);
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

  // ---- soundboards
  const addBoard = async () => {
    const created: Board = { id: newId(), name: newBoardName(libraryRef.current.boards, t('soundboards.boardBase')), pads: [] };
    await save([...libraryRef.current.boards, created]);
    pickBoard(created.id);
    setExplorer(true);
    setRenaming(`${BOARD}${created.id}`);
  };
  const renameBoard = (id: string, name: string) => void save(libraryRef.current.boards.map((b) => (b.id === id ? { ...b, name: name.trim() || b.name } : b)));
  const moveBoard = (id: string, step: number) => {
    const boards = libraryRef.current.boards.slice();
    const from = boards.findIndex((b) => b.id === id);
    if (from < 0) return;
    const [moved] = boards.splice(from, 1);
    boards.splice(movedIndex(boards.length + 1, from, step), 0, moved);
    void save(boards);
  };
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
    const target = await ensureBoard();
    await imports.start(target, jobs, (sound) => finishImport(sound, target));
  };
  const addFiles = (files: File[]) => void importAll(imports.fromFiles(files));
  const addPaths = (paths: string[]) => void importAll(imports.fromPaths(paths));
  const addPathsRef = useRef(addPaths);
  addPathsRef.current = addPaths;
  const chooseFiles = async () => {
    const target = await ensureBoard();
    imports.choose(target, (sound) => finishImport(sound, target));
  };
  // sounds of the library put on the soundboard on screen
  const addFromLibrary = async (sounds: SoundInfo[]) => {
    const target = await ensureBoard();
    const pads = sounds.map(padFor);
    await save(withPadsAdded(libraryRef.current.boards, target, pads));
    setSelection(selectKeys(pads.map((p) => p.id)));
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
  const ownerOf = (padId: string) => libraryRef.current.boards.find((b) => b.pads.some((p) => p.id === padId));
  // a pad is changed on its own soundboard, whichever one is on screen
  const change = (pad: Pad, patch: Partial<Pad>) => {
    const owner = ownerOf(pad.id);
    if (owner) void save(withPad(libraryRef.current.boards, owner.id, pad.id, patch));
  };
  const changeMany = (pads: Pad[], patch: Partial<Pad>) => {
    let boards = libraryRef.current.boards;
    for (const pad of pads) {
      const owner = boards.find((b) => b.pads.some((p) => p.id === pad.id));
      if (owner) boards = withPad(boards, owner.id, pad.id, patch);
    }
    void save(boards);
  };
  const removePads = async (pads: Pad[], e?: { shiftKey?: boolean }) => {
    if (!current || pads.length === 0) return;
    const title = pads.length === 1 ? t('dialog.removePad.title', { name: pads[0].name || t('soundboards.unnamedPad') }) : t('dialog.removePads.title', { count: pads.length });
    if (!(await ask({ title, message: t('dialog.removePad.message') }, e))) return;
    let boards = libraryRef.current.boards;
    for (const pad of pads) {
      void daemon.stopPad(current, pad.id).catch(() => undefined);
      boards = withPadRemoved(boards, current, pad.id);
    }
    await save(boards);
    await dropOrphans(pads, boards);
  };
  const movePad = (pad: Pad, step: number) => {
    if (!current || !board) return;
    const from = board.pads.findIndex((p) => p.id === pad.id);
    void save(withPadMoved(libraryRef.current.boards, current, from, movedIndex(board.pads.length, from, step)));
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
      .then(() => (pad ? undefined : (setPlaying([]), setProgress({}))))
      .catch((e) => notify(message(e)));
  };
  const toggle = (pads: Pad[]) => {
    const anyPlaying = pads.some((p) => playing.includes(p.id));
    pads.forEach((p) => (anyPlaying ? playing.includes(p.id) && stop(p) : void play(p)));
  };

  const problemsOf = (pad: Pad) => (status?.hotkey_problems ?? []).filter((h) => h.pad === pad.id).map((h) => h.message);
  const mine = imports.entries.filter((i) => i.owner === current);
  const q = foldText(query.trim());
  const shownPads = useMemo(() => (board?.pads ?? []).filter((pad) => !q || foldText(pad.name).includes(q) || foldText(library.sounds.find((s) => s.id === pad.sound)?.name ?? '').includes(q) || foldText(pad.key ?? '').includes(q)), [board, q, library.sounds]);
  const chosen = (board?.pads ?? []).filter((p) => selection.selected.includes(p.id));
  const showFile = (pad: Pad) => pad.sound && daemon.revealSound(pad.sound).catch((e) => notify(message(e)));
  const rename = (pad: Pad) => {
    setInspector(true);
    setSelection(selectKeys([pad.id]));
    window.setTimeout(() => document.querySelector<HTMLInputElement>('.pad-inspector input')?.focus(), 30);
  };

  // ---- menus
  const padMenu = (pads: Pad[]): MenuItem[] => {
    const one = pads.length === 1 ? pads[0] : undefined;
    const anyPlaying = pads.some((p) => playing.includes(p.id));
    return [
      { label: anyPlaying ? t('soundboards.stopPad') : t('soundboards.play'), keys: t('keys.space'), disabled: !active, run: () => toggle(pads) },
      SEPARATOR,
      { label: t('soundboards.renameMenu'), keys: 'F2', disabled: !one, run: () => one && rename(one) },
      { label: t('soundboards.settingsMenu'), run: () => setInspector(true) },
      ...(ON_THE_DAEMON_COMPUTER && one && soundOf(one) ? [{ label: t('soundboards.showFileShort'), run: () => void showFile(one) }] : []),
      { label: t('soundboards.resetPad'), run: () => changeMany(pads, usualPadSettings()) },
      ...(pads.some((p) => p.key) ? [{ label: t('soundboards.clearKey'), run: () => changeMany(pads, { key: null }) }] : []),
      SEPARATOR,
      { label: t('soundboards.moveUp'), keys: 'Alt+↑', disabled: !one, run: () => one && movePad(one, -1) },
      { label: t('soundboards.moveDown'), keys: 'Alt+↓', disabled: !one, run: () => one && movePad(one, 1) },
      SEPARATOR,
      { label: t('soundboards.deleteMenu'), danger: true, keys: 'Del', run: (e) => void removePads(pads, e as { shiftKey?: boolean }) },
    ];
  };
  const emptyMenu = (): MenuItem[] => [
    { label: t('soundboards.addSounds'), keys: 'Ctrl+O', run: () => void chooseFiles() },
    { label: t('soundboards.newBoard'), run: () => void addBoard() },
    SEPARATOR,
    { label: t('soundboards.selectAll'), keys: 'Ctrl+A', disabled: shownPads.length === 0, run: () => setSelection(selectAll(shownPads.map((p) => p.id), selection)) },
    { label: t('soundboards.stopAll'), disabled: playing.length === 0, run: () => stop() },
  ];
  const boardMenu = (b: Board): MenuItem[] => [
    { label: t('soundboards.addSounds'), run: () => (pickBoard(b.id), void chooseFiles()) },
    { label: t('soundboards.newBoard'), run: () => void addBoard() },
    SEPARATOR,
    { label: t('soundboards.renameBoard'), keys: 'F2', run: () => setRenaming(`${BOARD}${b.id}`) },
    { label: t('soundboards.moveUp'), keys: 'Alt+↑', run: () => moveBoard(b.id, -1) },
    { label: t('soundboards.moveDown'), keys: 'Alt+↓', run: () => moveBoard(b.id, 1) },
    SEPARATOR,
    { label: t('soundboards.deleteBoardMenu'), danger: true, keys: 'Del', run: (e) => void removeBoard(b, e as { shiftKey?: boolean }) },
  ];
  const openMenu = (at: { x: number; y: number }, items: MenuItem[]) => setMenu({ ...at, items });

  // ---- the keys that act on the chosen pads, in the grid and in the table
  const padKeys = (e: KeyboardEvent, pads: Pad[], vertical: boolean): boolean => {
    const one = pads.length === 1 ? pads[0] : undefined;
    if (e.altKey && one && (e.key === 'ArrowUp' || e.key === 'ArrowDown' || (!vertical && (e.key === 'ArrowLeft' || e.key === 'ArrowRight')))) {
      movePad(one, e.key === 'ArrowUp' || e.key === 'ArrowLeft' ? -1 : 1);
      return true;
    }
    if (e.key === ' ' && pads.length > 0) return toggle(pads), true;
    if (e.key === 'Delete' && pads.length > 0) return void removePads(pads, e), true;
    if (e.key === 'F2' && one) return rename(one), true;
    return false;
  };

  // ---- the explorer
  const treeNodes: TreeNode[] = library.boards.map((b) => ({
    id: `${BOARD}${b.id}`,
    text: b.name,
    label: <Highlight text={b.name} query={query} />,
    icon: 'grid',
    className: status?.active_boards.includes(b.id) ? 'board-live' : '',
    title: status?.active_boards.includes(b.id) ? t('soundboards.live') : t('soundboards.notLive'),
    badge: b.pads.length,
    children: b.pads.map((p) => ({
      id: `${PAD}${p.id}`,
      text: p.name || t('soundboards.unnamedPad'),
      label: <Highlight text={p.name || t('soundboards.unnamedPad')} query={query} />,
      icon: 'music',
      className: `${b.id === current && playing.includes(p.id) ? 'pad-playing' : ''}${p.sound && library.sounds.some((s) => s.id === p.sound) ? '' : ' pad-lost'}`,
      dim: b.id === current && q !== '' && !shownPads.includes(p),
      badge: p.key ? <span className="kbd-chip">{p.key}</span> : undefined,
    })),
  }));
  // the chosen pads, or their soundboard while its pads are folded away
  const treeSelected = selection.selected.length > 0 && current && expanded.includes(`${BOARD}${current}`) ? selection.selected.map((id) => `${PAD}${id}`) : current ? [`${BOARD}${current}`] : [];
  const treeSelect = (id: string, mods: { ctrl: boolean; shift: boolean }) => {
    if (id.startsWith(BOARD)) return pickBoard(id.slice(BOARD.length));
    const padId = id.slice(PAD.length);
    const owner = ownerOf(padId);
    if (!owner) return;
    if (owner.id !== current) {
      setBoardId(owner.id);
      setSelection(selectKeys([padId]));
    } else setSelection(clickRow(owner.pads.map((p) => p.id), selection, padId, mods));
  };
  const treeKey = (e: KeyboardEvent, id: string): boolean => {
    if (id.startsWith(BOARD)) {
      const b = library.boards.find((x) => `${BOARD}${x.id}` === id);
      if (!b) return false;
      if (e.key === 'F2') return setRenaming(id), true;
      if (e.key === 'Delete') return void removeBoard(b, e), true;
      if (e.altKey && (e.key === 'ArrowUp' || e.key === 'ArrowDown')) return moveBoard(b.id, e.key === 'ArrowUp' ? -1 : 1), true;
      return false;
    }
    if (e.key === 'F2') return setRenaming(id), true;
    return padKeys(e, chosen.length > 0 ? chosen : [], true);
  };
  const treeRename = (id: string, text: string | null) => {
    setRenaming(null);
    if (text === null) return;
    if (id.startsWith(BOARD)) return renameBoard(id.slice(BOARD.length), text);
    const padId = id.slice(PAD.length);
    const pad = ownerOf(padId)?.pads.find((p) => p.id === padId);
    if (pad && text.trim()) change(pad, { name: text.trim() });
  };

  // the page's own shortcuts: Ctrl+F looks in the pads, Ctrl+O adds sounds
  const pageKeys = (e: KeyboardEvent) => {
    const k = e.key.toLowerCase();
    if ((e.ctrlKey || e.metaKey) && !e.altKey && k === 'f') {
      e.preventDefault();
      setFocusFilter((n) => n + 1);
    } else if ((e.ctrlKey || e.metaKey) && !e.altKey && k === 'o') {
      e.preventDefault();
      void chooseFiles();
    }
  };

  // ---- the table view
  const columns: Column<Pad>[] = [
    {
      id: 'play',
      label: '',
      width: 34,
      fixed: true,
      render: (p) =>
        playing.includes(p.id) ? (
          <Button variant="flat" size="small" icon="stop" title={t('soundboards.stopPad')} onClick={() => stop(p)} />
        ) : (
          <Button variant="flat" size="small" icon="play" disabled={!active || !soundOf(p)} title={!soundOf(p) ? t('soundboards.lost') : !active ? t('soundboards.cannotPlay') : t('soundboards.playPad')} onClick={() => void play(p)} />
        ),
    },
    {
      id: 'name',
      label: t('soundboards.col.name'),
      width: 220,
      fixed: true,
      sort: (p) => p.name,
      render: (p) => (
        <>
          <span className="pad-swatch" style={p.color ? ({ '--pad': p.color } as CSSProperties) : undefined} />
          <span className="ellipsis">
            <Highlight text={p.name || t('soundboards.unnamedPad')} query={query} />
          </span>
          {progress[p.id] !== undefined && <span className="row-progress" style={{ width: `${Math.round(progress[p.id] * 1000) / 10}%` }} />}
        </>
      ),
    },
    { id: 'file', label: t('soundboards.col.file'), width: 220, sort: (p) => soundOf(p)?.name ?? '', render: (p) => <span className={`ellipsis${soundOf(p) ? ' muted-cell' : ' warn'}`} title={soundOf(p)?.name}>{soundOf(p)?.name ?? t('soundboards.lost')}</span> },
    { id: 'duration', label: t('soundboards.col.duration'), width: 72, align: 'end', sort: (p) => soundOf(p)?.frames ?? 0, render: (p) => (soundOf(p) ? formatDuration(soundOf(p)!.frames) : '—') },
    { id: 'size', label: t('soundboards.col.size'), width: 84, align: 'end', hidden: true, sort: (p) => soundOf(p)?.bytes ?? 0, render: (p) => (soundOf(p) ? formatBytes(soundOf(p)!.bytes) : '—') },
    {
      id: 'volume',
      label: t('soundboards.volume'),
      width: 170,
      sort: (p) => p.gain_db ?? 0,
      render: (p) => (
        <span className="cell-slider">
          <Slider label={t('soundboards.volume')} min={-30} max={12} step={0.5} value={p.gain_db ?? 0} onChange={(gain_db) => change(p, { gain_db })} />
          <span className="pad-db">{formatDb(p.gain_db ?? 0)}</span>
        </span>
      ),
    },
    { id: 'key', label: t('soundboards.padKey'), width: 120, sort: (p) => p.key ?? '', render: (p) => (p.key ? <span className={`kbd-chip${problemsOf(p).length ? ' warn' : ''}`} title={problemsOf(p).join('\n') || undefined}>{p.key}</span> : <span className="muted-cell">—</span>) },
    { id: 'loop', label: t('soundboards.loop'), width: 64, align: 'center', sort: (p) => (p.looped ? 1 : 0), render: (p) => <Switch checked={p.looped ?? false} title={t('soundboards.loopHint')} onChange={(looped) => change(p, { looped })} /> },
    { id: 'exclusive', label: t('soundboards.exclusive'), width: 76, align: 'center', sort: (p) => (p.exclusive ? 1 : 0), render: (p) => <Switch checked={p.exclusive ?? false} title={t('soundboards.exclusiveHint')} onChange={(exclusive) => change(p, { exclusive })} /> },
    { id: 'hold', label: t('soundboards.hold'), width: 70, align: 'center', hidden: true, sort: (p) => (p.hold ? 1 : 0), render: (p) => (p.hold ? <Icon name="check" size={13} /> : null) },
    { id: 'fade', label: t('soundboards.fade'), width: 72, align: 'end', hidden: true, sort: (p) => p.fade_ms ?? 0, render: (p) => (p.fade_ms ? `${p.fade_ms} ms` : '—') },
    { id: 'start', label: t('soundboards.start'), width: 70, align: 'end', hidden: true, sort: (p) => p.start_ms ?? 0, render: (p) => (p.start_ms ? `${(p.start_ms / 1000).toFixed(1)} s` : '—') },
    { id: 'end', label: t('soundboards.end'), width: 70, align: 'end', hidden: true, sort: (p) => p.end_ms ?? 0, render: (p) => (p.end_ms ? `${(p.end_ms / 1000).toFixed(1)} s` : '—') },
  ];

  const center = !board ? (
    <div className="boards-empty">
      <p>{t('soundboards.none')}</p>
      <Button variant="primary" icon="plus" onClick={() => void addBoard()}>
        {t('soundboards.newBoard')}
      </Button>
    </div>
  ) : view === 'table' ? (
    <DataTable
      id="pads"
      label={board.name}
      rows={shownPads}
      rowKey={(p) => p.id}
      columns={columns}
      selection={selection}
      onSelection={setSelection}
      onActivate={(p) => toggle([p])}
      onKey={(e, pads) => padKeys(e, pads, true)}
      onMenu={(at, pads) => openMenu(at, pads.length > 0 ? padMenu(pads) : emptyMenu())}
      rowClass={(p) => `${playing.includes(p.id) ? 'playing' : ''}${soundOf(p) ? '' : ' lost'}`}
      empty={q ? t('soundboards.noMatch') : t('soundboards.emptyBoard')}
    />
  ) : (
    <PadGrid
      pads={shownPads}
      selection={selection}
      onSelection={setSelection}
      cardWidth={cardWidth}
      label={board.name}
      onKey={(e, pads) => padKeys(e, pads, false)}
      onMenu={(at, pads) => openMenu(at, pads.length > 0 ? padMenu(pads) : emptyMenu())}
      empty={q ? t('soundboards.noMatch') : t('soundboards.emptyBoard')}
      render={(pad) => (
        <PadCard
          pad={pad}
          sound={soundOf(pad)}
          playing={playing.includes(pad.id)}
          progress={progress[pad.id]}
          canPlay={active}
          problems={problemsOf(pad)}
          onChange={(patch) => change(pad, patch)}
          onPlay={() => void play(pad)}
          onStop={() => stop(pad)}
          onSettings={() => {
            setSelection(selectKeys([pad.id]));
            setInspector(true);
          }}
          onRemove={(e) => void removePads([pad], e)}
          onContextMenu={() => undefined}
        />
      )}
      after={mine.map((i) => (
        <ImportCard key={i.job} job={imports.progress[i.job]} label={i.label} error={i.error} onDismiss={() => imports.dismiss(i.job)} />
      ))}
    />
  );

  const totalFrames = (board?.pads ?? []).reduce((n, p) => n + (soundOf(p)?.frames ?? 0), 0);
  return (
    <div
      className={`ide-page soundboards-page${over ? ' over' : ''}`}
      aria-label={t('soundboards.title')}
      onKeyDown={pageKeys}
      onDragOver={(e) => {
        if (IN_DESKTOP_WINDOW || !e.dataTransfer.types.includes('Files')) return;
        e.preventDefault();
        setOver(true);
      }}
      onDragLeave={(e) => {
        if (!e.currentTarget.contains(e.relatedTarget as Node | null)) setOver(false);
      }}
      onDrop={(e) => {
        if (IN_DESKTOP_WINDOW) return;
        e.preventDefault();
        setOver(false);
        addFiles(Array.from(e.dataTransfer.files));
      }}
    >
      <Toolbar label={t('soundboards.title')}>
        <Button size="small" variant="primary" icon="plus" title="Ctrl+O" onClick={() => void chooseFiles()}>
          {t('soundboards.addSounds')}
        </Button>
        <Button size="small" icon="grid" onClick={() => void addBoard()}>
          {t('soundboards.newBoard')}
        </Button>
        <ToolSeparator />
        <Button size="small" icon="play" disabled={!active || chosen.length === 0} title={`${t('soundboards.playChosen')} (${t('keys.space')})`} onClick={() => chosen.forEach((p) => void play(p))} />
        <Button size="small" icon="stop" disabled={playing.length === 0} title={t('soundboards.stopAll')} onClick={() => stop()} />
        <ToolSeparator />
        <FilterBox value={query} onChange={setQuery} placeholder={t('soundboards.filter')} count={board ? `${shownPads.length}/${board.pads.length}` : undefined} focusKey={focusFilter} />
        <div className="segmented" role="group" aria-label={t('soundboards.view')}>
          <Button size="small" icon="grid" pressed={view === 'grid'} title={t('soundboards.viewGrid')} onClick={() => setView('grid')} />
          <Button size="small" icon="list" pressed={view === 'table'} title={t('soundboards.viewTable')} onClick={() => setView('table')} />
        </div>
        {view === 'grid' && (
          <span className="tool-zoom" title={t('soundboards.cardSize')}>
            <Slider label={t('soundboards.cardSize')} min={160} max={420} step={10} value={cardWidth} onChange={setCardWidth} />
          </span>
        )}
        <span className="spacer" />
        <span className="tool-text">{IN_DESKTOP_WINDOW ? t('soundboards.dropHintDesktop') : t('soundboards.dropHint')}</span>
        <ToolSeparator />
        <Button size="small" icon="sideLeft" pressed={explorer} title={t('soundboards.explorer')} onClick={() => setExplorer(!explorer)} />
        <Button size="small" icon="sideBottom" pressed={libraryPane} title={t('soundboards.libraryTitle')} onClick={() => setLibraryPane(!libraryPane)} />
        <Button size="small" icon="sideRight" pressed={inspector} title={t('soundboards.inspector')} onClick={() => setInspector(!inspector)} />
      </Toolbar>
      <div className="ide-columns">
        {explorer && (
          <SidePane id="boards-explorer" side="left" usual={250} min={160} max={520}>
            <Pane
              title={t('soundboards.explorer')}
              actions={
                <>
                  <Button variant="flat" size="small" icon="plus" title={t('soundboards.newBoard')} onClick={() => void addBoard()} />
                  <Button variant="flat" size="small" icon="unfold" title={t('tree.expandAll')} onClick={() => setExpanded(library.boards.map((b) => `${BOARD}${b.id}`))} />
                  <Button variant="flat" size="small" icon="collapse" title={t('tree.collapseAll')} onClick={() => setExpanded([])} />
                </>
              }
            >
              {library.boards.length === 0 ? (
                <p className="pane-empty">{t('soundboards.none')}</p>
              ) : (
                <TreeView
                  nodes={treeNodes}
                  label={t('soundboards.explorer')}
                  selected={treeSelected}
                  expanded={expanded}
                  onToggle={(id) => setExpanded((x) => (x.includes(id) ? x.filter((y) => y !== id) : [...x, id]))}
                  onSelect={treeSelect}
                  onActivate={(id) => {
                    if (id.startsWith(BOARD)) setExpanded((x) => (x.includes(id) ? x.filter((y) => y !== id) : [...x, id]));
                    else {
                      const pad = ownerOf(id.slice(PAD.length))?.pads.find((p) => `${PAD}${p.id}` === id);
                      if (pad && ownerOf(pad.id)?.id === current) toggle([pad]);
                    }
                  }}
                  onKey={treeKey}
                  onMenu={(at, id) => {
                    if (!id) return openMenu(at, [{ label: t('soundboards.newBoard'), run: () => void addBoard() }]);
                    if (id.startsWith(BOARD)) {
                      const b = library.boards.find((x) => `${BOARD}${x.id}` === id);
                      return b && openMenu(at, boardMenu(b));
                    }
                    const owner = ownerOf(id.slice(PAD.length));
                    const pads = owner?.id === current && chosen.length > 0 ? chosen : (owner?.pads.filter((p) => `${PAD}${p.id}` === id) ?? []);
                    openMenu(at, padMenu(pads));
                  }}
                  renaming={renaming}
                  onRename={treeRename}
                />
              )}
            </Pane>
          </SidePane>
        )}
        <div className="ide-center">
          <Pane
            className="board-pane"
            title={
              board ? (
                <span className="board-title">
                  <span className={`state-dot ${active ? 'on' : ''}`} />
                  {board.name}
                  <span className="board-meta">
                    {t('soundboards.padCount', { count: board.pads.length })} · {formatDuration(totalFrames)}
                    {!active && ` · ${t('soundboards.notLive')}`}
                  </span>
                </span>
              ) : (
                t('soundboards.title')
              )
            }
            actions={chosen.length > 0 ? <span className="board-meta">{t('soundboards.chosen', { count: chosen.length })}</span> : undefined}
          >
            {view === 'table' && mine.length > 0 && (
              <div className="imports-strip">
                {mine.map((i) => (
                  <ImportCard key={i.job} job={imports.progress[i.job]} label={i.label} error={i.error} onDismiss={() => imports.dismiss(i.job)} />
                ))}
              </div>
            )}
            {center}
          </Pane>
          {libraryPane && (
            <SidePane id="boards-library" side="bottom" usual={260} min={120} max={900}>
              <Pane title={t('soundboards.libraryTitle')} actions={<Button variant="flat" size="small" icon="close" title={t('app.close')} onClick={() => setLibraryPane(false)} />}>
                <SoundsManager daemon={daemon} library={library} setLibrary={setLibrary} imports={imports} notify={notify} onAddToBoard={(s) => void addFromLibrary(s)} boardName={board?.name} />
              </Pane>
            </SidePane>
          )}
        </div>
        {inspector && (
          <SidePane id="boards-inspector" side="right" usual={320} min={240} max={640}>
            <Pane title={t('soundboards.inspector')} actions={<Button variant="flat" size="small" icon="close" title={t('app.close')} onClick={() => setInspector(false)} />}>
              {chosen.length === 1 ? (
                <div className="pad-inspector">
                  <PadSettings
                    pad={chosen[0]}
                    sound={soundOf(chosen[0])}
                    problems={problemsOf(chosen[0])}
                    compressing={compressing === chosen[0].sound}
                    onChange={(patch) => change(chosen[0], patch)}
                    onCompress={() => {
                      const s = soundOf(chosen[0]);
                      if (s) void compressSound(s);
                    }}
                    onReveal={() => void showFile(chosen[0])}
                  />
                </div>
              ) : chosen.length > 1 ? (
                <ManyPads pads={chosen} playing={playing} active={active} onChange={(patch) => changeMany(chosen, patch)} onPlay={() => chosen.forEach((p) => void play(p))} onStop={() => chosen.forEach((p) => playing.includes(p.id) && stop(p))} onRemove={(e) => void removePads(chosen, e)} />
              ) : board ? (
                <div className="board-inspector">
                  <h3>{board.name}</h3>
                  <dl className="facts">
                    <dt>{t('soundboards.padsLabel')}</dt>
                    <dd>{board.pads.length}</dd>
                    <dt>{t('soundboards.col.duration')}</dt>
                    <dd>{formatDuration(totalFrames)}</dd>
                    <dt>{t('soundboards.stateLabel')}</dt>
                    <dd>{active ? t('soundboards.live') : t('soundboards.notLive')}</dd>
                    <dt>{t('soundboards.playingLabel')}</dt>
                    <dd>{playing.length}</dd>
                  </dl>
                  <p className="muted">{t('soundboards.pickHint')}</p>
                  <div className="row">
                    <Button size="small" icon="pencil" onClick={() => setRenaming(`${BOARD}${board.id}`)} disabled={!explorer}>
                      {t('soundboards.renameBoard')}
                    </Button>
                    <Button size="small" icon="trash" title={t('dialog.shiftHint')} onClick={(e) => void removeBoard(board, e)}>
                      {t('soundboards.deleteBoardMenu')}
                    </Button>
                  </div>
                </div>
              ) : (
                <p className="pane-empty">{t('soundboards.none')}</p>
              )}
            </Pane>
          </SidePane>
        )}
      </div>
      {over && <div className="drop-veil">{t('soundboards.dropHere', { name: board?.name ?? t('soundboards.boardBase') })}</div>}
      {menu && <ContextMenu x={menu.x} y={menu.y} items={menu.items} onClose={() => setMenu(null)} />}
    </div>
  );
}

/**
 * The pads as a grid of cards that is driven like a list: a click chooses one (Ctrl, Shift for more), the arrows move in two dimensions,
 * Ctrl+A chooses them all, Escape none. The cards keep their own buttons and fields.
 */
function PadGrid({
  pads,
  selection,
  onSelection,
  cardWidth,
  label,
  render,
  onKey,
  onMenu,
  empty,
  after,
}: {
  pads: Pad[];
  selection: ListSelection;
  onSelection: (s: ListSelection) => void;
  cardWidth: number;
  label: string;
  render: (pad: Pad) => React.ReactNode;
  onKey: (e: KeyboardEvent, pads: Pad[]) => boolean;
  onMenu: (at: { x: number; y: number }, pads: Pad[]) => void;
  empty: string;
  after: React.ReactNode;
}) {
  const box = useRef<HTMLDivElement>(null);
  const keys = pads.map((p) => p.id);
  const chosen = pads.filter((p) => selection.selected.includes(p.id));
  // how many cards a row holds, read from where they are drawn
  const columns = () => {
    const cells = [...(box.current?.querySelectorAll<HTMLElement>('.pad-cell') ?? [])];
    const top = cells[0]?.offsetTop;
    const n = cells.filter((c) => c.offsetTop === top).length;
    return Math.max(1, n);
  };
  useEffect(() => {
    if (selection.focus) box.current?.querySelector(`[data-pad="${CSS.escape(selection.focus)}"]`)?.scrollIntoView({ block: 'nearest' });
  }, [selection.focus]);
  const key = (e: KeyboardEvent<HTMLDivElement>) => {
    if (isInteractive(e.target)) return;
    const mods = { ctrl: e.ctrlKey || e.metaKey, shift: e.shiftKey };
    const move = (step: number) => onSelection(moveCursor(keys, selection, step, mods));
    if (!e.altKey && e.key === 'ArrowRight') move(1);
    else if (!e.altKey && e.key === 'ArrowLeft') move(-1);
    else if (!e.altKey && e.key === 'ArrowDown') move(columns());
    else if (!e.altKey && e.key === 'ArrowUp') move(-columns());
    else if (e.key === 'Home') move(-Infinity);
    else if (e.key === 'End') move(Infinity);
    else if (mods.ctrl && e.key.toLowerCase() === 'a') onSelection(selectAll(keys, selection));
    else if (e.key === 'Escape' && selection.selected.length > 0) onSelection({ selected: [], focus: selection.focus, anchor: null });
    else if (e.key === 'ContextMenu' || (e.key === 'F10' && e.shiftKey)) {
      const r = box.current?.querySelector(`[data-pad="${CSS.escape(selection.focus ?? '')}"]`)?.getBoundingClientRect() ?? box.current?.getBoundingClientRect();
      if (r) onMenu({ x: r.left + 24, y: r.top + 24 }, chosen);
    } else if (!onKey(e, chosen)) return;
    e.preventDefault();
  };
  return (
    <div
      ref={box}
      className="pad-grid"
      role="listbox"
      aria-label={label}
      aria-multiselectable
      tabIndex={0}
      style={{ '--card': `${cardWidth}px` } as CSSProperties}
      onKeyDown={key}
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) {
          box.current?.focus({ preventScroll: true });
          if (e.button === 0 && !e.ctrlKey && !e.shiftKey) onSelection(NOTHING);
        }
      }}
      onContextMenu={(e) => {
        e.preventDefault();
        onMenu({ x: e.clientX, y: e.clientY }, []);
      }}
    >
      {pads.map((pad) => {
        const on = selection.selected.includes(pad.id);
        return (
          <div
            key={pad.id}
            data-pad={pad.id}
            role="option"
            aria-selected={on}
            className={`pad-cell${on ? ' selected' : ''}${selection.focus === pad.id ? ' cursor' : ''}`}
            onMouseDown={(e) => {
              if (!isInteractive(e.target)) box.current?.focus({ preventScroll: true });
              if (e.button === 2) {
                if (!on) onSelection(clickRow(keys, selection, pad.id));
                return;
              }
              if (e.button !== 0 || (isInteractive(e.target) && on)) return;
              onSelection(clickRow(keys, selection, pad.id, { ctrl: e.ctrlKey || e.metaKey, shift: e.shiftKey && !isInteractive(e.target) }));
            }}
            onContextMenu={(e) => {
              if (isInteractive(e.target) && (e.target as HTMLElement).closest('input')) return;
              e.preventDefault();
              e.stopPropagation();
              onMenu({ x: e.clientX, y: e.clientY }, on ? chosen : [pad]);
            }}
          >
            {render(pad)}
          </div>
        );
      })}
      {after}
      {pads.length === 0 && <p className="pane-empty grid-empty">{empty}</p>}
    </div>
  );
}

/** What several chosen pads share: their volume, whether they loop or cut the others, and what can be done to all of them. */
function ManyPads({ pads, playing, active, onChange, onPlay, onStop, onRemove }: { pads: Pad[]; playing: string[]; active: boolean; onChange: (patch: Partial<Pad>) => void; onPlay: () => void; onStop: () => void; onRemove: (e: React.MouseEvent) => void }) {
  const t = useT();
  const gains = new Set(pads.map((p) => p.gain_db ?? 0));
  const gain = gains.size === 1 ? [...gains][0] : 0;
  const all = (f: (p: Pad) => boolean | undefined) => pads.every((p) => !!f(p));
  return (
    <div className="many-pads">
      <h3>{t('soundboards.chosen', { count: pads.length })}</h3>
      <div className="settings-grid">
        <div className="srow">
          <span>{t('soundboards.volume')}</span>
          <div className="srow-inline">
            <Slider label={t('soundboards.volume')} min={-30} max={12} step={0.5} value={gain} onChange={(gain_db) => onChange({ gain_db })} />
            <span className="pad-db">{gains.size === 1 ? formatDb(gain) : t('soundboards.mixed')}</span>
          </div>
        </div>
        <div className="srow">
          <span>{t('soundboards.options')}</span>
          <div className="checks">
            <Switch checked={all((p) => p.looped)} title={t('soundboards.loopHint')} onChange={(looped) => onChange({ looped })}>
              {t('soundboards.loop')}
            </Switch>
            <Switch checked={all((p) => p.exclusive)} title={t('soundboards.exclusiveHint')} onChange={(exclusive) => onChange({ exclusive })}>
              {t('soundboards.exclusive')}
            </Switch>
          </div>
        </div>
      </div>
      <div className="row">
        <Button size="small" icon="play" disabled={!active} onClick={onPlay}>
          {t('soundboards.play')}
        </Button>
        <Button size="small" icon="stop" disabled={!pads.some((p) => playing.includes(p.id))} onClick={onStop}>
          {t('soundboards.stopPad')}
        </Button>
        <Button size="small" icon="undo" onClick={() => onChange(usualPadSettings())}>
          {t('soundboards.resetPad')}
        </Button>
        <Button size="small" icon="trash" title={t('dialog.shiftHint')} onClick={onRemove}>
          {t('soundboards.deleteMenu')}
        </Button>
      </div>
    </div>
  );
}
