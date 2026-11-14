import assert from 'node:assert/strict';
import { test } from 'node:test';
import { canConnect, canonical, depths, fromFlow, levelDb, meterFill, toFlow, uniqueId } from './graph.ts';
import type { GraphSpec } from './types.ts';

const chain: GraphSpec = {
  nodes: [
    { id: 'app', type: 'app_capture', target: { process_name: 'chrome.exe' } },
    { id: 'vol', type: 'gain', db: -6 },
    { id: 'mic', type: 'virtual_mic', name: 'Stream' },
  ],
  edges: [
    { from: { node: 'app', port: 'out' }, to: { node: 'vol', port: 'in' } },
    { from: { node: 'vol', port: 'out' }, to: { node: 'mic', port: 'in' } },
  ],
};

test('graph -> editor -> graph is the identity', () => {
  const flow = toFlow(chain, {});
  assert.deepEqual(fromFlow(flow.nodes, flow.edges), chain);
});

test('nodes without a saved position are laid out in columns by depth, saved ones stay', () => {
  const d = depths(chain);
  assert.deepEqual([d.get('app'), d.get('vol'), d.get('mic')], [0, 1, 2]);
  const flow = toFlow(chain, { vol: { x: 1, y: 2 } });
  const pos = Object.fromEntries(flow.nodes.map((n) => [n.id, n.position]));
  assert.deepEqual(pos.vol, { x: 1, y: 2 });
  assert.ok(pos.app.x < pos.mic.x);
});

test('a cycle does not hang the layout', () => {
  const cyc: GraphSpec = {
    nodes: [
      { id: 'a', type: 'gain', db: 0 },
      { id: 'b', type: 'gain', db: 0 },
    ],
    edges: [
      { from: { node: 'a', port: 'out' }, to: { node: 'b', port: 'in' } },
      { from: { node: 'b', port: 'out' }, to: { node: 'a', port: 'in' } },
    ],
  };
  assert.equal(toFlow(cyc, {}).nodes.length, 2);
});

test('unique ids', () => {
  assert.equal(uniqueId('gain', []), 'gain');
  assert.equal(uniqueId('gain', ['gain']), 'gain-2');
  assert.equal(uniqueId('gain', ['gain', 'gain-2']), 'gain-3');
});

test('canonical ignores the order of the lists', () => {
  const swapped: GraphSpec = { nodes: [...chain.nodes].reverse(), edges: [...chain.edges].reverse() };
  assert.equal(canonical(chain), canonical(swapped));
  const changed: GraphSpec = { ...chain, nodes: chain.nodes.map((n) => (n.id === 'vol' ? { ...n, db: -3 } : n)) };
  assert.notEqual(canonical(chain), canonical(changed));
});

test('connection rules', () => {
  const flow = toFlow(chain, {});
  const link = (source: string, target: string, sh = 'out', th = 'in') => canConnect(flow.nodes, flow.edges, { source, target, sourceHandle: sh, targetHandle: th });
  assert.equal(link('app', 'app'), false, 'no loop on one node');
  assert.equal(link('mic', 'app'), false, 'a virtual mic has no output');
  assert.equal(link('app', 'vol'), false, 'duplicate');
  assert.equal(link('app', 'mic'), false, 'the mic input is taken');
  assert.equal(link('vol', 'app'), false, 'a source has no input');
  assert.equal(link('app', 'vol', 'in', 'out'), false, 'wrong handles');
});

test('a mixer accepts several inputs and loops are refused', () => {
  const g: GraphSpec = {
    nodes: [
      { id: 'a', type: 'app_capture', target: { pid: 1 } },
      { id: 'b', type: 'device_capture', device: 'default' },
      { id: 'm', type: 'mixer' },
      { id: 'g', type: 'gain', db: 0 },
    ],
    edges: [
      { from: { node: 'a', port: 'out' }, to: { node: 'm', port: 'in' } },
      { from: { node: 'm', port: 'out' }, to: { node: 'g', port: 'in' } },
    ],
  };
  const flow = toFlow(g, {});
  const c = (source: string, target: string) => canConnect(flow.nodes, flow.edges, { source, target, sourceHandle: 'out', targetHandle: 'in' });
  assert.equal(c('b', 'm'), true, 'second input on a mixer');
  assert.equal(c('g', 'm'), false, 'would close the loop m -> g -> m');
});

test('meters', () => {
  assert.equal(levelDb(0), -60);
  assert.ok(Math.abs(levelDb(1)) < 1e-9);
  assert.equal(meterFill(1), 1);
  assert.equal(meterFill(0), 0);
  assert.ok(meterFill(0.1) > 0.6 && meterFill(0.1) < 0.7);
});
