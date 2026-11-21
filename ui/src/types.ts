// Mirror of the Rust types that travel over the control protocol (serde's JSON shapes). Keep in sync with
// src/types, src/hal and src/protocol of the daemon.

declare const __OMU_DEV_TOKEN__: string;
declare const __PRODUCT_NAME__: string;
export const DEV_TOKEN: string = __OMU_DEV_TOKEN__;
export const PRODUCT_NAME: string = __PRODUCT_NAME__;

export type AppTarget = { pid: number } | { process_name: string };
export type DeviceRef = 'default' | { id: string } | { name: string };
export type LogicOp = 'and' | 'or' | 'xor' | 'not';
export type MathOp = 'add' | 'subtract' | 'multiply' | 'divide' | 'min' | 'max';
export type CompareOp = 'greater' | 'less' | 'greater_or_equal' | 'less_or_equal' | 'equal' | 'not_equal';
export type TextOp = 'equals' | 'contains' | 'starts_with' | 'ends_with';
export type RoundMode = 'nearest' | 'down' | 'up';
/** What is typed in a block for an input that nothing is linked to. */
export type InputValue = boolean | number | string;

export type NodeKind =
  | { type: 'app_capture'; target: AppTarget }
  | { type: 'device_capture'; device: DeviceRef }
  | { type: 'device_render'; device: DeviceRef }
  | { type: 'virtual_speaker'; name: string }
  | { type: 'soundboard'; board: string; triggers?: string[] }
  | { type: 'virtual_mic'; name: string }
  | { type: 'network_in'; port: number; stream?: string; from?: string }
  | { type: 'network_out'; host: string; port: number; stream?: string }
  | { type: 'web_in'; name: string }
  | { type: 'web_out'; name: string }
  | { type: 'stream_in'; url: string }
  | { type: 'stream_out'; url: string }
  | { type: 'gain'; db: number }
  | { type: 'mixer' }
  | { type: 'gate' }
  | { type: 'branch'; branches: number }
  | { type: 'select'; branches: number }
  | { type: 'delay'; ms: number }
  | { type: 'noise_reduction' }
  | { type: 'deep_filter'; limit_db?: number }
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
  | { type: 'change' }
  | { type: 'text'; value: string }
  | { type: 'integer'; value: number }
  | { type: 'number'; value: number }
  | { type: 'join'; template: string; args?: number }
  | { type: 'math'; op: MathOp }
  | { type: 'compare'; op: CompareOp }
  | { type: 'text_compare'; op: TextOp }
  | { type: 'choose' }
  | { type: 'counter'; initial?: number }
  | { type: 'volume' }
  | { type: 'to_number' }
  | { type: 'round'; mode: RoundMode }
  | { type: 'from_condition' }
  | { type: 'to_condition' }
  | { type: 'group'; graph: GraphSpec; exposed: { name: string; node: string; port: string }[]; layout?: Record<string, [number, number]> };

export type NodeType = NodeKind['type'];
/** The eight families of blocks. */
export type Category = 'input' | 'output' | 'processing' | 'condition' | 'trigger' | 'number' | 'text' | 'conversion' | 'group';
/** The kinds of flow between blocks. */
export type FlowType = 'audio' | 'condition' | 'action' | 'text' | 'int' | 'float';

/**
 * A block of the graph. `id` is what the links call it (an editor makes a random one, a graph written by hand can use
 * any text that no other block uses); `display` is the name people give it, free text, missing when there is none.
 */
export type NodeSpec = { id: string; display?: string; /** Sends nothing: to hear the rest of the graph without it. */ muted?: boolean; /** What is typed for the inputs nothing is linked to, by port name. */ inputs?: Record<string, InputValue> } & NodeKind;
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
  /** For a block that makes a value (text, a number): what it is now, written out. */
  value?: string | null;
};
/** One line of the program's log. */
export type LogLine = { seq: number; text: string };
/** How loud the sound at the input of a block has been, cycle after cycle (decibels, the oldest first). */
export type Levels = { cycle_ms: number; db: number[] };
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
/** One problem with a graph, as data: a stable code, the block and connection point it is about, what to do. */
export type Diagnostic = { severity: 'error' | 'warning' | 'info'; code: string; message: string; node?: string; port?: string; hint?: string };
export type ValidateResult = { ok: boolean; errors: string[]; diagnostics?: Diagnostic[] };

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
export type TrustMode = 'system' | 'own';
export type Settings = { sound_cache_mb: number; compress_above_mb: number; show_tray_icon: boolean; start_at_logon: boolean; trust: TrustMode; certificates_dir?: string; remote_access: boolean };
export type RemoteInfo = { enabled: boolean; port: number; urls: string[]; fingerprint?: string | null; certificate_generated: boolean; error?: string | null };
export type CertificateInfo = { file: string; subject: string; issuer: string; expires: string; expired: boolean };
export type TrustReport = { mode: TrustMode; folder: string; folder_exists: boolean; certificates: CertificateInfo[]; problems: Diagnostic[] };

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
  /** This build of the program has what is needed to use the module. */
  supported?: boolean;
};
