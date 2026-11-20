import assert from 'node:assert/strict';
import { test } from 'node:test';
import { copyOf, pasteOf, type ClipEdge, type ClipNode } from './selection.ts';
import type { NodeSpec } from './types.ts';

const node = (id: string, x: number, y: number, extra: Partial<NodeSpec> = {}): ClipNode => ({ id, position: { x, y }, spec: { id, type: 'gain', db: 0, ...extra } as NodeSpec });
const edge = (source: string, target: string): ClipEdge => ({ source, target, sourceHandle: 'out', targetHandle: 'in' });
const snap = (p: { x: number; y: number }) => ({ x: Math.round(p.x / 24) * 24, y: Math.round(p.y / 24) * 24 });

test('copying takes the selected blocks and only the links between them', () => {
  const nodes = [node('a', 0, 0), node('b', 240, 0), node('c', 480, 0)];
  const clip = copyOf(nodes, [edge('a', 'b'), edge('b', 'c')], new Set(['a', 'b']))!;
  assert.deepEqual(clip.nodes.map((n) => n.id), ['a', 'b']);
  assert.deepEqual(clip.edges, [edge('a', 'b')], 'the link to c stays behind');
  assert.equal(copyOf(nodes, [], new Set()), null, 'nothing selected, nothing copied');
});

test('pasting makes new blocks with new identifiers, still linked among themselves', () => {
  const clip = copyOf([node('a', 0, 0), node('b', 240, 48)], [edge('a', 'b')], new Set(['a', 'b']))!;
  let n = 0;
  const pasted = pasteOf(clip, () => `new-${++n}`, null, 48, snap);
  assert.deepEqual(pasted.ids, ['new-1', 'new-2']);
  assert.equal(pasted.nodes[0].spec.id, 'new-1', 'the block knows its new identifier');
  assert.deepEqual(pasted.edges, [{ source: 'new-1', target: 'new-2', sourceHandle: 'out', targetHandle: 'in' }]);
  assert.deepEqual(pasted.nodes[1].position, { x: 288, y: 96 }, 'moved down and right by the offset');
});

test('pasting where the pointer is puts the top left corner of the copy there, on the grid', () => {
  const clip = copyOf([node('a', 100, 100), node('b', 340, 148)], [], new Set(['a', 'b']))!;
  const pasted = pasteOf(clip, (() => { let i = 0; return () => `p${++i}`; })(), { x: 500, y: 300 }, 48, snap);
  assert.deepEqual(pasted.nodes[0].position, { x: 504, y: 312 });
  assert.deepEqual(pasted.nodes[1].position, { x: 744, y: 360 });
});

test('the original is not touched by a paste', () => {
  const original = node('a', 0, 0, { display: 'Mine' });
  const clip = copyOf([original], [], new Set(['a']))!;
  pasteOf(clip, () => 'copy', null, 48, snap);
  assert.equal(original.spec.id, 'a');
  assert.equal(clip.nodes[0].spec.id, 'a');
});
