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
import { EVENT_GRAPH, EVENT_STATUS, Rpc, RpcError, api, findToken, type Connection } from './api.ts';
import { Editor, type EditorContext } from './context.ts';
import { NodeCard, type OmuNode } from './components/NodeCard.tsx';
import { Palette } from './components/Palette.tsx';
import { KINDS, canConnect, canonical, defaultKind, edgeId, fromFlow, idBase, positionsOf, toFlow, uniqueId } from './graph.ts';
import type { AppInfo, DeviceInfo, EngineStatus, GraphSpec, Layout, NodeKind, NodeType, SystemInfo } from './types.ts';
import { PRODUCT_NAME } from './types.ts';

const nodeTypes = { omu: NodeCard };
const ID_PATTERN = /^[A-Za-z0-9._-]+$/;

function message(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

export function App() {
  const rpc = useMemo(() => new Rpc(findToken()), []);
  const daemon = useMemo(() => api(rpc), [rpc]);
  const flow = useReactFlow();

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

  const graph = useMemo(
    () => fromFlow(nodes.map((n) => ({ spec: n.data.spec })), edges.map((e) => ({ id: e.id, source: e.source, target: e.target, sourceHandle: e.sourceHandle ?? 'out', targetHandle: e.targetHandle ?? 'in' }))),
    [nodes, edges],
  );
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
        if (dirtyRef.current) setNotice('The graph was changed elsewhere. Apply to overwrite it, or Revert to load it.');
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
    const t = setInterval(() => {
      daemon.apps().then(setApps).catch(() => undefined);
      daemon.devices().then(setDevices).catch(() => undefined);
    }, 4000);
    return () => clearInterval(t);
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

  const addNode = (type: NodeType) => {
    const id = uniqueId(idBase(type), nodesRef.current.map((n) => n.id));
    // start at the middle of the view and slide away from the nodes already there
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
        const id = edgeId({ from: { node: c.source, port: c.sourceHandle ?? 'out' }, to: { node: c.target, port: c.targetHandle ?? 'in' } });
        return addEdge({ ...c, id }, list);
      });
    },
    [setEdges],
  );

  const isValidConnection = useCallback(
    (c: { source: string; target: string; sourceHandle?: string | null; targetHandle?: string | null }) =>
      canConnect(
        nodesRef.current.map((n) => ({ id: n.id, spec: n.data.spec })),
        edgesRef.current.map((e) => ({ id: e.id, source: e.source, target: e.target, sourceHandle: e.sourceHandle ?? '', targetHandle: e.targetHandle ?? '' })),
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
      rename: (id: string, next: string) => {
        if (!ID_PATTERN.test(next)) return 'Use letters, digits, - _ and . only';
        if (nodesRef.current.some((n) => n.id === next)) return 'This name is already used';
        setNodes((list) => list.map((n) => (n.id === id ? { ...n, id: next, data: { spec: { ...n.data.spec, id: next } } } : n)));
        setEdges((list) =>
          list.map((e) => {
            const source = e.source === id ? next : e.source;
            const target = e.target === id ? next : e.target;
            return { ...e, source, target, id: edgeId({ from: { node: source, port: e.sourceHandle ?? 'out' }, to: { node: target, port: e.targetHandle ?? 'in' } }) };
          }),
        );
        return null;
      },
    }),
    [apps, devices, status, setNodes, setEdges],
  );

  const hasVirtualMic = graph.nodes.some((n) => n.type === 'virtual_mic');
  const noProvider = hasVirtualMic && info !== null && !(info.virtual_microphones?.available ?? false);

  return (
    <Editor.Provider value={context}>
      <div className="app">
        <header className="topbar">
          <strong className="brand">{PRODUCT_NAME}</strong>
          <span className={`pill ${conn}`}>{conn === 'open' ? 'connected' : conn === 'connecting' ? 'connecting…' : 'disconnected'}</span>
          {info && (
            <span className="muted" title={`protocol ${info.protocol_version}`}>
              {info.backend} · v{info.version}
            </span>
          )}
          {status && <span className={`pill ${status.running ? 'open' : 'closed'}`}>{status.running ? 'engine running' : 'engine stopped'}</span>}
          <span className="spacer" />
          <button onClick={autoLayout} disabled={nodes.length === 0}>Tidy up</button>
          <button onClick={() => setShowJson((v) => !v)} className={showJson ? 'on' : ''}>JSON</button>
          <button onClick={stopEngine} disabled={conn !== 'open' || !status?.running}>Stop engine</button>
          <button onClick={revert} disabled={conn !== 'open' || !dirty}>Revert</button>
          <button className="primary" onClick={apply} disabled={conn !== 'open' || busy || !dirty}>
            {busy ? 'Applying…' : dirty ? 'Apply' : 'Applied'}
          </button>
        </header>

        {(problems.length > 0 || notice || noProvider) && (
          <div className="banners">
            {problems.length > 0 && (
              <div className="banner error">
                <strong>The graph was refused:</strong>
                <ul>{problems.map((p) => <li key={p}>{p}</li>)}</ul>
                <button onClick={() => setProblems([])}>×</button>
              </div>
            )}
            {notice && (
              <div className="banner warn">
                {notice}
                <button onClick={() => setNotice(null)}>×</button>
              </div>
            )}
            {noProvider && (
              <div className="banner info">
                This system has no virtual microphone yet: a “{KINDS.virtual_mic.label}” node stays silent
                {info?.virtual_microphones?.error ? ` (${info.virtual_microphones.error})` : ''}. Send to a playback device instead, or install a virtual audio cable.
              </div>
            )}
          </div>
        )}

        <div className="canvas">
          <Palette onAdd={addNode} />
          <ReactFlow
            nodes={nodes}
            edges={edges}
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
            colorMode="dark"
            proOptions={{ hideAttribution: true }}
          >
            <Background gap={24} />
            <Controls showInteractive={false} />
            <MiniMap pannable zoomable nodeColor={(n) => KINDS[(n.data as { spec: { type: NodeType } }).spec.type].color} />
          </ReactFlow>
          {showJson && <textarea className="json" readOnly value={JSON.stringify(graph, null, 2)} />}
          {nodes.length === 0 && conn === 'open' && (
            <div className="empty">
              <p>No node yet.</p>
              <p className="muted">Add an <b>Application</b> and a <b>Virtual mic</b> from the left, link them, then press <b>Apply</b>.</p>
            </div>
          )}
        </div>
      </div>
    </Editor.Provider>
  );
}
