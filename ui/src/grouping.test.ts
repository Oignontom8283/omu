import assert from 'node:assert/strict';
import { test } from 'node:test';
import { groupOf, owningBlock, sizeOf, ungroup } from './grouping.ts';
import type { FlowEdge, FlowNode } from './graph.ts';
import type { NodeSpec } from './types.ts';

const node = (id: string, x: number, y: number, spec: Record<string, unknown>): FlowNode => ({ id, position: { x, y }, spec: { id, ...spec } as NodeSpec });
const edge = (source: string, sourceHandle: string, target: string, targetHandle: string): FlowEdge => ({ id: `${source}.${sourceHandle}->${target}.${targetHandle}`, source, sourceHandle, target, targetHandle });

const chain = () => ({
  nodes: [
    node('app', 0, 0, { type: 'app_capture', target: { process_name: 'a.exe' } }),
    node('vol', 240, 0, { type: 'gain', db: -6 }),
    node('mic', 480, 0, { type: 'virtual_mic', name: 'M' }),
  ],
  edges: [edge('app', 'out', 'vol', 'in'), edge('vol', 'out', 'mic', 'in')],
});

test('grouping a block takes it inside and shows the points that were linked', () => {
  const { nodes, edges } = chain();
  const made = groupOf(nodes, edges, new Set(['vol']), () => 'grp')!;
  assert.deepEqual(made.nodes.map((n) => n.id).sort(), ['app', 'grp', 'mic']);
  const group = made.nodes.find((n) => n.id === 'grp')!.spec;
  assert.ok(group.type === 'group');
  assert.deepEqual(group.exposed.map((e) => `${e.name}=${e.node}.${e.port}`).sort(), ['vol_in=vol.in', 'vol_out=vol.out']);
  assert.deepEqual(group.graph.nodes.map((n) => n.id), ['vol']);
  assert.deepEqual(made.edges.map((e) => e.id).sort(), ['app.out->grp.vol_in', 'grp.vol_out->mic.in']);
  assert.deepEqual(made.nodes.find((n) => n.id === 'grp')!.position, { x: 240, y: 0 }, 'where the block was');
});

test('links between blocks that are grouped together stay inside, and one point is exposed once', () => {
  const nodes = [...chain().nodes, node('mix', 240, 240, { type: 'mixer' })];
  const edges = [edge('app', 'out', 'vol', 'in'), edge('app', 'out', 'mix', 'in'), edge('vol', 'out', 'mix', 'in#2'), edge('mix', 'out', 'mic', 'in')];
  const made = groupOf(nodes, edges, new Set(['vol', 'mix']), () => 'grp')!;
  const group = made.nodes.find((n) => n.id === 'grp')!.spec;
  assert.ok(group.type === 'group');
  assert.deepEqual(group.graph.edges, [{ from: { node: 'vol', port: 'out' }, to: { node: 'mix', port: 'in' } }], 'the link inside, by connection point');
  assert.equal(group.exposed.length, 3, 'two points linked from the app, one to the microphone');
  assert.equal(made.edges.filter((e) => e.target === 'grp').length, 2);
});

test('taking a group apart gives the blocks and the links back, where they were', () => {
  const { nodes, edges } = chain();
  const made = groupOf(nodes, edges, new Set(['vol']), () => 'grp')!;
  const back = ungroup(made.nodes, made.edges, 'grp')!;
  assert.deepEqual(back.nodes.map((n) => n.id).sort(), ['app', 'mic', 'vol']);
  assert.deepEqual(back.edges.map((e) => e.id).sort(), ['app.out->vol.in', 'vol.out->mic.in']);
  assert.deepEqual(back.nodes.find((n) => n.id === 'vol')!.position, { x: 240, y: 0 });
  assert.deepEqual(back.nodes.find((n) => n.id === 'vol')!.spec, nodes[1].spec, 'the settings did not change');
  assert.equal(ungroup(nodes, edges, 'vol'), null, 'a block that is not a group');
});

test('nothing selected is nothing to group, and the size of a group counts the groups inside', () => {
  const { nodes, edges } = chain();
  assert.equal(groupOf(nodes, edges, new Set(), () => 'g'), null);
  const outer = groupOf(nodes, edges, new Set(['app', 'vol', 'mic']), () => 'big')!.nodes[0].spec;
  assert.ok(outer.type === 'group');
  assert.equal(sizeOf(outer), 3);
});

test('a problem found in the blocks inside a group belongs to the group', () => {
  const blocks = [{ id: 'g' }, { id: 'a.b' }];
  assert.equal(owningBlock('g.vol', blocks), 'g');
  assert.equal(owningBlock('g.deep.vol', blocks), 'g');
  assert.equal(owningBlock('a.b', blocks), 'a.b', 'an identifier with a dot that is a block');
  assert.equal(owningBlock('nobody.x', blocks), undefined);
});
