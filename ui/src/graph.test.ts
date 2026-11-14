import assert from 'node:assert/strict';
import { test } from 'node:test';
import {
  CATEGORIES,
  KINDS_BY_CATEGORY,
  NODE_TYPES,
  PORTS,
  canConnect,
  multiHandles,
  canonical,
  categoryOf,
  comboFromEvent,
  defaultKind,
  depths,
  edgesAfterKindChange,
  flowOfEdge,
  fromFlow,
  levelDb,
  meterFill,
  toFlow,
  uniqueId,
} from './graph.ts';
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

/** key -> variable -> gate, on audio: the example of the design. */
const gated: GraphSpec = {
  nodes: [
    { id: 'key', type: 'hotkey', keys: 'F8' },
    { id: 'on', type: 'variable', initial: true },
    { id: 'app', type: 'app_capture', target: { pid: 1 } },
    { id: 'gate', type: 'gate' },
    { id: 'mic', type: 'virtual_mic', name: 'M' },
  ],
  edges: [
    { from: { node: 'key', port: 'pressed' }, to: { node: 'on', port: 'toggle' } },
    { from: { node: 'on', port: 'value' }, to: { node: 'gate', port: 'open' } },
    { from: { node: 'app', port: 'out' }, to: { node: 'gate', port: 'in' } },
    { from: { node: 'gate', port: 'out' }, to: { node: 'mic', port: 'in' } },
  ],
};

test('graph -> editor -> graph is the identity, including the new blocks', () => {
  for (const g of [chain, gated]) {
    const flow = toFlow(g, {});
    assert.deepEqual(fromFlow(flow.nodes, flow.edges), g);
  }
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

test('the five families hold every kind exactly once, and each kind has ports', () => {
  assert.deepEqual(CATEGORIES, ['input', 'output', 'processing', 'condition', 'trigger']);
  assert.equal(new Set(NODE_TYPES).size, NODE_TYPES.length);
  assert.equal(NODE_TYPES.length, Object.keys(PORTS).length);
  for (const t of NODE_TYPES) {
    assert.ok(PORTS[t].length > 0, t);
    assert.equal(defaultKind(t).type, t);
    assert.ok(KINDS_BY_CATEGORY[categoryOf(t)].includes(t));
  }
});

test('the families send and receive the flows they should', () => {
  const sends = (t: (typeof NODE_TYPES)[number]) => PORTS[t].filter((q) => q.dir === 'out').map((q) => q.flow);
  const receives = (t: (typeof NODE_TYPES)[number]) => PORTS[t].filter((q) => q.dir === 'in').map((q) => q.flow);
  for (const t of KINDS_BY_CATEGORY.input) assert.deepEqual([sends(t), receives(t)], [['audio'], []]);
  for (const t of KINDS_BY_CATEGORY.output) assert.deepEqual([sends(t), receives(t)], [[], ['audio']]);
  for (const t of KINDS_BY_CATEGORY.trigger) assert.deepEqual([sends(t), receives(t)], [['action'], []], 'a trigger only sends actions');
  for (const t of KINDS_BY_CATEGORY.condition) assert.deepEqual(sends(t), ['condition']);
  for (const t of KINDS_BY_CATEGORY.processing) assert.deepEqual(sends(t), ['audio']);
});

test('connection rules: flows must match, outputs go to inputs, one edge per plain input', () => {
  const flow = toFlow(gated, {});
  const link = (source: string, target: string, sh: string, th: string) => canConnect(flow.nodes, flow.edges, { source, target, sourceHandle: sh, targetHandle: th });
  assert.equal(link('key', 'gate', 'pressed', 'open'), false, 'an action cannot open a gate: it needs a condition');
  assert.equal(link('app', 'on', 'out', 'toggle'), false, 'audio is not an action');
  assert.equal(link('on', 'gate', 'value', 'open'), false, 'duplicate');
  assert.equal(link('mic', 'app', 'in', 'out'), false, 'a virtual microphone sends nothing');
  assert.equal(link('gate', 'gate', 'out', 'in'), false, 'no loop on one block');
  assert.equal(link('app', 'mic', 'out', 'in'), false, 'the microphone input is taken');
  // several triggers may drive one variable
  const more: GraphSpec = { ...gated, nodes: [...gated.nodes, { id: 'k2', type: 'hotkey', keys: 'F9' }] };
  const f2 = toFlow(more, {});
  assert.equal(canConnect(f2.nodes, f2.edges, { source: 'k2', target: 'on', sourceHandle: 'pressed', targetHandle: 'toggle' }), true);
  assert.equal(canConnect(f2.nodes, f2.edges, { source: 'k2', target: 'on', sourceHandle: 'pressed', targetHandle: 'set' }), true);
});

test('loops are refused for every flow', () => {
  const g: GraphSpec = {
    nodes: [
      { id: 'a', type: 'logic', op: 'not' },
      { id: 'b', type: 'logic', op: 'not' },
    ],
    edges: [{ from: { node: 'a', port: 'out' }, to: { node: 'b', port: 'a' } }],
  };
  const f = toFlow(g, {});
  assert.equal(canConnect(f.nodes, f.edges, { source: 'b', target: 'a', sourceHandle: 'out', targetHandle: 'a' }), false);
  assert.equal(canConnect(f.nodes, f.edges, { source: 'b', target: 'a', sourceHandle: 'out', targetHandle: 'b' }), false);
});

test('changing the kind of a block keeps the edges that still fit and drops the others', () => {
  const flow = toFlow(gated, {});
  const nodes = flow.nodes.map((n) => ({ id: n.id, spec: n.spec }));
  // the gate becomes a gain: its audio edges stay, the condition edge (open) goes
  const asGain = edgesAfterKindChange(nodes, flow.edges, 'gate', 'gain');
  assert.equal(asGain.length, flow.edges.length - 1);
  assert.ok(!asGain.some((e) => e.targetHandle === 'open'));
  // an input app becomes a microphone: same flow, nothing is lost
  assert.equal(edgesAfterKindChange(nodes, flow.edges, 'app', 'device_capture').length, flow.edges.length);
  // a trigger becomes a timer: the link follows to the timer's own output (pressed -> tick)
  const asTimer = edgesAfterKindChange(nodes, flow.edges, 'key', 'timer');
  assert.equal(asTimer.length, flow.edges.length);
  assert.equal(asTimer.find((e) => e.source === 'key')?.sourceHandle, 'tick');
  // a variable becomes a level detector: its action input disappears
  const asLevel = edgesAfterKindChange(nodes, flow.edges, 'on', 'level');
  assert.ok(!asLevel.some((e) => e.targetHandle === 'toggle'));
});

test('the flow of an edge comes from where it starts', () => {
  const flow = toFlow(gated, {});
  const nodes = flow.nodes.map((n) => ({ id: n.id, spec: n.spec }));
  assert.deepEqual(flow.edges.map((e) => flowOfEdge(nodes, e)), ['action', 'condition', 'audio', 'audio']);
});

test('shortcuts are read from keyboard events', () => {
  const ev = (key: string, code: string, mods: Partial<{ ctrlKey: boolean; altKey: boolean; shiftKey: boolean; metaKey: boolean }> = {}) => ({
    key, code, ctrlKey: false, altKey: false, shiftKey: false, metaKey: false, ...mods,
  });
  assert.equal(comboFromEvent(ev('F8', 'F8')), 'F8');
  assert.equal(comboFromEvent(ev('m', 'KeyM', { ctrlKey: true, shiftKey: true })), 'Ctrl+Shift+M');
  assert.equal(comboFromEvent(ev('5', 'Digit5', { altKey: true })), 'Alt+5');
  assert.equal(comboFromEvent(ev('3', 'Numpad3')), 'Num3');
  assert.equal(comboFromEvent(ev(' ', 'Space', { metaKey: true })), 'Win+Space');
  assert.equal(comboFromEvent(ev('Control', 'ControlLeft', { ctrlKey: true })), null, 'a lone modifier is not a shortcut');
});

test('meters', () => {
  assert.equal(levelDb(0), -60);
  assert.ok(Math.abs(levelDb(1)) < 1e-9);
  assert.equal(meterFill(1), 1);
  assert.equal(meterFill(0), 0);
  assert.ok(meterFill(0.1) > 0.6 && meterFill(0.1) < 0.7);
});

test('a virtual speaker is an input: it sends audio, and its name survives the round trip', () => {
  assert.deepEqual(PORTS.virtual_speaker.map((q) => `${q.dir}:${q.name}`), ['out:out']);
  assert.equal(categoryOf('virtual_speaker'), 'input');
  const g: GraphSpec = {
    nodes: [
      { id: 'spk', type: 'virtual_speaker', name: 'Game' },
      { id: 'mic', type: 'virtual_mic', name: 'Stream' },
    ],
    edges: [{ from: { node: 'spk', port: 'out' }, to: { node: 'mic', port: 'in' } }],
  };
  const flow = toFlow(g, {});
  assert.deepEqual(fromFlow(flow.nodes, flow.edges), g);
});

test('a multi input gets one connection point per link, plus a free one; the daemon still sees the port `in`', () => {
  const g: GraphSpec = {
    nodes: [
      { id: 'a', type: 'app_capture', target: { pid: 1 } },
      { id: 'b', type: 'app_capture', target: { pid: 2 } },
      { id: 'c', type: 'app_capture', target: { pid: 3 } },
      { id: 'mix', type: 'mixer' },
    ],
    edges: [
      { from: { node: 'a', port: 'out' }, to: { node: 'mix', port: 'in' } },
      { from: { node: 'b', port: 'out' }, to: { node: 'mix', port: 'in' } },
      { from: { node: 'c', port: 'out' }, to: { node: 'mix', port: 'in' } },
    ],
  };
  const flow = toFlow(g, {});
  assert.deepEqual(flow.edges.map((e) => e.targetHandle), ['in', 'in#2', 'in#3']);
  assert.deepEqual(multiHandles(flow.edges, 'mix', 'in'), ['in', 'in#2', 'in#3', 'in#4']);
  assert.deepEqual(fromFlow(flow.nodes, flow.edges), g, 'the daemon only ever sees `in`');
  // an empty mixer shows a single free point; after a link in the middle goes away, the free point fills the hole
  assert.deepEqual(multiHandles([], 'mix', 'in'), ['in']);
  assert.deepEqual(multiHandles(flow.edges.filter((e) => e.targetHandle !== 'in#2'), 'mix', 'in'), ['in', 'in#2', 'in#3']);
  // another block's points are not counted
  assert.deepEqual(multiHandles(flow.edges, 'other', 'in'), ['in']);
});

test('connecting into a free point of a multi input works, the same source twice does not', () => {
  const nodes = [
    { id: 'a', spec: { id: 'a', type: 'app_capture', target: { pid: 1 } } as const },
    { id: 'b', spec: { id: 'b', type: 'app_capture', target: { pid: 2 } } as const },
    { id: 'mix', spec: { id: 'mix', type: 'mixer' } as const },
  ];
  const edges = [{ id: 'x', source: 'a', target: 'mix', sourceHandle: 'out', targetHandle: 'in' }];
  assert.ok(canConnect(nodes, edges, { source: 'b', target: 'mix', sourceHandle: 'out', targetHandle: 'in#2' }));
  assert.ok(!canConnect(nodes, edges, { source: 'a', target: 'mix', sourceHandle: 'out', targetHandle: 'in#2' }), 'no duplicate link');
  // a single input still takes one link only
  const gain = [...nodes, { id: 'g', spec: { id: 'g', type: 'gain', db: 0 } as const }];
  const one = [{ id: 'y', source: 'a', target: 'g', sourceHandle: 'out', targetHandle: 'in' }];
  assert.ok(!canConnect(gain, one, { source: 'b', target: 'g', sourceHandle: 'out', targetHandle: 'in' }));
});
