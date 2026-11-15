// The control protocol, from the browser: JSON-RPC 2.0 over one WebSocket (`/ws`), with automatic reconnection.
// The same endpoint serves a daemon on this machine or on another one: the page talks to whoever served it.

import { DEV_TOKEN } from './types.ts';
import type { AppInfo, Board, CleanOutcome, DeviceInfo, EngineStatus, GraphSpec, Layout, ModuleInfo, Settings, SoundInfo, SoundLibrary, SoundStatus, StaleDevices, SystemInfo, ValidateResult } from './types.ts';

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

  constructor(readonly token: string) {}

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
  soundboards: () => rpc.request<SoundLibrary>('soundboards.get'),
  setSoundboards: (boards: Board[]) => rpc.request<null>('soundboards.set', boards),
  importSound: (path: string, name?: string) => rpc.request<SoundInfo>('sounds.import', name ? { path, name } : { path }),
  modules: () => rpc.request<ModuleInfo[]>('modules.list'),
  installModule: (id: string) => rpc.request<null>('modules.install', { id }),
  cancelModule: (id: string) => rpc.request<null>('modules.cancel', { id }),
  enableModule: (id: string, enabled: boolean) => rpc.request<null>('modules.enable', { id, enabled }),
  removeModule: (id: string) => rpc.request<null>('modules.remove', { id }),
  compressSound: (id: string) => rpc.request<SoundInfo>('sounds.compress', { id }),
  removeSound: (id: string) => rpc.request<boolean>('sounds.remove', { id }),
  playPad: (board: string, pad: string) => rpc.request<null>('soundboard.play', { board, pad }),
  stopPad: (board: string, pad?: string) => rpc.request<null>('soundboard.stop', pad ? { board, pad } : { board }),
  playing: (board: string) => rpc.request<string[]>('soundboard.playing', { board }),
  soundStatus: () => rpc.request<SoundStatus>('soundboards.status'),
  settings: () => rpc.request<Settings>('settings.get'),
  setSettings: (s: Settings) => rpc.request<null>('settings.set', s),
});

export const EVENT_STATUS = 'engine.status';
export const EVENT_GRAPH = 'graph.changed';

/** Desktop window only: starts the daemon that sits next to the window's executable. */
export async function startDaemon(): Promise<void> {
  const { invoke } = await import('@tauri-apps/api/core');
  await invoke('launch_daemon');
}

/**
 * Browser only: sends the bytes of a sound file to the daemon, which converts and keeps it (the file is written to
 * disk as it arrives, so a very long one never sits in memory). The desktop window has no address to send to: it
 * hands the daemon the path of the file instead (`sounds.import`).
 */
export async function uploadSound(token: string, file: File): Promise<SoundInfo> {
  const reply = await fetch(`/sounds?name=${encodeURIComponent(file.name)}`, {
    method: 'POST',
    headers: { Authorization: `Bearer ${token}` },
    body: file,
  });
  const text = await reply.text();
  let body: unknown = null;
  try {
    body = JSON.parse(text);
  } catch {
    /* not JSON: reported below */
  }
  if (!reply.ok) {
    const e = (body as { error?: { code?: number; message?: string } } | null)?.error;
    throw new RpcError(e?.code ?? -1, e?.message ?? (text || `HTTP ${reply.status}`));
  }
  return body as SoundInfo;
}
