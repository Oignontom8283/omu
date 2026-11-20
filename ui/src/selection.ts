// Copying and pasting blocks: what is copied is the selected blocks and the links that join two of them; what is pasted is made of
// new blocks (new identifiers), put where the pointer is, still linked among themselves.

import type { NodeSpec } from './types.ts';

export type ClipNode = { id: string; position: { x: number; y: number }; spec: NodeSpec; inputs?: number };
export type ClipEdge = { source: string; target: string; sourceHandle: string; targetHandle: string };
export type Clip = { nodes: ClipNode[]; edges: ClipEdge[] };

/** The blocks of `ids` and the links between them (a link to a block that is not copied is left behind). */
export function copyOf(nodes: ClipNode[], edges: ClipEdge[], ids: ReadonlySet<string>): Clip | null {
  const picked = nodes.filter((n) => ids.has(n.id));
  if (picked.length === 0) return null;
  return {
    nodes: picked.map((n) => ({ id: n.id, position: { ...n.position }, spec: n.spec, ...(n.inputs === undefined ? {} : { inputs: n.inputs }) })),
    edges: edges.filter((e) => ids.has(e.source) && ids.has(e.target)).map((e) => ({ source: e.source, target: e.target, sourceHandle: e.sourceHandle, targetHandle: e.targetHandle })),
  };
}

/**
 * New blocks from a clip. The top left corner of what was copied goes to `at` (or moves `offset` down and right when there is no
 * place to put it), `snap` brings a position to the grid. The identifiers are new (`newId`), the links follow them.
 */
export function pasteOf(clip: Clip, newId: () => string, at: { x: number; y: number } | null, offset: number, snap: (p: { x: number; y: number }) => { x: number; y: number }): Clip & { ids: string[] } {
  const left = Math.min(...clip.nodes.map((n) => n.position.x));
  const top = Math.min(...clip.nodes.map((n) => n.position.y));
  const shift = at ? { x: at.x - left, y: at.y - top } : { x: offset, y: offset };
  const renamed = new Map(clip.nodes.map((n) => [n.id, newId()]));
  const nodes = clip.nodes.map((n) => {
    const id = renamed.get(n.id) as string;
    return { id, position: snap({ x: n.position.x + shift.x, y: n.position.y + shift.y }), spec: { ...n.spec, id } as NodeSpec, ...(n.inputs === undefined ? {} : { inputs: n.inputs }) };
  });
  const edges = clip.edges.map((e) => ({ ...e, source: renamed.get(e.source) as string, target: renamed.get(e.target) as string }));
  return { nodes, edges, ids: nodes.map((n) => n.id) };
}
