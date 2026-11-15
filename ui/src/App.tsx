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
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { EVENT_GRAPH, EVENT_STATUS, IN_DESKTOP_WINDOW, Rpc, RpcError, api, findToken, startDaemon, type Connection } from './api.ts';
import { Editor, type EditorContext } from './context.ts';
import { NodeCard, type OmuNode } from './components/NodeCard.tsx';
import { Palette } from './components/Palette.tsx';
import { Modules } from './components/Modules.tsx';
import { SettingsPanel } from './components/SettingsPanel.tsx';
import { Soundboards } from './components/Soundboards.tsx';
import {
  KINDS_BY_CATEGORY,
  canConnect,
  canonical,
  categoryOf,
  defaultKind,
  baseHandle,
  edgeId,
  edgesAfterKindChange,
  flowOfEdge,
  fromFlow,
  newId,
  namedProblem,
  layoutOf,
  MAX_ENTRIES,
  toFlow,
} from './graph.ts';
import { LANGUAGES, languageName, useI18n } from './i18n/index.tsx';
import { useTheme, type ThemeChoice } from './theme.tsx';
import type { AppInfo, Category, DeviceInfo, EngineStatus, GraphSpec, Layout, NodeKind, NodeSpec, NodeType, SoundLibrary, StaleDevices, SystemInfo } from './types.ts';
import { PRODUCT_NAME } from './types.ts';

const nodeTypes = { omu: NodeCard };

function message(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

/** The display name of a block, to be kept when its settings change. */
const named = (spec: NodeSpec): { display?: string } => (spec.display === undefined ? {} : { display: spec.display });

const asEdge = (e: Edge) => ({ id: e.id, source: e.source, target: e.target, sourceHandle: e.sourceHandle ?? '', targetHandle: e.targetHandle ?? '' });

export function App() {
  const rpc = useMemo(() => new Rpc(findToken()), []);
  const daemon = useMemo(() => api(rpc), [rpc]);
  const flow = useReactFlow();
  const { t, language, setLanguage } = useI18n();
  const { choice, setChoice, resolved } = useTheme();

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
  // the soundboards the daemon keeps, and the panel that edits them (`first`: the board it opens on)
  const [library, setLibrary] = useState<SoundLibrary>({ sounds: [], boards: [] });
  const [boardsPanel, setBoardsPanel] = useState<{ first?: string } | null>(null);
  // the panel of the optional modules (FFmpeg); it and the soundboards panel share the right side, one at a time
  const [modulesPanel, setModulesPanel] = useState(false);
  const [settingsPanel, setSettingsPanel] = useState(false);
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
    try {
      const [i, g, layout, a, d, lib] = await Promise.all([daemon.systemInfo(), daemon.graphGet(), daemon.layoutGet(), daemon.apps(), daemon.devices(), daemon.soundboards()]);
      setInfo(i);
      setLibrary(lib);
      setApps(a);
      setDevices(d);
      show(g, layout);
      setProblems([]);
      refreshStale();
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

  const apply = async () => {
    setBusy(true);
    setNotice(null);
    try {
      await daemon.graphSet(graph);
      setApplied(graph);
      setProblems([]);
      // a renamed or removed device leaves an old one behind in Windows: the button shows how many
      refreshStale();
    } catch (e) {
      setProblems(e instanceof RpcError && e.problems.length ? e.problems : [message(e)]);
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

  const addNode = (category: Category, kind?: NodeKind) => {
    const type = KINDS_BY_CATEGORY[category][0];
    // a random identifier: the name people give a block is a separate thing (its display name)
    const id = newId();
    // start at the middle of the view and slide away from the blocks already there
    const p = flow.screenToFlowPosition({ x: window.innerWidth / 2, y: window.innerHeight / 2 });
    const position = { x: p.x - 115, y: p.y - 70 };
    while (nodesRef.current.some((n) => Math.abs(n.position.x - position.x) < 40 && Math.abs(n.position.y - position.y) < 40)) {
      position.x += 40;
      position.y += 40;
    }
    const node: OmuNode = { id, type: 'omu', position, data: { spec: { id, ...(kind ?? defaultKind(type)) } } };
    setNodes((list) => [...list, node]);
    saveLayout([...nodesRef.current, node]);
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

  const context: EditorContext = useMemo(
    () => ({
      apps,
      devices,
      library,
      openModules: () => {
        setBoardsPanel(null);
        setSettingsPanel(false);
        setModulesPanel(true);
      },
      openSoundboards: (first?: string) => {
        setModulesPanel(false);
        setSettingsPanel(false);
        setBoardsPanel({ first });
      },
      statusOf: (id) => status?.nodes.find((n) => n.id === id),
      update: (id: string, kind: NodeKind) => setNodes((list) => list.map((n) => (n.id === id ? { ...n, data: { ...n.data, spec: { id, ...named(n.data.spec), ...kind } } } : n))),
      changeKind: (id: string, type: NodeType) => {
        const plain = nodesRef.current.map((n) => ({ id: n.id, spec: n.data.spec }));
        setEdges(edgesAfterKindChange(plain, edgesRef.current.map(asEdge), id, type));
        // a soundboard block starts on the first soundboard there is
        const fresh = type === 'soundboard' ? { type, board: library.boards[0]?.id ?? '' } : defaultKind(type);
        setNodes((list) => list.map((n) => (n.id === id ? { ...n, data: { spec: { id, ...named(n.data.spec), ...fresh } } } : n)));
      },
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
    [apps, devices, library, status, setNodes, setEdges],
  );

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
      <div className="app">
        <header className="topbar">
          <strong>{PRODUCT_NAME}</strong>
          <span className={`pill ${conn}`}>{t(connLabel)}</span>
          {info && (
            <span className="muted" title={`protocol ${info.protocol_version}`}>
              {info.backend} · v{info.version}
            </span>
          )}
          {status && <span className={`pill ${status.running ? 'open' : 'closed'}`}>{t(status.running ? 'app.engineRunning' : 'app.engineStopped')}</span>}
          <span className="spacer" />
          <select aria-label={t('app.language')} value={language} onChange={(e) => setLanguage(e.target.value)}>
            {LANGUAGES.map((code) => (
              <option key={code} value={code}>
                {languageName(code)}
              </option>
            ))}
          </select>
          <select aria-label={t('app.theme')} value={choice} onChange={(e) => setChoice(e.target.value as ThemeChoice)}>
            <option value="system">{t('app.themeSystem')}</option>
            <option value="light">{t('app.themeLight')}</option>
            <option value="dark">{t('app.themeDark')}</option>
          </select>
          <button
            onClick={() => {
              setModulesPanel(false);
              setSettingsPanel(false);
              setBoardsPanel((p) => (p ? null : {}));
            }}
            className={boardsPanel ? 'on' : ''}
          >
            {t('soundboards.title')}
          </button>
          <button
            onClick={() => {
              setBoardsPanel(null);
              setSettingsPanel(false);
              setModulesPanel((v) => !v);
            }}
            className={modulesPanel ? 'on' : ''}
          >
            {t('modules.title')}
          </button>
          <button
            onClick={() => {
              setBoardsPanel(null);
              setModulesPanel(false);
              setSettingsPanel((v) => !v);
            }}
            className={settingsPanel ? 'on' : ''}
          >
            {t('settings.title')}
          </button>
          <button onClick={autoLayout} disabled={nodes.length === 0}>
            {t('app.tidy')}
          </button>
          <button onClick={() => setShowJson((v) => !v)} className={showJson ? 'on' : ''}>
            {t('app.json')}
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
          <button onClick={revert} disabled={conn !== 'open' || !dirty}>
            {t('app.revert')}
          </button>
          <button className="primary" onClick={apply} disabled={conn !== 'open' || busy || !dirty}>
            {busy ? t('app.applying') : dirty ? t('app.apply') : t('app.applied')}
          </button>
        </header>

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

        <div className="canvas">
          <Palette onAdd={addNode} />
          <ReactFlow
            nodes={nodes}
            edges={shownEdges}
            nodeTypes={nodeTypes}
            onNodesChange={onNodesChange}
            onEdgesChange={onEdgesChange}
            onConnect={onConnect}
            isValidConnection={isValidConnection}
            onNodeDragStop={() => saveLayout()}
            deleteKeyCode={['Backspace', 'Delete']}
            fitView
            fitViewOptions={{ padding: 0.3, maxZoom: 1 }}
            minZoom={0.2}
            colorMode={resolved}
            proOptions={{ hideAttribution: true }}
          >
            <Background gap={24} />
            <Controls showInteractive={false} />
            <MiniMap pannable zoomable nodeColor={(n) => `var(--cat-${categoryOf((n.data as { spec: { type: NodeType } }).spec.type)})`} />
          </ReactFlow>
          {boardsPanel && (
            <Soundboards
              key={boardsPanel.first ?? ''}
              daemon={daemon}
              token={rpc.token}
              library={library}
              setLibrary={setLibrary}
              applied={applied}
              shown={graph}
              initialBoard={boardsPanel.first}
              onClose={() => setBoardsPanel(null)}
              onAddBlock={(board) => addNode('input', { type: 'soundboard', board })}
              notify={setNotice}
              onOpenModules={() => {
                setBoardsPanel(null);
                setSettingsPanel(false);
                setModulesPanel(true);
              }}
            />
          )}
          {modulesPanel && <Modules daemon={daemon} onClose={() => setModulesPanel(false)} notify={setNotice} />}
          {settingsPanel && <SettingsPanel daemon={daemon} onClose={() => setSettingsPanel(false)} notify={setNotice} />}
          {showJson && <textarea className="json" readOnly value={JSON.stringify(graph, null, 2)} />}
          {nodes.length === 0 && conn === 'open' && (
            <div className="empty">
              <p>{t('app.emptyTitle')}</p>
              <p className="muted">{t('app.emptyHint')}</p>
            </div>
          )}
        </div>
      </div>
    </Editor.Provider>
  );
}
