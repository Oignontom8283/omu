// Pure graph logic of the editor: the kinds of nodes, conversion between the daemon's graph and the editor's
// nodes/edges, automatic layout, change detection, connection rules. No React here: it is unit-tested with Node.

import type { DeviceRef, EdgeSpec, GraphSpec, Layout, NodeKind, NodeSpec, NodeType } from './types.ts';

export type KindMeta = {
  label: string;
  hint: string;
  /** Accent colour of the node. */
  color: string;
  hasInput: boolean;
  hasOutput: boolean;
  /** The input accepts several incoming edges (they are summed). */
  multiInput: boolean;
};

export const KINDS: Record<NodeType, KindMeta> = {
  app_capture: { label: 'Application', hint: 'Sound of one program', color: '#4cc9f0', hasInput: false, hasOutput: true, multiInput: false },
  device_capture: { label: 'Microphone', hint: 'A recording device', color: '#80ed99', hasInput: false, hasOutput: true, multiInput: false },
  device_render: { label: 'Speakers', hint: 'A playback device', color: '#f8961e', hasInput: true, hasOutput: false, multiInput: false },
  virtual_mic: { label: 'Virtual mic', hint: 'A microphone other programs can use', color: '#f72585', hasInput: true, hasOutput: false, multiInput: false },
  gain: { label: 'Gain', hint: 'Louder or quieter', color: '#b5179e', hasInput: true, hasOutput: true, multiInput: false },
  mixer: { label: 'Mixer', hint: 'Adds several sounds together', color: '#ffd166', hasInput: true, hasOutput: true, multiInput: true },
};

export const NODE_TYPES = Object.keys(KINDS) as NodeType[];

export function defaultKind(type: NodeType): NodeKind {
  switch (type) {
    case 'app_capture':
      return { type, target: { process_name: '' } };
    case 'device_capture':
      return { type, device: 'default' };
    case 'device_render':
      return { type, device: 'default' };
    case 'virtual_mic':
      return { type, name: 'Omu' };
    case 'gain':
      return { type, db: 0 };
    case 'mixer':
      return { type };
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
export const ROW_HEIGHT = 190;

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
      position = { x: 40 + col * COLUMN_WIDTH, y: 40 + row * ROW_HEIGHT };
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

/**
 * Whether the editor may create the connection (the daemon validates again when the graph is applied):
 * output -> input, no loop on one node, no duplicate, and one edge per input except on a mixer.
 */
export function canConnect(
  nodes: { id: string; spec: NodeSpec }[],
  edges: FlowEdge[],
  c: { source: string; target: string; sourceHandle: string | null; targetHandle: string | null },
): boolean {
  if (c.source === c.target) return false;
  if (c.sourceHandle !== 'out' || c.targetHandle !== 'in') return false;
  const from = nodes.find((n) => n.id === c.source);
  const to = nodes.find((n) => n.id === c.target);
  if (!from || !to || !KINDS[from.spec.type].hasOutput || !KINDS[to.spec.type].hasInput) return false;
  if (edges.some((e) => e.source === c.source && e.target === c.target)) return false;
  if (!KINDS[to.spec.type].multiInput && edges.some((e) => e.target === c.target)) return false;
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
