// Pure logic of the editor: the families and kinds of blocks, their ports, conversion between the daemon's graph and
// the editor's nodes and edges, layout, change detection, connection rules. No React here: it is unit-tested with Node.
// The ports mirror src/types/node.rs of the daemon (the daemon validates again when a graph is applied).

import { PRODUCT_NAME } from './types.ts';
import type { Category, DeviceRef, EdgeSpec, FlowType, GraphSpec, InputValue, Layout, NodeKind, NodeLayout, NodeSpec, NodeType } from './types.ts';

/** The gap of the editor's grid, in pixels: blocks snap to it and the rows of their ports are as high. */
export const GRID = 24;
/** The gap between two blocks side by side, the size of a dot of the grid: a block is its cells less this gap. */
export const GAP = 4;
/** A block is ten cells wide. */
export const NODE_WIDTH = 10 * GRID;

/**
 * A port of a block. `label` is the translation key of its name and `n` the number it shows (`else if 2`); a port
 * with `gapBefore` is drawn one row lower, so that the rows of a branch line up (`if` with `then`).
 */
/**
 * How far a number input can go (the daemon's `PortLimits`): `min` and `max` are never passed, a number that comes in by a
 * flow is cropped to them; `softMin` and `softMax` are the range a slider offers.
 */
export type PortLimits = { min: number; max: number; softMin: number; softMax: number };

/** The limits of the settings that have some (the same table as the daemon's, which crops the flows). */
export const LIMITS: Record<string, PortLimits> = {
  db: { min: -120, max: 40, softMin: -60, softMax: 24 },
  threshold_db: { min: -120, max: 0, softMin: -80, softMax: 0 },
  ms: { min: 0, max: 60000, softMin: 0, softMax: 5000 },
  interval_ms: { min: 10, max: 3600000, softMin: 10, softMax: 5000 },
  port: { min: 1, max: 65535, softMin: 1, softMax: 65535 },
};

/** `value` brought back into the limits (a number that is not a number becomes the least). */
export const cropTo = (limits: PortLimits, value: number): number => (Number.isNaN(value) ? limits.min : Math.min(limits.max, Math.max(limits.min, value)));

/**
 * A port of a block. `label` is the translation key of its name and `n` the number it shows (`else if 2`). Every input that
 * is not a sound carries its own value in the block (`inline`): a number, a text. When it is `param`, the value is a setting
 * of the block's kind (`field`) that a flow can give; otherwise it is kept with the other values typed in the block.
 */
export type PortMeta = {
  name: string;
  dir: 'in' | 'out';
  flow: FlowType;
  multi: boolean;
  label: string;
  n?: number;
  gapBefore?: boolean;
  entries?: boolean;
  /** Gives a setting of the block from a flow. */
  param?: boolean;
  /** The setting of the block's kind this input stands for. */
  field?: string;
  /** Shows a field to type the value in, next to the connection point. */
  inline?: boolean;
  limits?: PortLimits;
  /** What the number is counted in. */
  unit?: string;
  /** The translation key of what the input is for (its explanation is that key + `.help`). */
  help?: string;
  /** The control that draws its value: the name of a registered control (see `kit/registry.ts`); by default one is chosen from the sort of value. */
  control?: string;
  /** The translation key of what its field shows while empty. */
  placeholder?: string;
  /** The size of a step of its number (arrows, dragging). */
  step?: number;
};

/** What decides a block's ports: its kind, and for the blocks that branch, how many conditions they have. */
export type KindLike = { type: NodeType; branches?: number; triggers?: string[]; args?: number; graph?: GraphSpec; exposed?: { name: string; node: string; port: string }[] };

/** The most conditions a branching block can have (same as the daemon's). */
export const MAX_BRANCHES = 16;
/** The most arguments a join block can have (same as the daemon's). */
export const MAX_JOIN_ARGS = 16;

/** The input of a block that gives one of its settings from a flow (the flow wins over what is typed in the block). */
const param = (name: string, flow: FlowType, help: string, unit?: string): PortMeta => p(name, 'in', flow, false, { param: true, field: name, limits: LIMITS[name], help, unit });

/** Whether an input of flow `input` takes a flow of type `from` (the same as the daemon's rule). */
export function accepts(input: FlowType, from: FlowType): boolean {
  return input === from || (input === 'float' && from === 'int') || (input === 'text' && (from === 'int' || from === 'float' || from === 'condition'));
}

/** The translation key of a port's name: `port.set`, but `port.in.condition` for the input of a condition block. */
function labelKey(name: string, flow: FlowType): string {
  return (name === 'in' || name === 'out' || name === 'a' || name === 'b') && flow !== 'audio' ? `port.${name}.${flow}` : `port.${name}`;
}

// A sound input takes any number of links (the daemon adds the sounds together); the other inputs only when they are
// told to (the actions of a variable, the conditions of `all` and `any`).
const p = (name: string, dir: 'in' | 'out', flow: FlowType, multi = false, extra: Partial<PortMeta> = {}): PortMeta => {
  const sums = multi || (dir === 'in' && flow === 'audio');
  return {
    name,
    dir,
    flow,
    multi: sums,
    label: labelKey(name, flow),
    // an input that is not a sound or an action carries a value of its own, typed next to it
    inline: dir === 'in' && (flow === 'text' || flow === 'int' || flow === 'float') && !sums && !extra.entries,
    ...extra,
  };
};

const range = (n: number): number[] => Array.from({ length: n }, (_, i) => i + 1);

/** How many conditions a branching block has: at least one, and what the daemon allows at most. */
export function branchCount(kind: KindLike): number {
  const n = kind.branches;
  return Number.isInteger(n) ? Math.max(1, Math.min(MAX_BRANCHES, n as number)) : 1;
}

/**
 * The groups of rows a block draws on an orange zone: what belongs together logically. Rows are counted from the first
 * row of the port columns (a `gapBefore` port takes the row of its gap). A branch groups its conditions with the sounds
 * they send (and the `else`); a select groups each sound with its condition, and the `else` stands alone.
 */
export function zonesOf(kind: KindLike): { start: number; rows: number }[] {
  switch (kind.type) {
    case 'branch':
      return [{ start: 1, rows: branchCount(kind) + 1 }];
    case 'select': {
      const n = branchCount(kind);
      return [...range(n).map((k) => ({ start: (k - 1) * 2, rows: 2 })), { start: n * 2, rows: 1 }];
    }
    default:
      return [];
  }
}

/** The `if` / `else if` condition inputs `when1`.. of a branching block: the first one is the `if`. */
const whenPort = (k: number): PortMeta => p(`when${k}`, 'in', 'condition', false, { label: k === 1 ? 'port.if' : 'port.elif', n: k === 1 ? undefined : k });

/** The ports of a block, inputs and outputs in the order they are drawn. */
export function portsOf(kind: KindLike): PortMeta[] {
  switch (kind.type) {
    case 'app_capture':
    case 'device_capture':
    case 'virtual_speaker':
      return [p('out', 'out', 'audio')];
    case 'network_in':
      return [param('port', 'int', 'field.listenPort'), param('stream', 'text', 'field.streamName'), param('from', 'text', 'field.fromComputer'), p('out', 'out', 'audio')];
    case 'soundboard': {
      // the sound, then an action input for each pad the graph can play, and one that stops the whole soundboard
      const n = Math.min(kind.triggers?.length ?? 0, MAX_BRANCHES);
      if (n === 0) return [p('out', 'out', 'audio')];
      return [...range(n).map((k) => p(`play${k}`, 'in', 'action', true, { label: 'port.play', n: k })), p('stop', 'in', 'action', true), p('out', 'out', 'audio')];
    }
    case 'device_render':
    case 'virtual_mic':
      return [p('in', 'in', 'audio')];
    case 'network_out':
      return [p('in', 'in', 'audio'), param('host', 'text', 'field.sendTo'), param('port', 'int', 'field.sendPort'), param('stream', 'text', 'field.streamName')];
    case 'web_in':
      return [param('name', 'text', 'field.webName'), p('out', 'out', 'audio')];
    case 'web_out':
      return [p('in', 'in', 'audio'), param('name', 'text', 'field.webName')];
    case 'stream_in':
      return [param('url', 'text', 'field.streamUrl'), p('out', 'out', 'audio')];
    case 'stream_out':
      return [p('in', 'in', 'audio'), param('url', 'text', 'field.streamUrl')];
    case 'gain':
      return [p('in', 'in', 'audio'), param('db', 'float', 'field.gain', 'dB'), p('out', 'out', 'audio')];
    case 'delay':
    case 'noise_reduction':
    case 'deep_filter':
      return [p('in', 'in', 'audio'), p('out', 'out', 'audio')];
    case 'mixer':
      return [p('in', 'in', 'audio', true, { entries: true }), p('out', 'out', 'audio')];
    case 'gate':
      return [p('in', 'in', 'audio'), p('open', 'in', 'condition'), p('out', 'out', 'audio')];
    case 'branch': {
      // one sound in, a condition per branch, and a sound out per branch plus the `else`
      const ks = range(branchCount(kind));
      return [
        p('in', 'in', 'audio'),
        ...ks.map(whenPort),
        ...ks.map((k) => p(`out${k}`, 'out', 'audio', false, { label: 'port.then', n: k, gapBefore: k === 1 })),
        p('else', 'out', 'audio'),
      ];
    }
    case 'select': {
      // a sound per branch, next to its condition, then the sound for when no condition is true
      const ks = range(branchCount(kind));
      return [...ks.flatMap((k) => [p(`in${k}`, 'in', 'audio', false, { label: 'port.pick', n: k }), whenPort(k)]), p('else', 'in', 'audio'), p('out', 'out', 'audio')];
    }
    case 'variable':
      return [p('set', 'in', 'action', true), p('clear', 'in', 'action', true), p('toggle', 'in', 'action', true), p('value', 'out', 'condition')];
    case 'level':
      return [p('in', 'in', 'audio'), param('threshold_db', 'float', 'field.threshold', 'dB'), p('active', 'out', 'condition')];
    case 'logic':
      return [p('a', 'in', 'condition'), p('b', 'in', 'condition'), p('out', 'out', 'condition')];
    case 'not':
      return [p('in', 'in', 'condition'), p('out', 'out', 'condition')];
    case 'hold':
    case 'after':
      return [p('in', 'in', 'condition'), param('ms', 'int', kind.type === 'hold' ? 'field.hold' : 'field.after', 'ms'), p('out', 'out', 'condition')];
    case 'all':
    case 'any':
      return [p('in', 'in', 'condition', true, { entries: true }), p('out', 'out', 'condition')];
    case 'hotkey':
      return [p('pressed', 'out', 'action')];
    case 'timer':
      return [param('interval_ms', 'int', 'field.interval', 'ms'), p('tick', 'out', 'action')];
    case 'change':
      return [p('in', 'in', 'condition'), p('rise', 'out', 'action'), p('fall', 'out', 'action')];
    case 'text':
      return [p('out', 'out', 'text')];
    case 'integer':
      return [p('out', 'out', 'int')];
    case 'number':
      return [p('out', 'out', 'float')];
    case 'join': {
      const n = Number.isInteger(kind.args) ? Math.max(1, Math.min(MAX_JOIN_ARGS, kind.args as number)) : 2;
      return [param('template', 'text', 'field.template'), ...range(n).map((k) => p(`arg${k}`, 'in', 'text', false, { label: 'port.arg', n: k })), p('out', 'out', 'text')];
    }
    case 'math':
      return [p('a', 'in', 'float'), p('b', 'in', 'float'), p('out', 'out', 'float')];
    case 'compare':
      return [p('a', 'in', 'float'), p('b', 'in', 'float'), p('out', 'out', 'condition')];
    case 'text_compare':
      return [p('a', 'in', 'text'), p('b', 'in', 'text'), p('out', 'out', 'condition')];
    case 'choose':
      return [p('when', 'in', 'condition'), p('then', 'in', 'float'), p('else', 'in', 'float'), p('out', 'out', 'float')];
    case 'counter':
      return [p('up', 'in', 'action', true), p('down', 'in', 'action', true), p('reset', 'in', 'action', true), p('out', 'out', 'int')];
    case 'volume':
      return [p('in', 'in', 'audio'), p('out', 'out', 'float')];
    case 'to_number':
      return [p('in', 'in', 'text'), p('out', 'out', 'float')];
    case 'round':
      return [p('in', 'in', 'float'), p('out', 'out', 'int')];
    case 'from_condition':
      return [p('in', 'in', 'condition'), p('out', 'out', 'int')];
    case 'to_condition':
      return [p('in', 'in', 'float'), p('out', 'out', 'condition')];
    case 'group':
      // the points of its blocks that the group shows as its own: what they carry and which way they go
      return (kind.exposed ?? []).flatMap((e) => {
        const inner = kind.graph?.nodes.find((n) => n.id === e.node);
        const point = inner && portsOf(inner).find((q) => q.name === e.port);
        return point ? [p(e.name, point.dir, point.flow, point.multi, { label: e.name })] : [];
      });
  }
}

export const CATEGORIES: Category[] = ['input', 'output', 'processing', 'condition', 'trigger', 'number', 'text', 'conversion', 'group'];

/** The kinds of each family, in the order of the drop-down. */
export const KINDS_BY_CATEGORY: Record<Category, NodeType[]> = {
  input: ['app_capture', 'device_capture', 'virtual_speaker', 'soundboard', 'network_in', 'web_in', 'stream_in'],
  output: ['virtual_mic', 'device_render', 'network_out', 'web_out', 'stream_out'],
  processing: ['gain', 'mixer', 'gate', 'branch', 'select', 'delay', 'noise_reduction', 'deep_filter'],
  condition: ['variable', 'level', 'logic', 'not', 'all', 'any', 'hold', 'after', 'compare', 'text_compare', 'to_condition'],
  trigger: ['hotkey', 'timer', 'change'],
  number: ['integer', 'number', 'math', 'choose', 'counter'],
  text: ['text', 'join'],
  conversion: ['volume', 'to_number', 'round', 'from_condition'],
  // made by grouping blocks, not taken from a list
  group: [],
};

export const NODE_TYPES = CATEGORIES.flatMap((c) => KINDS_BY_CATEGORY[c]);

export function categoryOf(type: NodeType): Category {
  if (type === 'group') return 'group';
  return CATEGORIES.find((c) => KINDS_BY_CATEGORY[c].includes(type)) as Category;
}

/**
 * An input that sums or collects (the mixer's, `all` and `any`, a variable's actions) takes as many links as wanted
 * on one connection point; the daemon combines them on its own (adds sounds, and/or conditions).
 *
 * A sound or condition input of that kind has a second way to be used: the block can show several input points (a
 * button adds one), which is only clearer to look at, named `in`, `in#2`, `in#3`... For the daemon they are all the
 * port `in`.
 */
export function baseHandle(handle: string): string {
  return handle.split('#')[0];
}

/** What an input starts with when nothing was typed: zero (inside its limits), or nothing. */
export function defaultInput(port: PortMeta): InputValue {
  if (port.flow === 'text') return '';
  return port.limits ? cropTo(port.limits, 0) : 0;
}

/** The value typed in a block for one of its inputs: the setting of its kind, or what was typed for that input. */
export function inputValue(spec: NodeSpec, port: PortMeta): InputValue {
  if (port.field !== undefined) {
    const own = (spec as Record<string, unknown>)[port.field];
    return typeof own === 'number' || typeof own === 'string' || typeof own === 'boolean' ? own : defaultInput(port);
  }
  return spec.inputs?.[port.name] ?? defaultInput(port);
}

/** The values typed in a block, kept only for the inputs it has (an input that went away takes its value with it). */
export function prunedInputs(spec: NodeSpec): NodeSpec {
  if (!spec.inputs) return spec;
  const names = new Set(portsOf(spec).filter((q) => q.inline && q.field === undefined).map((q) => q.name));
  const kept = Object.fromEntries(Object.entries(spec.inputs).filter(([name]) => names.has(name)));
  const { inputs: _old, ...rest } = spec;
  return (Object.keys(kept).length ? { ...rest, inputs: kept } : rest) as NodeSpec;
}

export function portOf(kind: KindLike, name: string): PortMeta | undefined {
  return portsOf(kind).find((q) => q.name === baseHandle(name));
}

/** Whether the editor lets the user add input points to this port (the mixer's sound input, `all` and `any`). */
export function hasEntries(port: PortMeta): boolean {
  return port.dir === 'in' && port.multi && port.entries === true;
}

/** The port of a block that can show several input points, if it has one. */
export function entryPortOf(kind: KindLike): PortMeta | undefined {
  return portsOf(kind).find(hasEntries);
}

/** The input points a new block starts with, and the fewest one can be left with. */
export const DEFAULT_ENTRIES = 2;
export const MAX_ENTRIES = 32;

function handleNumber(handle: string): number {
  const n = Number(handle.split('#')[1]);
  return handle.includes('#') && n >= 2 ? n : 1;
}

function handleName(port: string, number: number): string {
  return number === 1 ? port : `${port}#${number}`;
}

type EdgeEnds = { target: string; targetHandle?: string | null };

/** How many links end on each input point of `port` of block `node` (by point number). */
export function linksPerEntry(edges: EdgeEnds[], node: string, port: string): Map<number, number> {
  const counts = new Map<number, number>();
  for (const e of edges) {
    if (e.target !== node || !e.targetHandle || baseHandle(e.targetHandle) !== port) continue;
    const n = handleNumber(e.targetHandle);
    counts.set(n, (counts.get(n) ?? 0) + 1);
  }
  return counts;
}

/** How many input points to show: those asked for, and never fewer than the highest one that has a link. */
export function entryCount(edges: EdgeEnds[], node: string, port: string, asked: number | undefined): number {
  const highest = Math.max(0, ...linksPerEntry(edges, node, port).keys());
  const wanted = Number.isInteger(asked) ? (asked as number) : DEFAULT_ENTRIES;
  return Math.min(MAX_ENTRIES, Math.max(1, wanted, highest));
}

/** The names of the connection points of a port that has `count` of them. */
export function entryHandles(port: string, count: number): string[] {
  return Array.from({ length: count }, (_, i) => handleName(port, i + 1));
}

export function entryNumber(handle: string): number {
  return handleNumber(handle);
}

export function defaultKind(type: NodeType): NodeKind {
  switch (type) {
    case 'app_capture':
      return { type, target: { process_name: '' } };
    case 'device_capture':
    case 'device_render':
      return { type, device: 'default' };
    case 'virtual_mic':
      return { type, name: PRODUCT_NAME };
    case 'virtual_speaker':
      return { type, name: PRODUCT_NAME };
    case 'soundboard':
      return { type, board: '' };
    case 'network_in':
      return { type, port: 6980, stream: '', from: '' };
    case 'network_out':
      return { type, host: '', port: 6980, stream: '' };
    case 'web_in':
      return { type, name: 'phone' };
    case 'web_out':
      return { type, name: 'listen' };
    case 'stream_in':
      return { type, url: 'rtmp://' };
    case 'stream_out':
      return { type, url: 'rtmp://' };
    case 'gain':
      return { type, db: 0 };
    case 'mixer':
    case 'gate':
    case 'not':
    case 'all':
    case 'any':
    case 'change':
    case 'noise_reduction':
      return { type };
    case 'deep_filter':
      return { type, limit_db: 100 };
    case 'branch':
    case 'select':
      return { type, branches: 1 };
    case 'delay':
      return { type, ms: 100 };
    case 'hold':
      return { type, ms: 500 };
    case 'after':
      return { type, ms: 200 };
    case 'variable':
      return { type, initial: true };
    case 'level':
      return { type, threshold_db: -40 };
    case 'logic':
      return { type, op: 'and' };
    case 'hotkey':
      return { type, keys: '' };
    case 'timer':
      return { type, interval_ms: 1000 };
    case 'text':
      return { type, value: '' };
    case 'integer':
    case 'number':
      return { type, value: 0 };
    case 'join':
      return { type, template: '$arg1 $arg2', args: 2 };
    case 'math':
      return { type, op: 'add' };
    case 'compare':
      return { type, op: 'greater' };
    case 'text_compare':
      return { type, op: 'equals' };
    case 'choose':
    case 'volume':
    case 'to_number':
    case 'from_condition':
    case 'to_condition':
      return { type };
    case 'round':
      return { type, mode: 'nearest' };
    case 'group':
      return { type, graph: { nodes: [], edges: [] }, exposed: [] };
    case 'counter':
      return { type, initial: 0 };
  }
}

/** The ports of every kind of block, with the settings a new block starts with. */
export const PORTS = Object.fromEntries(NODE_TYPES.map((t) => [t, portsOf(defaultKind(t))])) as Record<NodeType, PortMeta[]>;

/** A node id not used yet: `gain`, `gain-2`, `gain-3`... */
export function uniqueId(base: string, taken: Iterable<string>): string {
  const used = new Set(taken);
  if (!used.has(base)) return base;
  for (let n = 2; ; n++) {
    const id = `${base}-${n}`;
    if (!used.has(id)) return id;
  }
}

/** What a new block is called in the graph: a random identifier (a UUID), so that it never has to change. */
export function newId(): string {
  const c = globalThis.crypto;
  if (c && typeof c.randomUUID === 'function') return c.randomUUID();
  // a page that is not a secure context has no randomUUID: make a version 4 one from random bytes
  const b = new Uint8Array(16);
  if (c && typeof c.getRandomValues === 'function') c.getRandomValues(b);
  else for (let i = 0; i < 16; i++) b[i] = Math.floor(Math.random() * 256);
  b[6] = (b[6] & 0x0f) | 0x40;
  b[8] = (b[8] & 0x3f) | 0x80;
  const h = Array.from(b, (x) => x.toString(16).padStart(2, '0')).join('');
  return `${h.slice(0, 8)}-${h.slice(8, 12)}-${h.slice(12, 16)}-${h.slice(16, 20)}-${h.slice(20)}`;
}

const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

/** Whether an identifier was made by an editor (nobody reads it) rather than chosen by a person. */
export function isGeneratedId(id: string): boolean {
  return UUID.test(id);
}

/** A message of the daemon with the identifiers of the blocks it names replaced by what people call them. */
export function namedProblem(text: string, nodes: { id: string; display?: string }[], unnamed: string): string {
  let out = text;
  for (const n of nodes) {
    if (!out.includes(n.id)) continue;
    const label = nameOf(n) || (isGeneratedId(n.id) ? unnamed : '');
    if (label && label !== n.id) out = out.split(n.id).join(label);
  }
  return out;
}

/** The text that stands for a block when a message has to name it: its display name, else a chosen id, else nothing. */
export function nameOf(spec: { id: string; display?: string }): string {
  const display = spec.display?.trim();
  if (display) return display;
  return isGeneratedId(spec.id) ? '' : spec.id;
}

/** A block of the editor. `inputs` is how many input points it shows, for the blocks that can show several. */
export type FlowNode = { id: string; position: { x: number; y: number }; spec: NodeSpec; inputs?: number };
export type FlowEdge = { id: string; source: string; target: string; sourceHandle: string; targetHandle: string };

export function edgeId(e: EdgeSpec): string {
  return `${e.from.node}.${e.from.port}->${e.to.node}.${e.to.port}`;
}

/** Column of each node: how far it is from the sources (longest path), so a graph reads left to right. */
export function depths(graph: GraphSpec): Map<string, number> {
  const depth = new Map<string, number>(graph.nodes.map((n) => [n.id, 0]));
  // relax edges as many times as there are nodes (a cycle cannot loop forever)
  for (let i = 0; i < graph.nodes.length; i++) {
    let changed = false;
    for (const e of graph.edges) {
      const d = (depth.get(e.from.node) ?? 0) + 1;
      if (depth.has(e.to.node) && d > (depth.get(e.to.node) ?? 0) && d <= graph.nodes.length) {
        depth.set(e.to.node, d);
        changed = true;
      }
    }
    if (!changed) break;
  }
  return depth;
}

// whole numbers of grid cells, so that a block placed here has its rows of ports on the grid
export const COLUMN_WIDTH = 12 * GRID;
/** Room left for the palette that floats over the top left of the canvas. */
export const LEFT_MARGIN = 10 * GRID;
export const ROW_HEIGHT = 10 * GRID;

/** Places the nodes that have no saved position; saved positions are kept. */
export function toFlow(graph: GraphSpec, layout: Layout): { nodes: FlowNode[]; edges: FlowEdge[] } {
  const depth = depths(graph);
  const rowsInColumn = new Map<number, number>();
  const nodes: FlowNode[] = graph.nodes.map((spec) => {
    let position = layout[spec.id];
    if (!position) {
      const col = depth.get(spec.id) ?? 0;
      const row = rowsInColumn.get(col) ?? 0;
      rowsInColumn.set(col, row + 1);
      position = { x: LEFT_MARGIN + col * COLUMN_WIDTH, y: 2 * GRID + row * ROW_HEIGHT };
    }
    return { id: spec.id, position: { x: position.x, y: position.y }, spec };
  });
  const edges = graph.edges.map((e) => ({ id: edgeId(e), source: e.from.node, target: e.to.node, sourceHandle: e.from.port, targetHandle: e.to.port }));

  // On a sound input that shows several points, each link goes to the point it was left on; a link without one
  // (made from the command line, or before the layout was saved) takes the first point that has no link yet.
  const slotOf = (e: FlowEdge): number => {
    const n = layout[e.target]?.slots?.[e.id];
    return Number.isInteger(n) && (n as number) >= 1 && (n as number) <= MAX_ENTRIES ? (n as number) : 0;
  };
  const entryPort = (e: FlowEdge): boolean => {
    const spec = graph.nodes.find((n) => n.id === e.target);
    const port = spec && portOf(spec, e.targetHandle);
    return !!port && hasEntries(port);
  };
  const used = new Map<string, Set<number>>();
  const slots = new Map<string, number>();
  for (const e of edges.filter(entryPort)) {
    const n = slotOf(e);
    if (n) {
      slots.set(e.id, n);
      used.set(e.target, (used.get(e.target) ?? new Set()).add(n));
    }
  }
  for (const e of edges.filter((x) => entryPort(x) && !slots.has(x.id))) {
    const taken = used.get(e.target) ?? new Set<number>();
    let n = 1;
    while (taken.has(n)) n++;
    slots.set(e.id, n);
    used.set(e.target, taken.add(n));
  }
  for (const e of edges) if (slots.has(e.id)) e.targetHandle = handleName(e.targetHandle, slots.get(e.id) as number);

  for (const n of nodes) {
    const port = entryPortOf(n.spec);
    if (port) n.inputs = entryCount(edges, n.id, port.name, layout[n.id]?.inputs);
  }
  return { nodes, edges };
}

export function fromFlow(nodes: { spec: NodeSpec }[], edges: FlowEdge[]): GraphSpec {
  return {
    nodes: nodes.map((n) => n.spec),
    edges: edges.map((e) => ({
      from: { node: e.source, port: baseHandle(e.sourceHandle) },
      to: { node: e.target, port: baseHandle(e.targetHandle) },
    })),
  };
}

/** A canonical text of a graph, to tell whether two graphs are the same whatever the order of their lists. */
export function canonical(graph: GraphSpec): string {
  const nodes = [...graph.nodes].sort((a, b) => a.id.localeCompare(b.id));
  const edges = [...graph.edges].sort((a, b) => edgeId(a).localeCompare(edgeId(b)));
  return JSON.stringify({ nodes, edges });
}

/** What the daemon is asked to remember about the drawing: places, and for each summing block its points and links. */
export function layoutOf(nodes: FlowNode[], edges: FlowEdge[]): Layout {
  return Object.fromEntries(
    nodes.map((n) => {
      const entry: NodeLayout = { x: Math.round(n.position.x), y: Math.round(n.position.y) };
      const port = entryPortOf(n.spec);
      if (port) {
        entry.inputs = entryCount(edges, n.id, port.name, n.inputs);
        entry.slots = Object.fromEntries(edges.filter((e) => e.target === n.id).map((e) => [e.id, entryNumber(e.targetHandle)]));
      }
      return [n.id, entry];
    }),
  );
}

/** The flow carried by an edge, from the block it starts at. */
export function flowOfEdge(nodes: { id: string; spec: NodeSpec }[], edge: { source: string; sourceHandle: string | null | undefined }): FlowType | undefined {
  const from = nodes.find((n) => n.id === edge.source);
  return from && edge.sourceHandle ? portOf(from.spec, edge.sourceHandle)?.flow : undefined;
}

/**
 * Whether the editor may create the connection (the daemon validates again when the graph is applied): from an output
 * to an input of the same flow, no loop on one block, no loop through the graph, no duplicate, and one edge per input
 * except on the inputs that sum or collect (the mixer's, the actions of a variable).
 */
export function canConnect(
  nodes: { id: string; spec: NodeSpec }[],
  edges: FlowEdge[],
  c: { source: string; target: string; sourceHandle: string | null; targetHandle: string | null },
): boolean {
  if (c.source === c.target || !c.sourceHandle || !c.targetHandle) return false;
  const from = nodes.find((n) => n.id === c.source);
  const to = nodes.find((n) => n.id === c.target);
  if (!from || !to) return false;
  const out = portOf(from.spec, c.sourceHandle);
  const input = portOf(to.spec, c.targetHandle);
  if (!out || !input || out.dir !== 'out' || input.dir !== 'in' || !accepts(input.flow, out.flow)) return false;
  if (edges.some((e) => e.source === c.source && e.sourceHandle === c.sourceHandle && e.target === c.target && baseHandle(e.targetHandle) === baseHandle(c.targetHandle as string))) return false;
  if (!input.multi && edges.some((e) => e.target === c.target && e.targetHandle === c.targetHandle)) return false;
  // a path from the target back to the source would close a loop
  const next = new Map<string, string[]>();
  for (const e of edges) next.set(e.source, [...(next.get(e.source) ?? []), e.target]);
  const seen = new Set<string>();
  const stack = [c.target];
  while (stack.length) {
    const n = stack.pop() as string;
    if (n === c.source) return false;
    if (seen.has(n)) continue;
    seen.add(n);
    stack.push(...(next.get(n) ?? []));
  }
  return true;
}

/**
 * The edges after block `id` changes to kind `to` (its ports change). An edge keeps its port when the new kind has one
 * of that name and flow; otherwise it moves to the new kind's first port of the same direction and flow (a keyboard
 * shortcut becoming a timer keeps its link: `pressed` -> `tick`); if there is none, the edge is dropped.
 */
export function edgesAfterKindChange(
  nodes: { id: string; spec: NodeSpec }[],
  edges: FlowEdge[],
  id: string,
  to: NodeType,
): FlowEdge[] {
  const specOf = (nodeId: string) => nodes.find((n) => n.id === nodeId)?.spec;
  const after = defaultKind(to);
  const out: FlowEdge[] = [];
  for (const e of edges) {
    const fromSpec = specOf(e.source);
    const toSpec = specOf(e.target);
    if (!fromSpec || !toSpec) continue;
    let { sourceHandle, targetHandle } = e;
    const flow = portOf(fromSpec, e.sourceHandle)?.flow;
    if (e.source === id) {
      const same = portOf(after, sourceHandle);
      const moved = same && same.dir === 'out' && same.flow === flow ? same : portsOf(after).find((q) => q.dir === 'out' && q.flow === flow);
      if (!moved) continue;
      sourceHandle = moved.name;
    }
    if (e.target === id) {
      const same = portOf(after, targetHandle);
      const moved = same && same.dir === 'in' && same.flow === flow ? same : portsOf(after).find((q) => q.dir === 'in' && q.flow === flow);
      if (!moved) continue;
      targetHandle = moved.name;
    }
    out.push({ ...e, sourceHandle, targetHandle, id: edgeId({ from: { node: e.source, port: sourceHandle }, to: { node: e.target, port: targetHandle } }) });
  }
  return out;
}

export function deviceLabel(d: DeviceRef): string {
  return d === 'default' ? 'default' : 'id' in d ? d.id : d.name;
}

/** Level in decibels for a meter, floored at -60. */
export function levelDb(peak: number): number {
  return peak <= 0.001 ? -60 : Math.max(-60, 20 * Math.log10(peak));
}

/** Position 0..1 of a level on a meter that spans -60..0 dB. */
export function meterFill(peak: number): number {
  return (levelDb(peak) + 60) / 60;
}

/** `Ctrl+Shift+F8` from a keyboard event (the shortcut field uses it); `null` for a lone modifier key. */
export function comboFromEvent(e: { key: string; code: string; ctrlKey: boolean; altKey: boolean; shiftKey: boolean; metaKey: boolean }): string | null {
  const named: Record<string, string> = { ' ': 'Space', Escape: 'Escape', Enter: 'Enter', Tab: 'Tab', Backspace: 'Backspace', Insert: 'Insert', Delete: 'Delete', Home: 'Home', End: 'End', PageUp: 'PageUp', PageDown: 'PageDown', ArrowUp: 'Up', ArrowDown: 'Down', ArrowLeft: 'Left', ArrowRight: 'Right' };
  let key: string | null = null;
  if (/^F\d{1,2}$/.test(e.key)) key = e.key;
  else if (e.key in named) key = named[e.key];
  else if (/^Numpad\d$/.test(e.code)) key = `Num${e.code.slice(6)}`;
  else if (/^Key[A-Z]$/.test(e.code)) key = e.code.slice(3);
  else if (/^Digit\d$/.test(e.code)) key = e.code.slice(5);
  if (!key) return null;
  return [e.ctrlKey && 'Ctrl', e.altKey && 'Alt', e.shiftKey && 'Shift', e.metaKey && 'Win', key].filter(Boolean).join('+');
}
