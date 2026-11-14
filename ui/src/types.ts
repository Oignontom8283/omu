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
  | { type: 'virtual_mic'; name: string }
  | { type: 'gain'; db: number }
  | { type: 'mixer' }
  | { type: 'gate' }
  | { type: 'variable'; initial: boolean }
  | { type: 'level'; threshold_db: number }
  | { type: 'logic'; op: LogicOp }
  | { type: 'hotkey'; keys: string }
  | { type: 'timer'; interval_ms: number };

export type NodeType = NodeKind['type'];
/** The five families of blocks. */
export type Category = 'input' | 'output' | 'processing' | 'condition' | 'trigger';
/** The three kinds of flow between blocks. */
export type FlowType = 'audio' | 'condition' | 'action';

export type NodeSpec = { id: string } & NodeKind;
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
