// Pure logic of the editor: the families and kinds of blocks, their ports, conversion between the daemon's graph and
// the editor's nodes and edges, layout, change detection, connection rules. No React here: it is unit-tested with Node.
// The ports mirror src/types/node.rs of the daemon (the daemon validates again when a graph is applied).

import type { Category, DeviceRef, EdgeSpec, FlowType, GraphSpec, Layout, NodeKind, NodeSpec, NodeType } from './types.ts';

export type PortMeta = { name: string; dir: 'in' | 'out'; flow: FlowType; multi: boolean };

const p = (name: string, dir: 'in' | 'out', flow: FlowType, multi = false): PortMeta => ({ name, dir, flow, multi });

/** The ports of each kind of block, inputs first. */
export const PORTS: Record<NodeType, PortMeta[]> = {
  app_capture: [p('out', 'out', 'audio')],
  device_capture: [p('out', 'out', 'audio')],
  device_render: [p('in', 'in', 'audio')],
  virtual_speaker: [p('out', 'out', 'audio')],
  virtual_mic: [p('in', 'in', 'audio')],
  gain: [p('in', 'in', 'audio'), p('out', 'out', 'audio')],
  mixer: [p('in', 'in', 'audio', true), p('out', 'out', 'audio')],
  gate: [p('in', 'in', 'audio'), p('open', 'in', 'condition'), p('out', 'out', 'audio')],
  variable: [p('set', 'in', 'action', true), p('clear', 'in', 'action', true), p('toggle', 'in', 'action', true), p('value', 'out', 'condition')],
  level: [p('in', 'in', 'audio'), p('active', 'out', 'condition')],
  logic: [p('a', 'in', 'condition'), p('b', 'in', 'condition'), p('out', 'out', 'condition')],
  hotkey: [p('pressed', 'out', 'action')],
  timer: [p('tick', 'out', 'action')],
};

export const CATEGORIES: Category[] = ['input', 'output', 'processing', 'condition', 'trigger'];

/** The kinds of each family, in the order of the drop-down. */
export const KINDS_BY_CATEGORY: Record<Category, NodeType[]> = {
  input: ['app_capture', 'device_capture', 'virtual_speaker'],
  output: ['virtual_mic', 'device_render'],
  processing: ['gain', 'mixer', 'gate'],
  condition: ['variable', 'level', 'logic'],
  trigger: ['hotkey', 'timer'],
};

export const NODE_TYPES = CATEGORIES.flatMap((c) => KINDS_BY_CATEGORY[c]);

export function categoryOf(type: NodeType): Category {
  return CATEGORIES.find((c) => KINDS_BY_CATEGORY[c].includes(type)) as Category;
}

export function portOf(type: NodeType, name: string): PortMeta | undefined {
  return PORTS[type].find((q) => q.name === name);
}

export function defaultKind(type: NodeType): NodeKind {
  switch (type) {
    case 'app_capture':
      return { type, target: { process_name: '' } };
    case 'device_capture':
    case 'device_render':
      return { type, device: 'default' };
    case 'virtual_mic':
      return { type, name: 'Omu' };
    case 'virtual_speaker':
      return { type, name: 'Omu' };
    case 'gain':
      return { type, db: 0 };
    case 'mixer':
    case 'gate':
      return { type };
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
  }
}

/** A node id not used yet: `gain`, `gain-2`, `gain-3`... */
export function uniqueId(base: string, taken: Iterable<string>): string {
  const used = new Set(taken);
  if (!used.has(base)) return base;
  for (let n = 2; ; n++) {
    const id = `${base}-${n}`;
    if (!used.has(id)) return id;
  }
}

export function idBase(type: NodeType): string {
  return type.replace('_', '-');
}

export type FlowNode = { id: string; position: { x: number; y: number }; spec: NodeSpec };
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

export const COLUMN_WIDTH = 300;
/** Room left for the palette that floats over the top left of the canvas. */
export const LEFT_MARGIN = 250;
export const ROW_HEIGHT = 230;

/** Places the nodes that have no saved position; saved positions are kept. */
export function toFlow(graph: GraphSpec, layout: Layout): { nodes: FlowNode[]; edges: FlowEdge[] } {
  const depth = depths(graph);
  const rowsInColumn = new Map<number, number>();
  const nodes = graph.nodes.map((spec) => {
    let position = layout[spec.id];
    if (!position) {
      const col = depth.get(spec.id) ?? 0;
      const row = rowsInColumn.get(col) ?? 0;
      rowsInColumn.set(col, row + 1);
      position = { x: LEFT_MARGIN + col * COLUMN_WIDTH, y: 40 + row * ROW_HEIGHT };
    }
    return { id: spec.id, position: { x: position.x, y: position.y }, spec };
  });
  const edges = graph.edges.map((e) => ({
    id: edgeId(e),
    source: e.from.node,
    target: e.to.node,
    sourceHandle: e.from.port,
    targetHandle: e.to.port,
  }));
  return { nodes, edges };
}

export function fromFlow(nodes: { spec: NodeSpec }[], edges: FlowEdge[]): GraphSpec {
  return {
    nodes: nodes.map((n) => n.spec),
    edges: edges.map((e) => ({
      from: { node: e.source, port: e.sourceHandle },
      to: { node: e.target, port: e.targetHandle },
    })),
  };
}

/** A canonical text of a graph, to tell whether two graphs are the same whatever the order of their lists. */
export function canonical(graph: GraphSpec): string {
  const nodes = [...graph.nodes].sort((a, b) => a.id.localeCompare(b.id));
  const edges = [...graph.edges].sort((a, b) => edgeId(a).localeCompare(edgeId(b)));
  return JSON.stringify({ nodes, edges });
}

export function positionsOf(nodes: FlowNode[]): Layout {
  return Object.fromEntries(nodes.map((n) => [n.id, { x: Math.round(n.position.x), y: Math.round(n.position.y) }]));
}

/** The flow carried by an edge, from the block it starts at. */
export function flowOfEdge(nodes: { id: string; spec: NodeSpec }[], edge: { source: string; sourceHandle: string | null | undefined }): FlowType | undefined {
  const from = nodes.find((n) => n.id === edge.source);
  return from && edge.sourceHandle ? portOf(from.spec.type, edge.sourceHandle)?.flow : undefined;
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
  const out = portOf(from.spec.type, c.sourceHandle);
  const input = portOf(to.spec.type, c.targetHandle);
  if (!out || !input || out.dir !== 'out' || input.dir !== 'in' || out.flow !== input.flow) return false;
  if (edges.some((e) => e.source === c.source && e.sourceHandle === c.sourceHandle && e.target === c.target && e.targetHandle === c.targetHandle)) return false;
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
  const typeOf = (nodeId: string) => nodes.find((n) => n.id === nodeId)?.spec.type;
  const out: FlowEdge[] = [];
  for (const e of edges) {
    const fromType = typeOf(e.source);
    const toType = typeOf(e.target);
    if (!fromType || !toType) continue;
    let { sourceHandle, targetHandle } = e;
    const flow = portOf(fromType, e.sourceHandle)?.flow;
    if (e.source === id) {
      const same = portOf(to, sourceHandle);
      const moved = same && same.dir === 'out' && same.flow === flow ? same : PORTS[to].find((q) => q.dir === 'out' && q.flow === flow);
      if (!moved) continue;
      sourceHandle = moved.name;
    }
    if (e.target === id) {
      const same = portOf(to, targetHandle);
      const moved = same && same.dir === 'in' && same.flow === flow ? same : PORTS[to].find((q) => q.dir === 'in' && q.flow === flow);
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
