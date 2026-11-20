import {
  Background,
  Controls,
  MiniMap,
  ReactFlow,
  addEdge,
  useEdgesState,
  useNodesState,
  useReactFlow,
  type Connection as FlowConnection,
  type Edge,
} from '@xyflow/react';
import '@xyflow/react/dist/style.css';
import { useCallback, useEffect, useMemo, useRef, useState, type CSSProperties } from 'react';
import { EVENT_GRAPH, EVENT_STATUS, IN_DESKTOP_WINDOW, Rpc, RpcError, api, findToken, startDaemon, type Connection } from './api.ts';
import { Editor, type EditorContext } from './context.ts';
import { NodeCard, type OmuNode } from './components/NodeCard.tsx';
import { Doc, docKey, type DocRef } from './components/BlockHelp.tsx';
import { Calibration } from './components/Calibration.tsx';
import { CommandPalette, type PaletteItem } from './components/CommandPalette.tsx';
import { History } from './history.ts';
import { Button } from './kit/controls.tsx';
import { loadDraft, loadVersions, saveDraft, saveVersions, withVersion, type Version } from './versions.ts';
import { bindings, commandOf, loadOverrides, type CommandId } from './keybindings.ts';
import { copyOf, pasteOf, type Clip, type ClipEdge, type ClipNode } from './selection.ts';
import { DockFrame, InspectorPanel, LogsPanel, MonitorPanel, ProblemsPanel, Splitter, StatusBar, VersionsPanel, loadDock, saveDock, type DockState } from './components/Workbench.tsx';
import { ContextMenu, type MenuItem } from './components/ContextMenu.tsx';
import { BlocksBar, type DragIn } from './components/BlocksBar.tsx';
import { FloatingWindow } from './components/FloatingWindow.tsx';
import { JsonWindow } from './components/ToolWindows.tsx';
import { flashPort } from './components/BlockHelp.tsx';
import { Modules } from './components/Modules.tsx';
import { SettingsPanel } from './components/SettingsPanel.tsx';
import { Soundboards } from './components/Soundboards.tsx';
import {
  GAP,
  GRID,
  KINDS_BY_CATEGORY,
  NODE_TYPES,
  NODE_WIDTH,
  canConnect,
  canonical,
  categoryOf,
  defaultKind,
  baseHandle,
  edgeId,
  flowOfEdge,
  fromFlow,
  newId,
  nameOf,
  namedProblem,
  layoutOf,
  MAX_ENTRIES,
  prunedInputs,
  toFlow,
} from './graph.ts';
import { useI18n } from './i18n/index.tsx';
import { SETTING_DECLS } from './settingsRegistry.ts';
import { useTheme } from './theme.tsx';
import type { AppInfo, Category, DeviceInfo, Diagnostic, EngineStatus, GraphSpec, InputValue, Layout, NodeKind, NodeSpec, NodeType, SoundLibrary, StaleDevices, SystemInfo } from './types.ts';
import { PRODUCT_NAME } from './types.ts';

const nodeTypes = { omu: NodeCard };

const snapToGrid = (v: number) => Math.round(v / GRID) * GRID;
/** The top left corner of a block that is centred, as far as the grid allows, on point `p` of the graph. */
const cornerFor = (p: { x: number; y: number }) => ({ x: snapToGrid(p.x - NODE_WIDTH / 2), y: snapToGrid(p.y - GRID) });

/** The tabs of the top bar: the graph, then the places that were panels over it. */
type Tab = 'nodes' | 'soundboards' | 'modules' | 'settings';
const TABS: { id: Tab; label: string }[] = [
  { id: 'nodes', label: 'app.tabNodes' },
  { id: 'soundboards', label: 'soundboards.title' },
  { id: 'modules', label: 'modules.title' },
  { id: 'settings', label: 'settings.title' },
];

function message(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

/** The display name of a block, to be kept when its settings change. */
const named = (spec: NodeSpec): { display?: string; inputs?: NodeSpec['inputs'] } => ({ ...(spec.display === undefined ? {} : { display: spec.display }), ...(spec.inputs === undefined ? {} : { inputs: spec.inputs }) });

const asEdge = (e: Edge) => ({ id: e.id, source: e.source, target: e.target, sourceHandle: e.sourceHandle ?? '', targetHandle: e.targetHandle ?? '' });

export function App() {
  const rpc = useMemo(() => new Rpc(findToken()), []);
  const daemon = useMemo(() => api(rpc), [rpc]);
  const flow = useReactFlow();
  const { t } = useI18n();
  const { resolved } = useTheme();

  const [conn, setConn] = useState<Connection>('connecting');
  const [info, setInfo] = useState<SystemInfo | null>(null);
  const [nodes, setNodes, onNodesChange] = useNodesState<OmuNode>([]);
  const [edges, setEdges, onEdgesChange] = useEdgesState<Edge>([]);
  const [applied, setApplied] = useState<GraphSpec>({ nodes: [], edges: [] });
  const [status, setStatus] = useState<EngineStatus | null>(null);
  const [apps, setApps] = useState<AppInfo[]>([]);
  const [devices, setDevices] = useState<DeviceInfo[]>([]);
  const [problems, setProblems] = useState<string[]>([]);
  const [notice, setNotice] = useState<string | null>(null);
  // the old virtual devices Windows still remembers, and whether a cleanup is running
  const [stale, setStale] = useState<StaleDevices | null>(null);
  const [cleaning, setCleaning] = useState(false);
  const [busy, setBusy] = useState(false);
  const [showJson, setShowJson] = useState(false);
  const [dock, setDockState] = useState<DockState>(loadDock);
  const setDock = useCallback((d: DockState) => {
    setDockState(d);
    saveDock(d);
  }, []);
  // what is wrong with the graph, asked of the daemon (the same rules as when it is applied) a moment after each change
  const [diagnostics, setDiagnostics] = useState<Diagnostic[]>([]);
  const [versions, setVersions] = useState<Version[]>(loadVersions);
  // changes that were not applied when the page was closed
  const [draftOffer, setDraftOffer] = useState<Version | null>(null);
  const firstLoad = useRef(true);
  // the draft is only kept once what the daemon has is on screen (before that, nothing differs)
  const [loaded, setLoaded] = useState(false);
  // the soundboards the daemon keeps; `boardsFirst` is the board the soundboards tab opens on
  const [library, setLibrary] = useState<SoundLibrary>({ sounds: [], boards: [] });
  const [boardsFirst, setBoardsFirst] = useState<string | undefined>(undefined);
  // the tab shown: the graph, the soundboards, the optional modules (FFmpeg) or the settings
  const [tab, setTab] = useState<Tab>('nodes');
  // the information windows that are open, the last one in front
  const [helps, setHelps] = useState<{ key: string; doc: DocRef; x: number; y: number }[]>([]);
  // the calibration windows that are open (a block each)
  const [calibrations, setCalibrations] = useState<{ node: string; x: number; y: number }[]>([]);
  // the search bar (everything, or only the blocks to add, at a point of the graph), the setting it sent us to, the menu of a right click
  const [palette, setPalette] = useState<{ mode: 'all' | 'block'; at?: { x: number; y: number }; initial?: string } | null>(null);
  // the keyboard shortcuts: the defaults, and what the person changed (kept in this browser)
  const [overrides] = useState(loadOverrides);
  const keys = useMemo(() => bindings(overrides), [overrides]);
  // the steps of the session (undo, redo) and what was copied
  type Step = { nodes: ClipNode[]; edges: (ClipEdge & { id: string })[] };
  const history = useRef(new History<Step>((s) => JSON.stringify(s)));
  const clipboard = useRef<Clip | null>(null);
  const [settingFocus, setSettingFocus] = useState<{ id: string; nonce: number } | undefined>(undefined);
  const [menu, setMenu] = useState<{ x: number; y: number; items: MenuItem[] } | null>(null);
  const pointer = useRef<{ x: number; y: number } | null>(null);
  const layoutTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  const graph = useMemo(() => fromFlow(nodes.map((n) => ({ spec: n.data.spec })), edges.map(asEdge)), [nodes, edges]);
  const dirty = canonical(graph) !== canonical(applied);

  // keep the latest values for the callbacks that must not change identity
  const nodesRef = useRef(nodes);
  const edgesRef = useRef(edges);
  nodesRef.current = nodes;
  edgesRef.current = edges;
  const dirtyRef = useRef(dirty);
  dirtyRef.current = dirty;

  // Saves where the blocks are and how their links are laid out, a moment after the last change (what is saved is read
  // when the moment comes, so a change made in between is included).
  const saveLayout = useCallback(
    (list?: OmuNode[]) => {
      if (layoutTimer.current) clearTimeout(layoutTimer.current);
      layoutTimer.current = setTimeout(() => {
        const shown = list ?? nodesRef.current;
        const flowNodes = shown.map((n) => ({ id: n.id, position: n.position, spec: n.data.spec, inputs: n.data.inputs }));
        daemon.layoutSet(layoutOf(flowNodes, edgesRef.current.map(asEdge))).catch(() => undefined);
      }, 500);
    },
    [daemon],
  );

  // The drawing as last shown or saved: when what is on screen differs (a link moved to another point, a point added,
  // a block renamed), it is saved. Nothing is saved before the first graph is shown, so an empty editor never
  // overwrites a saved layout.
  const savedLayout = useRef<string | null>(null);
  useEffect(() => {
    if (savedLayout.current === null) return;
    const now = JSON.stringify(layoutOf(nodes.map((n) => ({ id: n.id, position: n.position, spec: n.data.spec, inputs: n.data.inputs })), edges.map(asEdge)));
    if (now === savedLayout.current) return;
    savedLayout.current = now;
    saveLayout();
  }, [nodes, edges, saveLayout]);

  const show = useCallback(
    (g: GraphSpec, layout: Layout) => {
      const f = toFlow(g, layout);
      savedLayout.current = JSON.stringify(layoutOf(f.nodes, f.edges));
      setNodes(f.nodes.map((n) => ({ id: n.id, type: 'omu', position: n.position, data: { spec: n.spec, inputs: n.inputs } })));
      setEdges(f.edges.map((e) => ({ ...e })));
      setApplied(g);
    },
    [setNodes, setEdges],
  );

  const refreshStale = useCallback(() => {
    daemon.staleDevices().then(setStale).catch(() => setStale(null));
  }, [daemon]);

  const load = useCallback(async () => {
    history.current.reset();
    try {
      const [i, g, layout, a, d, lib] = await Promise.all([daemon.systemInfo(), daemon.graphGet(), daemon.layoutGet(), daemon.apps(), daemon.devices(), daemon.soundboards()]);
      setInfo(i);
      setLibrary(lib);
      setApps(a);
      setDevices(d);
      show(g, layout);
      setProblems([]);
      refreshStale();
      // the first time only: changes left unapplied by the last session are offered back
      if (firstLoad.current) {
        firstLoad.current = false;
        const draft = loadDraft();
        if (draft && canonical(draft.graph) !== canonical(g)) setDraftOffer(draft);
        else saveDraft(null);
        setLoaded(true);
      }
    } catch (e) {
      setNotice(message(e));
    }
  }, [daemon, show, refreshStale]);

  // connection, first load, events
  useEffect(() => {
    const offState = rpc.onState((c) => {
      setConn(c);
      if (c === 'open') void load();
    });
    const offEvent = rpc.onEvent((method, params) => {
      if (method === EVENT_STATUS) setStatus(params as EngineStatus);
      if (method === EVENT_GRAPH) {
        if (dirtyRef.current) setNotice('app.changedElsewhere');
        else void daemon.graphGet().then((g) => daemon.layoutGet().then((l) => show(g, l)));
      }
    });
    rpc.start();
    return () => {
      offState();
      offEvent();
      rpc.stop();
    };
  }, [rpc, daemon, load, show]);

  // applications and devices change while the panel is open
  useEffect(() => {
    if (conn !== 'open') return;
    const timer = setInterval(() => {
      daemon.apps().then(setApps).catch(() => undefined);
      daemon.devices().then(setDevices).catch(() => undefined);
    }, 4000);
    return () => clearInterval(timer);
  }, [conn, daemon]);

  // where the blocks are now, as the daemon keeps it
  const currentLayout = () =>
    layoutOf(
      nodes.map((n) => ({ id: n.id, position: n.position, spec: n.data.spec, inputs: n.data.inputs })),
      edges.map(asEdge),
    );

  const apply = async () => {
    setBusy(true);
    setNotice(null);
    try {
      await daemon.graphSet(graph);
      setApplied(graph);
      setProblems([]);
      // what was applied is a version to come back to, and nothing is left to rescue
      const kept = withVersion(versions, { at: new Date().toISOString(), graph, layout: currentLayout() });
      setVersions(kept);
      saveVersions(kept);
      saveDraft(null);
      // a renamed or removed device leaves an old one behind in Windows: the button shows how many
      refreshStale();
    } catch (e) {
      setProblems(e instanceof RpcError && e.problems.length ? e.problems : [message(e)]);
      if (e instanceof RpcError && e.diagnostics.length) setDiagnostics(e.diagnostics);
    } finally {
      setBusy(false);
    }
  };

  const revert = () => {
    setNotice(null);
    void load();
  };

  // Stopping the daemon ends all audio and removes the virtual devices: it asks twice (a second click within 4 s).
  const [confirmShutdown, setConfirmShutdown] = useState(false);
  useEffect(() => {
    if (!confirmShutdown) return;
    const timer = window.setTimeout(() => setConfirmShutdown(false), 4000);
    return () => window.clearTimeout(timer);
  }, [confirmShutdown]);
  const shutdownDaemon = async () => {
    if (!confirmShutdown) {
      setConfirmShutdown(true);
      return;
    }
    setConfirmShutdown(false);
    try {
      await daemon.shutdown();
    } catch (e) {
      // the daemon may close the connection before the reply reaches us (or be gone already): that is the answer too
      if (e instanceof RpcError && e.code !== -1) setNotice(message(e));
    }
  };

  // Makes Windows forget the old virtual devices. The daemon asks for administrator rights only if it has none already
  // (the system shows its own confirmation window then).
  const cleanDevices = async () => {
    setCleaning(true);
    setNotice(null);
    try {
      const done = await daemon.cleanDevices();
      setNotice(done.removed > 0 ? t('app.cleanDone', { count: done.removed }) : t('app.cleanNothing'));
    } catch (e) {
      setNotice(e instanceof RpcError && e.code === -32004 ? 'app.cleanCancelled' : message(e));
    } finally {
      setCleaning(false);
      refreshStale();
    }
  };

  const startEngine = async () => {
    try {
      await daemon.start();
    } catch (e) {
      setNotice(message(e));
    }
  };

  const stopEngine = async () => {
    try {
      await daemon.stop();
    } catch (e) {
      setNotice(message(e));
    }
  };

  /** Adds a block of `category` (its first kind unless `kind` is given), at `at` (a point of the graph) or in the middle of the view. Returns its identifier. */
  const addNode = (category: Category, kind?: NodeKind, at?: { x: number; y: number }): string => {
    const type = KINDS_BY_CATEGORY[category][0];
    // a random identifier: the name people give a block is a separate thing (its display name)
    const id = newId();
    const position = cornerFor(at ?? flow.screenToFlowPosition({ x: window.innerWidth / 2, y: window.innerHeight / 2 }));
    // from the middle of the view: slide away from the blocks already there
    while (!at && nodesRef.current.some((n) => Math.abs(n.position.x - position.x) < 2 * GRID && Math.abs(n.position.y - position.y) < 2 * GRID)) {
      position.x += 2 * GRID;
      position.y += 2 * GRID;
    }
    const node: OmuNode = { id, type: 'omu', position, data: { spec: { id, ...(kind ?? defaultKind(type)) } } };
    setNodes((list) => [...list, node]);
    saveLayout([...nodesRef.current, node]);
    return id;
  };

  // a block dragged out of the tray: made where the pointer is, then moved with it
  const dragIn: DragIn = {
    begin: (type, x, y) => addNode(categoryOf(type), defaultKind(type), flow.screenToFlowPosition({ x, y })),
    move: (id, x, y) => {
      const position = cornerFor(flow.screenToFlowPosition({ x, y }));
      setNodes((list) => list.map((n) => (n.id === id && (n.position.x !== position.x || n.position.y !== position.y) ? { ...n, position } : n)));
    },
    end: () => saveLayout(),
  };

  const autoLayout = () => {
    const f = toFlow(graph, {});
    setNodes((list) => list.map((n) => ({ ...n, position: f.nodes.find((x) => x.id === n.id)?.position ?? n.position })));
    saveLayout();
    window.setTimeout(() => void flow.fitView({ duration: 300, padding: 0.2 }), 50);
  };

  const onConnect = useCallback(
    (c: FlowConnection) => {
      setEdges((list) => {
        const id = edgeId({ from: { node: c.source, port: c.sourceHandle ?? '' }, to: { node: c.target, port: baseHandle(c.targetHandle ?? '') } });
        return addEdge({ ...c, id }, list);
      });
    },
    [setEdges],
  );

  const isValidConnection = useCallback(
    (c: { source: string; target: string; sourceHandle?: string | null; targetHandle?: string | null }) =>
      canConnect(
        nodesRef.current.map((n) => ({ id: n.id, spec: n.data.spec })),
        edgesRef.current.map(asEdge),
        { source: c.source, target: c.target, sourceHandle: c.sourceHandle ?? null, targetHandle: c.targetHandle ?? null },
      ),
    [],
  );

  // opens the search bar; to add blocks it remembers where the pointer is over the graph, to put them there
  const openPalette = (mode: 'all' | 'block', at?: { x: number; y: number }, initial?: string) => {
    const p = pointer.current;
    const canvas = document.querySelector('.canvas')?.getBoundingClientRect();
    const inside = p && canvas && p.x >= canvas.left && p.x <= canvas.right && p.y >= canvas.top && p.y <= canvas.bottom;
    setPalette({ mode, at: at ?? (inside && tab === 'nodes' ? flow.screenToFlowPosition(p) : undefined), initial });
  };

  // ---- editing: what the editor holds, as a step of the session
  const stepNow = (): Step => ({
    nodes: nodes.map((n) => ({ id: n.id, position: n.position, spec: n.data.spec, ...(n.data.inputs === undefined ? {} : { inputs: n.data.inputs }) })),
    edges: edges.map((e) => ({ ...asEdge(e) })),
  });
  const restore = (step: Step) => {
    setNodes(step.nodes.map((n) => ({ id: n.id, type: 'omu', position: n.position, data: { spec: n.spec, inputs: n.inputs } })));
    setEdges(step.edges.map((e) => ({ id: e.id, source: e.source, target: e.target, sourceHandle: e.sourceHandle, targetHandle: e.targetHandle })));
  };
  // each change of the graph is a step, a moment after the last one (typing in a field is one step, not one per letter)
  useEffect(() => {
    const timer = window.setTimeout(() => history.current.record(stepNow()), 300);
    return () => window.clearTimeout(timer);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [nodes, edges]);
  const undo = () => {
    const step = history.current.undo();
    if (step) restore(step);
  };
  const redo = () => {
    const step = history.current.redo();
    if (step) restore(step);
  };
  const copy = (): boolean => {
    const ids = new Set(nodes.filter((n) => n.selected).map((n) => n.id));
    const clip = copyOf(stepNow().nodes, stepNow().edges, ids);
    if (!clip) return false;
    clipboard.current = clip;
    // other programs can read it too (best effort: the page may not be allowed)
    void navigator.clipboard?.writeText(JSON.stringify({ omu: 1, ...clip })).catch(() => undefined);
    return true;
  };
  const paste = (at?: { x: number; y: number }) => {
    const clip = clipboard.current;
    if (!clip) return;
    const p = pointer.current;
    const canvas = document.querySelector('.canvas')?.getBoundingClientRect();
    const here = at ?? (p && canvas && p.x >= canvas.left && p.x <= canvas.right && p.y >= canvas.top && p.y <= canvas.bottom ? flow.screenToFlowPosition(p) : null);
    const made = pasteOf(clip, newId, here, 2 * GRID, (q) => ({ x: snapToGrid(q.x), y: snapToGrid(q.y) }));
    setNodes((list) => [...list.map((n) => ({ ...n, selected: false })), ...made.nodes.map((n): OmuNode => ({ id: n.id, type: 'omu', position: n.position, selected: true, data: { spec: n.spec, inputs: n.inputs } }))]);
    setEdges((list) => [...list, ...made.edges.map((e) => ({ id: `${e.source}.${e.sourceHandle}->${e.target}.${e.targetHandle}`, ...e }))]);
  };
  const duplicate = () => {
    if (copy()) paste();
  };
  const selectAll = () => setNodes((list) => list.map((n) => ({ ...n, selected: true })));
  const toggleDock = (panel: NonNullable<DockState['bottom']>) => setDock({ ...dock, bottom: dock.bottom === panel ? null : panel });

  // what each command does (the keys, and the search bar, call these)
  const run = (id: CommandId) => {
    switch (id) {
      case 'edit.undo':
        return undo();
      case 'edit.redo':
        return redo();
      case 'edit.copy':
        return void copy();
      case 'edit.paste':
        return paste();
      case 'edit.duplicate':
        return duplicate();
      case 'edit.selectAll':
        return selectAll();
      case 'search.blocks':
        return openPalette('all', undefined, '@');
      case 'palette.open':
      case 'palette.openAlt':
        return openPalette('all');
      case 'palette.addBlock':
        return openPalette('block');
      case 'dock.problems':
        return toggleDock('problems');
      case 'dock.logs':
        return toggleDock('logs');
      case 'dock.monitor':
        return toggleDock('monitor');
      case 'dock.inspector':
        return setDock({ ...dock, right: dock.right ? null : 'inspector' });
    }
  };

  // the keyboard: the commands that edit the graph only act when no text is being typed (the field keeps its own undo and copy)
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const el = e.target as HTMLElement | null;
      const typing = !!el && (el.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(el.tagName));
      const id = commandOf(e, keys);
      if (!id) return;
      const alwaysOn = id.startsWith('palette.open') || id === 'search.blocks' || id.startsWith('dock.');
      if (typing && !alwaysOn) return;
      if (!alwaysOn && tab !== 'nodes') return;
      e.preventDefault();
      run(id);
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  });

  // opens the documentation window of an object (or brings it to the front when it is open), near a point of the screen
  const openDoc = useCallback((doc: DocRef, anchor: { x: number; y: number }) => {
    const key = docKey(doc);
    setHelps((list) => {
      const open = list.find((h) => h.key === key);
      const at = { x: Math.max(4, Math.min(window.innerWidth - 500, anchor.x)), y: Math.max(4, Math.min(window.innerHeight - 220, anchor.y)) };
      return [...list.filter((h) => h.key !== key), open ?? { key, doc, ...at }];
    });
  }, []);

  const context: EditorContext = useMemo(
    () => ({
      apps,
      devices,
      library,
      openModules: () => setTab('modules'),
      openSoundboards: (first?: string) => {
        setBoardsFirst(first);
        setTab('soundboards');
      },
      statusOf: (id) => status?.nodes.find((n) => n.id === id),
      openHelp: (id: string, anchor: { x: number; y: number }) => openDoc({ kind: 'node', node: id }, anchor),
      openCalibration: (id: string, anchor: { x: number; y: number }) =>
        setCalibrations((list) => [...list.filter((c) => c.node !== id), list.find((c) => c.node === id) ?? { node: id, x: Math.max(4, Math.min(window.innerWidth - 420, anchor.x)), y: Math.max(4, Math.min(window.innerHeight - 320, anchor.y)) }]),
      removeNode: (id: string) => void flow.deleteElements({ nodes: [{ id }] }),
      // the values typed in a block go with it, but only for the inputs it still has
      update: (id: string, kind: NodeKind) => setNodes((list) => list.map((n) => (n.id === id ? { ...n, data: { ...n.data, spec: prunedInputs({ id, ...named(n.data.spec), ...kind } as NodeSpec) } } : n))),
      setInput: (id: string, port: string, value: InputValue) =>
        setNodes((list) => list.map((n) => (n.id === id ? { ...n, data: { ...n.data, spec: { ...n.data.spec, inputs: { ...n.data.spec.inputs, [port]: value } } as NodeSpec } } : n))),
      setInputs: (id: string, count: number) => {
        setNodes((list) => list.map((n) => (n.id === id ? { ...n, data: { ...n.data, inputs: Math.max(1, Math.min(MAX_ENTRIES, count)) } } : n)));
      },
      setDisplay: (id: string, text: string) =>
        setNodes((list) =>
          list.map((n) => {
            if (n.id !== id) return n;
            const { display: _old, ...rest } = n.data.spec;
            // free text; none at all means no name (the property is left out)
            return { ...n, data: { ...n.data, spec: (text === '' ? rest : { ...rest, display: text }) as NodeSpec } };
          }),
        ),
    }),
    [apps, devices, library, status, flow, openDoc, setNodes, setEdges],
  );

  // the window of a block that is gone goes with it
  useEffect(() => {
    const gone = (h: { doc: DocRef }) => h.doc.kind === 'node' && !nodes.some((n) => n.id === (h.doc as { node: string }).node);
    setHelps((list) => (list.some(gone) ? list.filter((h) => !gone(h)) : list));
    setCalibrations((list) => (list.every((c) => nodes.some((n) => n.id === c.node)) ? list : list.filter((c) => nodes.some((n) => n.id === c.node))));
  }, [nodes]);

  // what the search bar offers: the commands, the settings (declared in `settingsRegistry.ts`) and every kind of block
  const paletteItems: PaletteItem[] = palette
    ? [
        ...TABS.map((x): PaletteItem => ({ id: `go-${x.id}`, kind: 'command', label: t('palette.goTo', { name: t(x.label) }), icon: 'arrow', run: () => setTab(x.id) })),
        ...(dirty ? [{ id: 'apply', kind: 'command' as const, label: t('app.apply'), icon: 'check' as const, run: () => void apply() }, { id: 'revert', kind: 'command' as const, label: t('app.revert'), icon: 'undo' as const, run: revert }] : []),
        { id: 'tidy', kind: 'command', label: t('app.tidy'), icon: 'grid', run: autoLayout },
        { id: 'json', kind: 'command', label: t('app.json'), icon: 'braces', run: () => setShowJson((v) => !v) },
        { id: 'undo', kind: 'command', label: t('keys.undo'), icon: 'undo', shortcut: keys['edit.undo'], run: undo },
        { id: 'redo', kind: 'command', label: t('keys.redo'), icon: 'arrow', shortcut: keys['edit.redo'], run: redo },
        { id: 'copy', kind: 'command', label: t('keys.copy'), icon: 'list', shortcut: keys['edit.copy'], run: () => void copy() },
        { id: 'paste', kind: 'command', label: t('keys.paste'), icon: 'list', shortcut: keys['edit.paste'], run: () => paste() },
        { id: 'duplicate', kind: 'command', label: t('keys.duplicate'), icon: 'list', shortcut: keys['edit.duplicate'], run: duplicate },
        { id: 'selectAll', kind: 'command', label: t('keys.selectAll'), icon: 'grid', shortcut: keys['edit.selectAll'], run: selectAll },
        ...nodes.map((n): PaletteItem => ({ id: `node-${n.id}`, kind: 'node', label: nameOf(n.data.spec) || t(`kind.${n.data.spec.type}`), family: categoryOf(n.data.spec.type), keywords: `${t(`kind.${n.data.spec.type}`)} ${n.id}`, run: () => goTo({ severity: 'info', code: 'go', message: '', node: n.id }) })),
        { id: 'problems', kind: 'command', label: t('dock.problems'), icon: 'error', run: () => setDock({ ...dock, bottom: dock.bottom === 'problems' ? null : 'problems' }) },
        { id: 'logs', kind: 'command', label: t('app.logs'), icon: 'list', run: () => setDock({ ...dock, bottom: dock.bottom === 'logs' ? null : 'logs' }) },
        { id: 'versions', kind: 'command', label: t('dock.versions'), icon: 'undo', run: () => toggleDock('versions') },
        { id: 'monitor', kind: 'command', label: t('dock.monitor'), icon: 'panel', run: () => setDock({ ...dock, bottom: dock.bottom === 'monitor' ? null : 'monitor' }) },
        { id: 'inspector', kind: 'command', label: t('dock.inspector'), icon: 'panel', run: () => setDock({ ...dock, right: dock.right ? null : 'inspector' }) },
        status?.running
          ? { id: 'engine', kind: 'command', label: t('app.stopEngine'), icon: 'stop', run: () => void stopEngine() }
          : { id: 'engine', kind: 'command', label: t('app.startEngine'), icon: 'play', run: () => void startEngine() },
        ...SETTING_DECLS.map((d): PaletteItem => ({
          id: `setting-${d.id}`,
          kind: 'setting',
          icon: 'gear',
          label: t(d.label),
          keywords: d.hint ? t(d.hint) : undefined,
          run: () => {
            setTab('settings');
            setSettingFocus({ id: d.id, nonce: Date.now() });
          },
        })),
        ...NODE_TYPES.map((type): PaletteItem => ({
          id: `block-${type}`,
          kind: 'block',
          label: t(`kind.${type}`),
          family: categoryOf(type),
          // the words of its explanation find it too, without the marks of the links
          keywords: t(`kind.${type}.hint`).replace(/\[([^\]]+)\]\([^)]*\)/g, '$1'),
          run: () => {
            setTab('nodes');
            addNode(categoryOf(type), defaultKind(type), palette.at);
          },
        })),
      ]
    : [];

  // the daemon's opinion of the graph (its warnings too), a moment after the last change
  const graphKey = canonical(graph);
  useEffect(() => {
    if (conn !== 'open') return;
    const timer = window.setTimeout(() => {
      daemon
        .graphValidate(graph)
        .then((r) => setDiagnostics(r.diagnostics ?? []))
        .catch(() => undefined);
    }, 400);
    return () => window.clearTimeout(timer);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [graphKey, conn, daemon]);
  // the changes not applied yet are kept a moment after each edit, so that a closed window or a crash loses nothing
  useEffect(() => {
    if (conn !== 'open' || !loaded || draftOffer) return;
    const timer = window.setTimeout(() => saveDraft(dirty ? { at: new Date().toISOString(), graph, layout: currentLayout() } : null), 1000);
    return () => window.clearTimeout(timer);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [graphKey, dirty, conn, draftOffer, loaded]);
  // brings a version of the graph back into the editor (it is not applied until the person says so)
  const bringBack = (v: Version) => {
    const f = toFlow(v.graph, v.layout);
    setNodes(f.nodes.map((n) => ({ id: n.id, type: 'omu', position: n.position, data: { spec: n.spec, inputs: n.inputs } })));
    setEdges(f.edges.map((e) => ({ ...e })));
  };
  const errors = diagnostics.filter((d) => d.severity === 'error').length;
  const warnings = diagnostics.filter((d) => d.severity === 'warning').length;
  const selected = nodes.find((n) => n.selected)?.data.spec;
  // a problem leads to the block it is about
  const goTo = (d: Diagnostic) => {
    if (!d.node) return;
    const id = d.node;
    setTab('nodes');
    setNodes((list) => list.map((n) => ({ ...n, selected: n.id === id })));
    void flow.fitView({ nodes: [{ id }], duration: 300, maxZoom: 1, padding: 0.8 });
    if (d.port) window.setTimeout(() => flashPort(id, d.port as string), 380);
  };
  const clampSize = (n: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, n));

  // each edge is drawn in the colour of the flow it carries
  const plainNodes = useMemo(() => nodes.map((n) => ({ id: n.id, spec: n.data.spec })), [nodes]);
  const shownEdges = useMemo(
    () => edges.map((e) => ({ ...e, className: `flow-${flowOfEdge(plainNodes, { source: e.source, sourceHandle: e.sourceHandle }) ?? 'audio'}` })),
    [edges, plainNodes],
  );

  const noProvider = graph.nodes.some((n) => n.type === 'virtual_mic' || n.type === 'virtual_speaker') && info !== null && !(info.virtual_microphones?.available ?? false);
  const connLabel = conn === 'open' ? 'app.connected' : conn === 'connecting' ? 'app.connecting' : 'app.disconnected';

  return (
    <Editor.Provider value={context}>
      <div className="app" style={{ '--grid': `${GRID}px`, '--gap': `${GAP}px`, '--node-width': `${NODE_WIDTH}px` } as CSSProperties}>
        <header className="topbar">
          <strong>{PRODUCT_NAME}</strong>
          <nav className="tabs">
            {TABS.map((x) => (
              <button key={x.id} className={`${tab === x.id ? 'on' : ''}${x.id === 'nodes' && dirty ? ' dirty' : ''}`} aria-current={tab === x.id ? 'page' : undefined} onClick={() => setTab(x.id)}>
                {t(x.label)}
              </button>
            ))}
          </nav>
          <span className={`pill ${conn}`}>{t(connLabel)}</span>
          {info && (
            <span className="muted" title={`protocol ${info.protocol_version}`}>
              {info.backend} · v{info.version}
            </span>
          )}
          {status && <span className={`pill ${status.running ? 'open' : 'closed'}`}>{t(status.running ? 'app.engineRunning' : 'app.engineStopped')}</span>}
          <span className="spacer" />
          <button className="search-button" onClick={() => openPalette('all')} title={t('palette.title')}>
            {t('palette.open')} <kbd>F1</kbd>
          </button>
          {status?.running ? (
            <button onClick={stopEngine} disabled={conn !== 'open'}>
              {t('app.stopEngine')}
            </button>
          ) : (
            <button onClick={startEngine} disabled={conn !== 'open' || applied.nodes.length === 0} title={t('app.startEngineHint')}>
              {t('app.startEngine')}
            </button>
          )}
          {stale?.supported && (
            <button onClick={cleanDevices} disabled={conn !== 'open' || cleaning || stale.stale.length === 0} title={stale.stale.map((s) => s.name).join(', ') || t('app.cleanDevicesHint')}>
              {cleaning ? t('app.cleaning') : t('app.cleanDevices', { count: stale.stale.length })}
            </button>
          )}
          <button className={confirmShutdown ? 'danger' : ''} onClick={shutdownDaemon} disabled={conn !== 'open'} title={t('app.stopDaemonHint')}>
            {confirmShutdown ? t('app.stopDaemonConfirm') : t('app.stopDaemon')}
          </button>
        </header>

        {tab === 'nodes' && (
          <BlocksBar drag={dragIn}>
            <button onClick={autoLayout} disabled={nodes.length === 0}>
              {t('app.tidy')}
            </button>
            <button onClick={() => setShowJson((v) => !v)} className={showJson ? 'on' : ''}>
              {t('app.json')}
            </button>
            <button onClick={revert} disabled={conn !== 'open' || !dirty}>
              {t('app.revert')}
            </button>
            <button className="primary" onClick={apply} disabled={conn !== 'open' || busy || !dirty}>
              {busy ? t('app.applying') : dirty ? t('app.apply') : t('app.applied')}
            </button>
          </BlocksBar>
        )}

        {draftOffer && (
          <div className="banners">
            <div className="banner info">
              {t('draft.found', { date: new Date(draftOffer.at).toLocaleString() })}{' '}
              <Button size="small" variant="primary" onClick={() => { bringBack(draftOffer); setDraftOffer(null); }}>
                {t('draft.restore')}
              </Button>{' '}
              <Button size="small" onClick={() => { saveDraft(null); setDraftOffer(null); }}>
                {t('draft.discard')}
              </Button>
            </div>
          </div>
        )}
        {(problems.length > 0 || notice || noProvider || (IN_DESKTOP_WINDOW && conn === 'closed')) && (
          <div className="banners">
            {problems.length > 0 && (
              <div className="banner error">
                <strong>{t('app.refused')}</strong>
                <ul>
                  {problems.map((p) => namedProblem(p, graph.nodes, t('node.unnamed'))).map((p) => (
                    <li key={p}>{p}</li>
                  ))}
                </ul>
                <button className="plain" aria-label={t('app.close')} onClick={() => setProblems([])}>
                  ×
                </button>
              </div>
            )}
            {IN_DESKTOP_WINDOW && conn === 'closed' && (
              <div className="banner warn">
                {t('app.daemonDown')}{' '}
                <button
                  className="inline"
                  onClick={() => {
                    startDaemon().catch((e) => setNotice(message(e)));
                  }}
                >
                  {t('app.startDaemon')}
                </button>
              </div>
            )}
            {notice && (
              <div className="banner warn">
                {notice.startsWith('app.') ? t(notice) : notice}
                <button className="plain" aria-label={t('app.close')} onClick={() => setNotice(null)}>
                  ×
                </button>
              </div>
            )}
            {noProvider && <div className="banner info">{t('app.noProvider', { error: info?.virtual_microphones?.error ? ` (${info.virtual_microphones.error})` : '' })}</div>}
          </div>
        )}

        <div className="workbench">
          <div className="work-main">
        <div className="canvas" onPointerMove={(e) => (pointer.current = { x: e.clientX, y: e.clientY })}>
          <ReactFlow
            nodes={nodes}
            edges={shownEdges}
            nodeTypes={nodeTypes}
            onNodesChange={onNodesChange}
            onEdgesChange={onEdgesChange}
            onConnect={onConnect}
            isValidConnection={isValidConnection}
            onNodeDragStop={() => saveLayout()}
            onPaneContextMenu={(e) => {
              e.preventDefault();
              const at = flow.screenToFlowPosition({ x: e.clientX, y: e.clientY });
              setMenu({ x: e.clientX, y: e.clientY, items: [{ label: t('palette.addBlockMenu'), run: () => setPalette({ mode: 'block', at }) }, ...(clipboard.current ? [{ label: t('keys.paste'), run: () => paste(at) }] : []), { label: t('keys.selectAll'), run: selectAll }, { label: t('app.tidy'), run: autoLayout }] });
            }}
            onNodeContextMenu={(e, node) => {
              e.preventDefault();
              setMenu({
                x: e.clientX,
                y: e.clientY,
                items: [
                  { label: t('block.help'), run: () => openDoc({ kind: 'node', node: node.id }, { x: e.clientX + 12, y: e.clientY }) },
                  { label: t('keys.copy'), run: () => { setNodes((list) => list.map((n) => ({ ...n, selected: n.selected || n.id === node.id }))); window.setTimeout(() => void copy(), 0); } },
                  { label: t('keys.duplicate'), run: () => { setNodes((list) => list.map((n) => ({ ...n, selected: n.id === node.id }))); window.setTimeout(duplicate, 0); } },
                  { label: t('block.remove'), danger: true, run: () => void flow.deleteElements({ nodes: [{ id: node.id }] }) },
                ],
              });
            }}
            deleteKeyCode={['Backspace', 'Delete']}
            snapToGrid
            snapGrid={[GRID, GRID]}
            fitView
            fitViewOptions={{ padding: 0.3, maxZoom: 1 }}
            minZoom={0.2}
            colorMode={resolved}
            proOptions={{ hideAttribution: true }}
          >
            {/* the dots are drawn half a cell off the blocks' origin by default: bring their centres back on the multiples of the cell, where the blocks' sides and the ports are */}
            <Background gap={GRID} size={GAP} offset={GAP / 2 - GRID / 2} color="var(--line)" />
            <Controls showInteractive={false} />
            <MiniMap pannable zoomable nodeColor={(n) => `var(--cat-${categoryOf((n.data as { spec: { type: NodeType } }).spec.type)})`} />
          </ReactFlow>
          {tab === 'nodes' &&
            helps.map((h, order) => {
              const spec = h.doc.kind === 'node' ? nodes.find((n) => n.id === (h.doc as { node: string }).node)?.data.spec : undefined;
              if (h.doc.kind === 'node' && !spec) return null;
              const category = h.doc.kind === 'type' ? undefined : categoryOf(h.doc.kind === 'node' ? spec!.type : h.doc.type);
              return (
                <FloatingWindow
                  key={h.key}
                  className={category ? `cat-${category}` : ''}
                  title={t('help.title')}
                  x={h.x}
                  y={h.y}
                  order={order}
                  onMove={(x, y) => setHelps((list) => list.map((w) => (w.key === h.key ? { ...w, x, y } : w)))}
                  onFocus={() => setHelps((list) => (list[list.length - 1]?.key === h.key ? list : [...list.filter((w) => w.key !== h.key), list.find((w) => w.key === h.key)!]))}
                  onClose={() => setHelps((list) => list.filter((w) => w.key !== h.key))}
                >
                  <Doc doc={h.doc} spec={spec} onOpen={(ref) => openDoc(ref, { x: h.x + 28, y: h.y + 28 })} />
                </FloatingWindow>
              );
            })}
          {tab === 'nodes' &&
            calibrations.map((c, order) => {
              const spec = nodes.find((n) => n.id === c.node)?.data.spec;
              if (!spec) return null;
              const bring = () => setCalibrations((list) => (list[list.length - 1]?.node === c.node ? list : [...list.filter((w) => w.node !== c.node), list.find((w) => w.node === c.node)!]));
              return (
                <FloatingWindow
                  key={c.node}
                  className="cat-processing"
                  title={t('calibration.title')}
                  x={c.x}
                  y={c.y}
                  order={helps.length + order}
                  onMove={(x, y) => setCalibrations((list) => list.map((w) => (w.node === c.node ? { ...w, x, y } : w)))}
                  onFocus={bring}
                  onClose={() => setCalibrations((list) => list.filter((w) => w.node !== c.node))}
                >
                  <Calibration spec={spec} daemon={daemon} running={!!status?.nodes.some((n) => n.id === c.node)} />
                </FloatingWindow>
              );
            })}
          {tab === 'soundboards' && (
            <Soundboards
              key={boardsFirst ?? ''}
              daemon={daemon}
              token={rpc.token}
              library={library}
              setLibrary={setLibrary}
              applied={applied}
              shown={graph}
              initialBoard={boardsFirst}
              onAddBlock={(board) => {
                addNode('input', { type: 'soundboard', board });
                setTab('nodes');
              }}
              notify={setNotice}
              onOpenModules={() => setTab('modules')}
            />
          )}
          {tab === 'modules' && <Modules daemon={daemon} notify={setNotice} />}
          {tab === 'settings' && <SettingsPanel daemon={daemon} notify={setNotice} focus={settingFocus} />}
          {showJson && <JsonWindow graph={graph} onClose={() => setShowJson(false)} />}
          {nodes.length === 0 && conn === 'open' && (
            <div className="empty">
              <p>{t('app.emptyTitle')}</p>
              <p className="muted">{t('app.emptyHint')}</p>
            </div>
          )}
        </div>
            {dock.right === 'inspector' && (
              <>
                <Splitter direction="vertical" onResize={(d) => setDock({ ...dock, rightWidth: clampSize(dock.rightWidth - d, 220, 640) })} />
                <div style={{ width: dock.rightWidth }} className="dock-wrap">
                  <DockFrame side="right" title={t('dock.inspector')} onClose={() => setDock({ ...dock, right: null })}>
                    <InspectorPanel spec={selected} />
                  </DockFrame>
                </div>
              </>
            )}
          </div>
          {dock.bottom && (
            <>
              <Splitter direction="horizontal" onResize={(d) => setDock({ ...dock, bottomHeight: clampSize(dock.bottomHeight - d, 90, 600) })} />
              <div style={{ height: dock.bottomHeight }} className="dock-wrap">
                <DockFrame
                  title={dock.bottom === 'problems' ? `${t('dock.problems')} (${diagnostics.length})` : dock.bottom === 'logs' ? t('app.logs') : dock.bottom === 'versions' ? t('dock.versions') : t('dock.monitor')}
                  onClose={() => setDock({ ...dock, bottom: null })}
                >
                  {dock.bottom === 'problems' && <ProblemsPanel diagnostics={diagnostics} nodes={nodes.map((n) => n.data.spec)} onGo={goTo} />}
                  {dock.bottom === 'logs' && <LogsPanel daemon={daemon} />}
                  {dock.bottom === 'versions' && <VersionsPanel versions={versions} current={graph} onRestore={bringBack} />}
                  {dock.bottom === 'monitor' && <MonitorPanel status={status} nodes={nodes.map((n) => n.data.spec)} />}
                </DockFrame>
              </div>
            </>
          )}
        </div>
        <StatusBar dock={dock} setDock={setDock} errors={errors} warnings={warnings} status={status} connected={conn === 'open'} blocks={nodes.length} />
        {palette && <CommandPalette key={`${palette.mode}${palette.initial ?? ''}`} items={paletteItems} mode={palette.mode} initial={palette.initial} onClose={() => setPalette(null)} />}
        {menu && <ContextMenu x={menu.x} y={menu.y} items={menu.items} onClose={() => setMenu(null)} />}
      </div>
    </Editor.Provider>
  );
}
