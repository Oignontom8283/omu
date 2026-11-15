// The control protocol, from the browser: JSON-RPC 2.0 over one WebSocket (`/ws`), with automatic reconnection.
// The same endpoint serves a daemon on this machine or on another one: the page talks to whoever served it.

import { DEV_TOKEN } from './types.ts';
import type { AppInfo, CleanOutcome, DeviceInfo, EngineStatus, GraphSpec, Layout, StaleDevices, SystemInfo, ValidateResult } from './types.ts';

export type Connection = 'connecting' | 'open' | 'closed';

/** True when the panel runs inside the desktop window (Tauri) instead of a browser. */
export const IN_DESKTOP_WINDOW = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

type Invoke = (command: string, args?: Record<string, unknown>) => Promise<unknown>;

export class RpcError extends Error {
  code: number;
  data: unknown;
  constructor(code: number, message: string, data?: unknown) {
    super(message);
    this.code = code;
    this.data = data;
  }
  /** The list of problems the daemon attached to a rejected graph. */
  get problems(): string[] {
    return Array.isArray(this.data) ? this.data.map(String) : [];
  }
}

/** The access token: given in the address of the page (`#token=...`), then kept for the tab's lifetime. */
export function findToken(): string {
  const fromHash = new URLSearchParams(window.location.hash.replace(/^#/, '')).get('token');
  if (fromHash) {
    try {
      sessionStorage.setItem('omu-token', fromHash);
    } catch {
      /* storage may be blocked: the token then lives in memory only */
    }
    // keep the secret out of the address bar, history and screenshots
    window.history.replaceState(null, '', window.location.pathname + window.location.search);
    return fromHash;
  }
  try {
    return sessionStorage.getItem('omu-token') ?? DEV_TOKEN;
  } catch {
    return DEV_TOKEN;
  }
}

type Pending = { resolve: (v: unknown) => void; reject: (e: Error) => void };
type EventHandler = (method: string, params: unknown) => void;

export class Rpc {
  private socket: WebSocket | null = null;
  private nextId = 1;
  private pending = new Map<number, Pending>();
  private events = new Set<EventHandler>();
  private states = new Set<(c: Connection) => void>();
  private retry: ReturnType<typeof setTimeout> | null = null;
  private stopped = false;
  state: Connection = 'connecting';

  constructor(private token: string) {}

  // Desktop window: the app's own commands carry the requests (see src/gui); nothing is pushed to the page, so
  // the status and graph events are made by polling.
  private invoke: Invoke | null = null;
  private poll: ReturnType<typeof setInterval> | null = null;
  private lastGraph = '';

  start(): void {
    this.stopped = false;
    if (IN_DESKTOP_WINDOW) void this.startDesktop();
    else this.connect();
  }

  stop(): void {
    this.stopped = true;
    if (this.retry) clearTimeout(this.retry);
    if (this.poll) clearInterval(this.poll);
    this.socket?.close();
  }

  private async startDesktop(): Promise<void> {
    const { invoke } = await import('@tauri-apps/api/core');
    this.invoke = invoke as Invoke;
    let tick = 0;
    const once = async () => {
      try {
        const status = await this.request<unknown>('engine.status');
        if (this.state !== 'open') this.set('open');
        this.events.forEach((h) => h('engine.status', status));
        if (tick++ % 8 === 0) {
          const graph = JSON.stringify(await this.request<unknown>('graph.get'));
          if (this.lastGraph && graph !== this.lastGraph) this.events.forEach((h) => h('graph.changed', JSON.parse(graph)));
          this.lastGraph = graph;
        }
      } catch {
        if (this.state !== 'closed') this.set('closed');
      }
    };
    this.set('connecting');
    await once();
    this.poll = setInterval(() => void once(), 250);
  }

  onEvent(h: EventHandler): () => void {
    this.events.add(h);
    return () => this.events.delete(h);
  }

  onState(h: (c: Connection) => void): () => void {
    this.states.add(h);
    h(this.state);
    return () => this.states.delete(h);
  }

  private set(state: Connection): void {
    this.state = state;
    this.states.forEach((h) => h(state));
  }

  private connect(): void {
    this.set('connecting');
    const scheme = window.location.protocol === 'https:' ? 'wss' : 'ws';
    const url = `${scheme}://${window.location.host}/ws?token=${encodeURIComponent(this.token)}`;
    const socket = new WebSocket(url);
    this.socket = socket;
    socket.onopen = () => this.set('open');
    socket.onmessage = (m) => this.receive(String(m.data));
    socket.onclose = () => {
      this.socket = null;
      this.pending.forEach((p) => p.reject(new Error('the connection to the daemon was lost')));
      this.pending.clear();
      this.set('closed');
      if (!this.stopped) this.retry = setTimeout(() => this.connect(), 1500);
    };
    socket.onerror = () => socket.close();
  }

  private receive(text: string): void {
    let msg: { id?: number | null; result?: unknown; error?: { code: number; message: string; data?: unknown }; method?: string; params?: unknown };
    try {
      msg = JSON.parse(text);
    } catch {
      return;
    }
    if (msg.method && msg.id === undefined) {
      this.events.forEach((h) => h(msg.method as string, msg.params));
      return;
    }
    if (typeof msg.id !== 'number') return;
    const p = this.pending.get(msg.id);
    if (!p) return;
    this.pending.delete(msg.id);
    if (msg.error) p.reject(new RpcError(msg.error.code, msg.error.message, msg.error.data));
    else p.resolve(msg.result);
  }

  request<T>(method: string, params?: unknown): Promise<T> {
    if (this.invoke) {
      return this.invoke('rpc', { method, params: params ?? null }).then(
        (r) => r as T,
        (e: { code?: number; message?: string; data?: unknown }) => {
          throw new RpcError(e?.code ?? -1, e?.message ?? String(e), e?.data);
        },
      );
    }
    return new Promise<T>((resolve, reject) => {
      if (!this.socket || this.socket.readyState !== WebSocket.OPEN) {
        reject(new Error('not connected to the daemon'));
        return;
      }
      const id = this.nextId++;
      this.pending.set(id, { resolve: resolve as (v: unknown) => void, reject });
      this.socket.send(JSON.stringify({ jsonrpc: '2.0', id, method, params: params ?? null }));
    });
  }
}

/** The daemon's methods, typed (see src/protocol/methods.rs). */
export const api = (rpc: Rpc) => ({
  systemInfo: () => rpc.request<SystemInfo>('system.info'),
  devices: () => rpc.request<DeviceInfo[]>('devices.list'),
  staleDevices: () => rpc.request<StaleDevices>('devices.stale'),
  cleanDevices: () => rpc.request<CleanOutcome>('devices.clean'),
  apps: () => rpc.request<AppInfo[]>('apps.list'),
  graphGet: () => rpc.request<GraphSpec>('graph.get'),
  graphValidate: (g: GraphSpec) => rpc.request<ValidateResult>('graph.validate', g),
  graphSet: (g: GraphSpec) => rpc.request<null>('graph.set', g),
  status: () => rpc.request<EngineStatus>('engine.status'),
  stop: () => rpc.request<null>('engine.stop'),
  shutdown: () => rpc.request<null>('system.shutdown'),
  layoutGet: () => rpc.request<Layout>('layout.get'),
  layoutSet: (l: Layout) => rpc.request<null>('layout.set', l),
});

export const EVENT_STATUS = 'engine.status';
export const EVENT_GRAPH = 'graph.changed';

/** Desktop window only: starts the daemon that sits next to the window's executable. */
export async function startDaemon(): Promise<void> {
  const { invoke } = await import('@tauri-apps/api/core');
  await invoke('launch_daemon');
}
