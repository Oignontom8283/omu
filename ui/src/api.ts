// The control protocol, from the browser: JSON-RPC 2.0 over one WebSocket (`/ws`), with automatic reconnection.
// The same endpoint serves a daemon on this machine or on another one: the page talks to whoever served it.

import { DEV_TOKEN } from './types.ts';
import type { AppInfo, Board, CleanOutcome, ControlAction, DeviceInfo, Diagnostic, EngineStatus, GraphSpec, ImportJob, Layout, Levels, LogLine, ModuleInfo, Settings, SoundInfo, SoundLibrary, SoundStatus, RemoteInfo, StaleDevices, SystemInfo, TrustReport, ValidateResult } from './types.ts';

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
  /** The problems the daemon attached to a rejected graph, as data (an old daemon sent plain text). */
  get diagnostics(): Diagnostic[] {
    if (!Array.isArray(this.data)) return [];
    return this.data.map((d): Diagnostic => (typeof d === 'object' && d !== null ? (d as Diagnostic) : { severity: 'error', code: 'problem', message: String(d) }));
  }
  /** Their texts. */
  get problems(): string[] {
    return this.diagnostics.map((d) => d.message);
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
  /** How many times the connection has been established: whoever follows the daemon's numbering (the logs) starts again when it changes. */
  opens = 0;

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
    // stopped while the window's commands were being loaded: nothing may start
    if (this.stopped) return;
    this.invoke = invoke as Invoke;
    let tick = 0;
    let failures = 0;
    const once = async () => {
      try {
        const status = await this.request<unknown>('engine.status');
        failures = 0;
        if (this.state !== 'open') this.set('open');
        this.events.forEach((h) => h('engine.status', status));
        if (tick++ % 8 === 0) {
          const graph = JSON.stringify(await this.request<unknown>('graph.get'));
          if (this.lastGraph && graph !== this.lastGraph) this.events.forEach((h) => h('graph.changed', JSON.parse(graph)));
          this.lastGraph = graph;
        }
      } catch {
        // one poll that fails is not a lost daemon (a busy moment): the connection is given up after three in a row
        if (++failures >= 3 && this.state !== 'closed') this.set('closed');
      }
    };
    this.set('connecting');
    await once();
    if (this.stopped) return;
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
    if (state === 'open' && this.state !== 'open') this.opens++;
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
      // a socket that was already replaced by a newer one says nothing about the connection
      if (this.socket !== socket) return;
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
      // a daemon that does not answer must not leave a screen waiting for ever (the slow methods wait for a person or a download)
      const timer = setTimeout(() => {
        this.pending.delete(id);
        reject(new Error('the daemon did not answer in time'));
      }, /^(devices\.clean|modules\.|sounds\.)/.test(method) ? 300_000 : 30_000);
      this.pending.set(id, {
        resolve: (v) => {
          clearTimeout(timer);
          (resolve as (v: unknown) => void)(v);
        },
        reject: (e) => {
          clearTimeout(timer);
          reject(e);
        },
      });
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
  levels: (node: string) => rpc.request<Levels>('engine.levels', { node }),
  /** Presses a button, or sets a switch or a slider, of the running graph (`persist`: keep where it is set with the graph). */
  control: (node: string, action: ControlAction, persist = true) => rpc.request<void>('node.control', { node, action, persist }),
  logs: (after?: number) => rpc.request<LogLine[]>('logs.recent', { after, limit: 300 }),
  opens: () => rpc.opens,
  start: () => rpc.request<null>('engine.start'),
  shutdown: () => rpc.request<null>('system.shutdown'),
  layoutGet: () => rpc.request<Layout>('layout.get'),
  layoutSet: (l: Layout) => rpc.request<null>('layout.set', l),
  soundboards: () => rpc.request<SoundLibrary>('soundboards.get'),
  setSoundboards: (boards: Board[]) => rpc.request<null>('soundboards.set', boards),
  importSound: (path: string, job?: string) => rpc.request<SoundInfo>('sounds.import', job ? { path, job } : { path }),
  imports: () => rpc.request<ImportJob[]>('sounds.imports'),
  modules: () => rpc.request<ModuleInfo[]>('modules.list'),
  installModule: (id: string) => rpc.request<null>('modules.install', { id }),
  cancelModule: (id: string) => rpc.request<null>('modules.cancel', { id }),
  enableModule: (id: string, enabled: boolean) => rpc.request<null>('modules.enable', { id, enabled }),
  removeModule: (id: string) => rpc.request<null>('modules.remove', { id }),
  compressSound: (id: string) => rpc.request<SoundInfo>('sounds.compress', { id }),
  removeSound: (id: string) => rpc.request<boolean>('sounds.remove', { id }),
  revealSound: (id: string) => rpc.request<null>('sounds.reveal', { id }),
  playPad: (board: string, pad: string) => rpc.request<null>('soundboard.play', { board, pad }),
  stopPad: (board: string, pad?: string) => rpc.request<null>('soundboard.stop', pad ? { board, pad } : { board }),
  playing: (board: string) => rpc.request<string[]>('soundboard.playing', { board }),
  soundStatus: () => rpc.request<SoundStatus>('soundboards.status'),
  settings: () => rpc.request<Settings>('settings.get'),
  setSettings: (s: Settings) => rpc.request<null>('settings.set', s),
  certificates: () => rpc.request<TrustReport>('certificates.list'),
  remote: () => rpc.request<RemoteInfo>('remote.info'),
  openLogsFolder: () => rpc.request<null>('logs.open_folder'),
  pauseHotkeys: (paused: boolean) => rpc.request<null>('hotkeys.pause', { paused }),
});

export const EVENT_STATUS = 'engine.status';
export const EVENT_GRAPH = 'graph.changed';

/** Desktop window only: the system's dialog to choose sound or video files; the paths chosen (none when dismissed). */
export async function pickSoundFiles(): Promise<string[]> {
  const { invoke } = await import('@tauri-apps/api/core');
  return (await invoke('pick_sound_files')) as string[];
}

/** True when this page runs on the same computer as the daemon (the desktop window, or a browser on this very computer): what opens a folder there is the person's own. */
export const ON_THE_DAEMON_COMPUTER = IN_DESKTOP_WINDOW || (typeof location !== 'undefined' && ['localhost', '127.0.0.1', '[::1]'].includes(location.hostname));

/** Puts a text in a file the person chooses the place of: the system's dialog in the desktop window, the browser's download elsewhere. False when the dialog is dismissed. */
export async function saveTextFile(name: string, text: string): Promise<boolean> {
  if (IN_DESKTOP_WINDOW) {
    const { invoke } = await import('@tauri-apps/api/core');
    return (await invoke('save_text_file', { name, text })) as boolean;
  }
  const url = URL.createObjectURL(new Blob([text], { type: 'text/plain;charset=utf-8' }));
  const link = document.createElement('a');
  link.href = url;
  link.download = name;
  document.body.append(link);
  link.click();
  link.remove();
  window.setTimeout(() => URL.revokeObjectURL(url), 10_000);
  return true;
}

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
export async function uploadSound(token: string, file: File, job?: string): Promise<SoundInfo> {
  const reply = await fetch(`/sounds?name=${encodeURIComponent(file.name)}${job ? `&job=${encodeURIComponent(job)}` : ''}`, {
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
