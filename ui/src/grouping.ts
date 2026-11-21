// Making several blocks into one (a group), and taking a group apart. The daemon does the same on its side when it runs a graph (it
// takes the groups apart); here it is the editor's gesture: the blocks go inside, the links that crossed their border become the
// connection points of the group, and everything goes back where it was when the group is taken apart.

import { GRID, baseHandle, edgeId, fromFlow, nameOf, toFlow, type FlowEdge, type FlowNode } from './graph.ts';
import type { Layout, NodeSpec } from './types.ts';

type GroupSpec = Extract<NodeSpec, { type: 'group' }>;

const snap = (v: number) => Math.round(v / GRID) * GRID;
const linkId = (e: Pick<FlowEdge, 'source' | 'sourceHandle' | 'target' | 'targetHandle'>) => `${e.source}.${e.sourceHandle}->${e.target}.${e.targetHandle}`;

/** A name for a connection point of the group that is made of the block and the point it comes from, and is not used yet. */
function nameFor(spec: NodeSpec, port: string, taken: Set<string>): string {
  const base = `${nameOf(spec) || spec.type}_${port}`.toLowerCase().replace(/[^a-z0-9]+/g, '_').replace(/^_+|_+$/g, '') || port;
  let name = base;
  for (let n = 2; taken.has(name); n++) name = `${base}_${n}`;
  taken.add(name);
  return name;
}

/**
 * The editor after the blocks of `ids` are made into one group: the group (put where the top left of the blocks was), the blocks
 * that stay, the links that stay and the links that now go to the group. Null when there is nothing to group.
 */
export function groupOf(nodes: FlowNode[], edges: FlowEdge[], ids: ReadonlySet<string>, newId: () => string): { nodes: FlowNode[]; edges: FlowEdge[]; group: string } | null {
  const inside = nodes.filter((n) => ids.has(n.id));
  if (inside.length === 0 || inside.some((n) => n.spec.type === undefined)) return null;
  const internal = edges.filter((e) => ids.has(e.source) && ids.has(e.target));
  const crossing = edges.filter((e) => ids.has(e.source) !== ids.has(e.target));
  const exposed: { name: string; node: string; port: string }[] = [];
  const names = new Map<string, string>();
  const taken = new Set<string>();
  const nameOfPoint = (node: string, port: string): string => {
    const key = `${node}|${port}`;
    const known = names.get(key);
    if (known) return known;
    const spec = nodes.find((n) => n.id === node)!.spec;
    const name = nameFor(spec, port, taken);
    names.set(key, name);
    exposed.push({ name, node, port });
    return name;
  };
  const id = newId();
  const outside: FlowEdge[] = crossing.map((e) => {
    const next = ids.has(e.source)
      ? { ...e, source: id, sourceHandle: nameOfPoint(e.source, baseHandle(e.sourceHandle)) }
      : { ...e, target: id, targetHandle: nameOfPoint(e.target, baseHandle(e.targetHandle)) };
    return { ...next, id: linkId(next) };
  });
  const left = Math.min(...inside.map((n) => n.position.x));
  const top = Math.min(...inside.map((n) => n.position.y));
  const spec = {
    id,
    type: 'group',
    graph: fromFlow(inside, internal),
    exposed,
    layout: Object.fromEntries(inside.map((n) => [n.id, [n.position.x, n.position.y] as [number, number]])),
  } as GroupSpec;
  const group: FlowNode = { id, position: { x: snap(left), y: snap(top) }, spec };
  return {
    nodes: [...nodes.filter((n) => !ids.has(n.id)), group],
    edges: [...edges.filter((e) => !ids.has(e.source) && !ids.has(e.target)), ...outside],
    group: id,
  };
}

/** The editor after a group is taken apart: its blocks come back, with the links they had, and what was linked to the group is linked to them. */
export function ungroup(nodes: FlowNode[], edges: FlowEdge[], groupId: string): { nodes: FlowNode[]; edges: FlowEdge[]; ids: string[] } | null {
  const spec = nodes.find((n) => n.id === groupId);
  if (!spec || spec.spec.type !== 'group') return null;
  const group = spec.spec as GroupSpec;
  const drawn = Object.values(group.layout ?? {});
  const left = drawn.length ? Math.min(...drawn.map((p) => p[0])) : 0;
  const top = drawn.length ? Math.min(...drawn.map((p) => p[1])) : 0;
  const layout: Layout = Object.fromEntries(Object.entries(group.layout ?? {}).map(([k, p]) => [k, { x: snap(spec.position.x + p[0] - left), y: snap(spec.position.y + p[1] - top) }]));
  const inner = toFlow(group.graph, layout);
  const leads = new Map(group.exposed.map((e) => [e.name, e]));
  const rewired: FlowEdge[] = [];
  for (const e of edges) {
    if (e.source !== groupId && e.target !== groupId) {
      rewired.push(e);
      continue;
    }
    const fromGroup = e.source === groupId;
    const lead = leads.get(fromGroup ? e.sourceHandle : e.targetHandle);
    if (!lead) continue; // a link to a point the group no longer has
    const next = fromGroup ? { ...e, source: lead.node, sourceHandle: lead.port } : { ...e, target: lead.node, targetHandle: lead.port };
    rewired.push({ ...next, id: linkId(next) });
  }
  return {
    nodes: [...nodes.filter((n) => n.id !== groupId), ...inner.nodes],
    edges: [...rewired, ...inner.edges.map((e) => ({ ...e, id: e.id || edgeId({ from: { node: e.source, port: e.sourceHandle }, to: { node: e.target, port: e.targetHandle } }) }))],
    ids: inner.nodes.map((n) => n.id),
  };
}

/** How many blocks a group holds (the groups inside it count their own). */
export function sizeOf(spec: GroupSpec): number {
  return spec.graph.nodes.reduce((sum, n) => sum + (n.type === 'group' ? sizeOf(n as GroupSpec) : 1), 0);
}

/** The block of the editor a problem found in a flattened graph belongs to: `g.vol` is in the group `g`. */
export function owningBlock(id: string, nodes: { id: string }[]): string | undefined {
  if (nodes.some((n) => n.id === id)) return id;
  const parts = id.split('.');
  for (let i = parts.length - 1; i > 0; i--) {
    const prefix = parts.slice(0, i).join('.');
    if (nodes.some((n) => n.id === prefix)) return prefix;
  }
  return undefined;
}

