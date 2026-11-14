// The control protocol, from the browser: JSON-RPC 2.0 over one WebSocket (`/ws`), with automatic reconnection.
// The same endpoint serves a daemon on this machine or on another one: the page talks to whoever served it.

import { DEV_TOKEN } from './types.ts';
import type { AppInfo, DeviceInfo, EngineStatus, GraphSpec, Layout, SystemInfo, ValidateResult } from './types.ts';

export type Connection = 'connecting' | 'open' | 'closed';

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

  start(): void {
    this.stopped = false;
    this.connect();
  }

  stop(): void {
    this.stopped = true;
    if (this.retry) clearTimeout(this.retry);
    this.socket?.close();
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
  apps: () => rpc.request<AppInfo[]>('apps.list'),
  graphGet: () => rpc.request<GraphSpec>('graph.get'),
  graphValidate: (g: GraphSpec) => rpc.request<ValidateResult>('graph.validate', g),
  graphSet: (g: GraphSpec) => rpc.request<null>('graph.set', g),
  status: () => rpc.request<EngineStatus>('engine.status'),
  stop: () => rpc.request<null>('engine.stop'),
  layoutGet: () => rpc.request<Layout>('layout.get'),
  layoutSet: (l: Layout) => rpc.request<null>('layout.set', l),
});

export const EVENT_STATUS = 'engine.status';
export const EVENT_GRAPH = 'graph.changed';
