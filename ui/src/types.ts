// Mirror of the Rust types that travel over the control protocol (serde's JSON shapes). Keep in sync with
// src/types, src/hal and src/protocol of the daemon.

declare const __OMU_DEV_TOKEN__: string;
declare const __PRODUCT_NAME__: string;
export const DEV_TOKEN: string = __OMU_DEV_TOKEN__;
export const PRODUCT_NAME: string = __PRODUCT_NAME__;

export type AppTarget = { pid: number } | { process_name: string };
export type DeviceRef = 'default' | { id: string } | { name: string };

export type NodeKind =
  | { type: 'app_capture'; target: AppTarget }
  | { type: 'device_capture'; device: DeviceRef }
  | { type: 'device_render'; device: DeviceRef }
  | { type: 'virtual_mic'; name: string }
  | { type: 'gain'; db: number }
  | { type: 'mixer' };

export type NodeType = NodeKind['type'];
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
  capabilities: { devices: boolean; app_capture: boolean; virtual_devices: boolean };
  virtual_microphones: ProviderInfo | null;
};

export type Layout = Record<string, { x: number; y: number }>;
export type ValidateResult = { ok: boolean; errors: string[] };
