// Mirror of the Rust types that travel over the control protocol (serde's JSON shapes). Keep in sync with
// src/types, src/hal and src/protocol of the daemon.

declare const __OMU_DEV_TOKEN__: string;
declare const __PRODUCT_NAME__: string;
export const DEV_TOKEN: string = __OMU_DEV_TOKEN__;
export const PRODUCT_NAME: string = __PRODUCT_NAME__;

export type AppTarget = { pid: number } | { process_name: string };
export type DeviceRef = 'default' | { id: string } | { name: string };
export type LogicOp = 'and' | 'or' | 'xor' | 'not';

export type NodeKind =
  | { type: 'app_capture'; target: AppTarget }
  | { type: 'device_capture'; device: DeviceRef }
  | { type: 'device_render'; device: DeviceRef }
  | { type: 'virtual_speaker'; name: string }
  | { type: 'soundboard'; board: string; triggers?: string[] }
  | { type: 'virtual_mic'; name: string }
  | { type: 'gain'; db: number }
  | { type: 'mixer' }
  | { type: 'gate' }
  | { type: 'branch'; branches: number }
  | { type: 'select'; branches: number }
  | { type: 'delay'; ms: number }
  | { type: 'noise_reduction' }
  | { type: 'variable'; initial: boolean }
  | { type: 'level'; threshold_db: number }
  | { type: 'logic'; op: LogicOp }
  | { type: 'not' }
  | { type: 'all' }
  | { type: 'any' }
  | { type: 'hold'; ms: number }
  | { type: 'after'; ms: number }
  | { type: 'hotkey'; keys: string }
  | { type: 'timer'; interval_ms: number }
  | { type: 'change' };

export type NodeType = NodeKind['type'];
/** The five families of blocks. */
export type Category = 'input' | 'output' | 'processing' | 'condition' | 'trigger';
/** The three kinds of flow between blocks. */
export type FlowType = 'audio' | 'condition' | 'action';

/**
 * A block of the graph. `id` is what the links call it (an editor makes a random one, a graph written by hand can use
 * any text that no other block uses); `display` is the name people give it, free text, missing when there is none.
 */
export type NodeSpec = { id: string; display?: string } & NodeKind;
export type PortRef = { node: string; port: string };
export type EdgeSpec = { from: PortRef; to: PortRef };
export type GraphSpec = { nodes: NodeSpec[]; edges: EdgeSpec[] };

export type NodeStatus = {
  id: string;
  state: 'running' | 'failed';
  message?: string;
  peak: number;
  underruns: number;
  overruns: number;
  fill_ms: number;
  frames: number;
  /** For a block holding a yes/no: its value now. */
  active?: boolean | null;
  /** Actions a block has sent. */
  events?: number;
};
export type EngineStatus = { running: boolean; cycles: number; late_cycles: number; nodes: NodeStatus[] };

export type DeviceInfo = {
  id: string;
  name: string;
  direction: 'capture' | 'render';
  is_default: boolean;
  format: { sample_rate: number; channels: number } | null;
};
export type AppInfo = { pid: number; name: string; title: string | null; active: boolean };

export type ProviderInfo = { name: string; available: boolean; error: string | null };
export type SystemInfo = {
  name: string;
  version: string;
  protocol_version: number;
  platform: string;
  backend: string;
  capabilities: { devices: boolean; app_capture: boolean; virtual_devices: boolean; hotkeys?: boolean };
  virtual_microphones: ProviderInfo | null;
  /** The previous run of the daemon did not stop cleanly; `crash_report` is the file it left, when it left one. */
  previous_run_unclean?: boolean;
  crash_report?: string | null;
};

/**
 * How the editor draws one block: where it is and, for a block that sums its inputs (the mixer), how many input points
 * it shows and which point each link is on. None of this is part of the audio graph.
 */
export type NodeLayout = { x: number; y: number; inputs?: number; slots?: Record<string, number> };
export type Layout = Record<string, NodeLayout>;
export type ValidateResult = { ok: boolean; errors: string[] };

/** An old virtual device the system still remembers (a name a microphone or speaker used to have). */
export type StaleDevice = { name: string };
/** What could be cleaned up, and whether doing it would ask for administrator rights. */
export type StaleDevices = { supported: boolean; elevated: boolean; stale: StaleDevice[] };
export type CleanOutcome = { removed: number; remaining: number; asked_for_rights: boolean };

/** A sound that was imported (converted once to the format sounds are stored in). */
export type SoundInfo = { id: string; name: string; channels: number; frames: number; bytes: number; compressed?: boolean };
/** One button of a soundboard. */
export type Pad = { id: string; name: string; sound?: string | null; gain_db?: number; looped?: boolean; exclusive?: boolean; hold?: boolean; fade_ms?: number; start_ms?: number; end_ms?: number; color?: string | null; key?: string | null };
/** A pad whose keyboard shortcut is not working (another program has the keys, or the system has no shortcuts). */
export type HotkeyProblem = { board: string; pad: string; message: string };
export type Board = { id: string; name: string; pads: Pad[] };
/** Everything saved about soundboards (they are not part of the graph: a soundboard block only names one). */
export type SoundLibrary = { sounds: SoundInfo[]; boards: Board[] };
/** How the soundboards' memory is used. */
export type SoundStatus = {
  active: boolean;
  cache_limit_bytes: number;
  cache_allocated_bytes: number;
  cache_used_bytes: number;
  sounds_loaded: number;
  active_boards: string[];
  late_frames: number;
  streams: number;
  hotkey_problems?: HotkeyProblem[];
};
export type Settings = { sound_cache_mb: number; compress_above_mb: number; show_tray_icon: boolean };

/** How far a download is (`total` is 0 when the size is not known). */
export type DownloadProgress = { done: number; total: number };
/** An optional module (FFmpeg): a component that is not part of the program and that people switch on. */
export type ModuleInfo = {
  id: string;
  name: string;
  description: string;
  enabled: boolean;
  available: boolean;
  /** Where the usable copy is: the modules folder, or the system. */
  source?: 'local' | 'system';
  version?: string;
  installing?: DownloadProgress;
  error?: string;
  downloadable: boolean;
  download_mb: number;
};
