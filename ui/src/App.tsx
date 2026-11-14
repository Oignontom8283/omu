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
import {
  KINDS_BY_CATEGORY,
  canConnect,
  canonical,
  categoryOf,
  defaultKind,
  edgeId,
  edgesAfterKindChange,
  flowOfEdge,
  fromFlow,
  idBase,
  positionsOf,
  toFlow,
  uniqueId,
} from './graph.ts';
import { LANGUAGES, languageName, useI18n } from './i18n/index.tsx';
import { useTheme, type ThemeChoice } from './theme.tsx';
import type { AppInfo, Category, DeviceInfo, EngineStatus, GraphSpec, Layout, NodeKind, NodeType, SystemInfo } from './types.ts';
import { PRODUCT_NAME } from './types.ts';

const nodeTypes = { omu: NodeCard };
const ID_PATTERN = /^[A-Za-z0-9._-]+$/;

function message(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

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
  const [busy, setBusy] = useState(false);
  const [showJson, setShowJson] = useState(false);
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

  const saveLayout = useCallback(
    (list = nodesRef.current) => {
      if (layoutTimer.current) clearTimeout(layoutTimer.current);
      layoutTimer.current = setTimeout(() => {
        daemon.layoutSet(positionsOf(list.map((n) => ({ id: n.id, position: n.position, spec: n.data.spec })))).catch(() => undefined);
      }, 500);
    },
    [daemon],
  );

  const show = useCallback(
    (g: GraphSpec, layout: Layout) => {
      const f = toFlow(g, layout);
      setNodes(f.nodes.map((n) => ({ id: n.id, type: 'omu', position: n.position, data: { spec: n.spec } })));
      setEdges(f.edges.map((e) => ({ ...e })));
      setApplied(g);
    },
    [setNodes, setEdges],
  );

  const load = useCallback(async () => {
    try {
      const [i, g, layout, a, d] = await Promise.all([daemon.systemInfo(), daemon.graphGet(), daemon.layoutGet(), daemon.apps(), daemon.devices()]);
      setInfo(i);
      setApps(a);
      setDevices(d);
      show(g, layout);
      setProblems([]);
    } catch (e) {
      setNotice(message(e));
    }
  }, [daemon, show]);

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

  const stopEngine = async () => {
    try {
      await daemon.stop();
    } catch (e) {
      setNotice(message(e));
    }
  };

  const addNode = (category: Category) => {
    const type = KINDS_BY_CATEGORY[category][0];
    const id = uniqueId(idBase(type), nodesRef.current.map((n) => n.id));
    // start at the middle of the view and slide away from the blocks already there
    const p = flow.screenToFlowPosition({ x: window.innerWidth / 2, y: window.innerHeight / 2 });
    const position = { x: p.x - 115, y: p.y - 70 };
    while (nodesRef.current.some((n) => Math.abs(n.position.x - position.x) < 40 && Math.abs(n.position.y - position.y) < 40)) {
      position.x += 40;
      position.y += 40;
    }
    const node: OmuNode = { id, type: 'omu', position, data: { spec: { id, ...defaultKind(type) } } };
    setNodes((list) => [...list, node]);
    saveLayout([...nodesRef.current, node]);
  };

  const autoLayout = () => {
    const f = toFlow(graph, {});
    setNodes((list) => list.map((n) => ({ ...n, position: f.nodes.find((x) => x.id === n.id)?.position ?? n.position })));
    saveLayout(f.nodes.map((n) => ({ id: n.id, type: 'omu', position: n.position, data: { spec: n.spec } })));
    window.setTimeout(() => void flow.fitView({ duration: 300, padding: 0.2 }), 50);
  };

  const onConnect = useCallback(
    (c: FlowConnection) => {
      setEdges((list) => {
        const id = edgeId({ from: { node: c.source, port: c.sourceHandle ?? '' }, to: { node: c.target, port: c.targetHandle ?? '' } });
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
      statusOf: (id) => status?.nodes.find((n) => n.id === id),
      update: (id: string, kind: NodeKind) => setNodes((list) => list.map((n) => (n.id === id ? { ...n, data: { spec: { id, ...kind } } } : n))),
      changeKind: (id: string, type: NodeType) => {
        const plain = nodesRef.current.map((n) => ({ id: n.id, spec: n.data.spec }));
        setEdges(edgesAfterKindChange(plain, edgesRef.current.map(asEdge), id, type));
        setNodes((list) => list.map((n) => (n.id === id ? { ...n, data: { spec: { id, ...defaultKind(type) } } } : n)));
      },
      rename: (id: string, next: string) => {
        if (!ID_PATTERN.test(next)) return 'node.nameChars';
        if (nodesRef.current.some((n) => n.id === next)) return 'node.nameTaken';
        setNodes((list) => list.map((n) => (n.id === id ? { ...n, id: next, data: { spec: { ...n.data.spec, id: next } } } : n)));
        setEdges((list) =>
          list.map((e) => {
            const source = e.source === id ? next : e.source;
            const target = e.target === id ? next : e.target;
            return { ...e, source, target, id: edgeId({ from: { node: source, port: e.sourceHandle ?? '' }, to: { node: target, port: e.targetHandle ?? '' } }) };
          }),
        );
        return null;
      },
    }),
    [apps, devices, status, setNodes, setEdges],
  );

  // each edge is drawn in the colour of the flow it carries
  const plainNodes = useMemo(() => nodes.map((n) => ({ id: n.id, spec: n.data.spec })), [nodes]);
  const shownEdges = useMemo(
    () => edges.map((e) => ({ ...e, className: `flow-${flowOfEdge(plainNodes, { source: e.source, sourceHandle: e.sourceHandle }) ?? 'audio'}` })),
    [edges, plainNodes],
  );

  const noProvider = graph.nodes.some((n) => n.type === 'virtual_mic') && info !== null && !(info.virtual_microphones?.available ?? false);
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
          <button onClick={autoLayout} disabled={nodes.length === 0}>
            {t('app.tidy')}
          </button>
          <button onClick={() => setShowJson((v) => !v)} className={showJson ? 'on' : ''}>
            {t('app.json')}
          </button>
          <button onClick={stopEngine} disabled={conn !== 'open' || !status?.running}>
            {t('app.stopEngine')}
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
                  {problems.map((p) => (
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
