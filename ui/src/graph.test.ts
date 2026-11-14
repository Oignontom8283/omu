import assert from 'node:assert/strict';
import { test } from 'node:test';
import {
  CATEGORIES,
  KINDS_BY_CATEGORY,
  NODE_TYPES,
  PORTS,
  isGeneratedId,
  namedProblem,
  nameOf,
  newId,
  branchCount,
  entryPortOf,
  portsOf,
  canConnect,
  entryCount,
  entryHandles,
  layoutOf,
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
  const only = (flows: string[], flow: string) => flows.length > 0 && flows.every((f) => f === flow);
  for (const t of KINDS_BY_CATEGORY.trigger) {
    assert.ok(only(sends(t), 'action'), `${t}: a trigger only sends actions`);
    assert.ok(receives(t).every((f) => f === 'condition'), `${t}: and receives at most a condition`);
  }
  for (const t of KINDS_BY_CATEGORY.condition) assert.ok(only(sends(t), 'condition'), t);
  for (const t of KINDS_BY_CATEGORY.processing) assert.ok(only(sends(t), 'audio'), t);
});

test('a branch has a condition per branch and a sound output per branch plus an else; a select the other way round', () => {
  const names = (kind: Parameters<typeof portsOf>[0], dir: 'in' | 'out' = 'in') => portsOf(kind).filter((q) => q.dir === dir).map((q) => q.name);
  assert.deepEqual(names({ type: 'branch', branches: 1 }), ['in', 'when1']);
  assert.deepEqual(names({ type: 'branch', branches: 1 }, 'out'), ['out1', 'else']);
  assert.deepEqual(names({ type: 'branch', branches: 3 }), ['in', 'when1', 'when2', 'when3']);
  assert.deepEqual(names({ type: 'branch', branches: 3 }, 'out'), ['out1', 'out2', 'out3', 'else']);
  assert.deepEqual(names({ type: 'select', branches: 2 }), ['in1', 'when1', 'in2', 'when2', 'else']);
  assert.deepEqual(names({ type: 'select', branches: 2 }, 'out'), ['out']);
  // the same names as the daemon's, whatever the order they are drawn in
  assert.deepEqual([...names({ type: 'select', branches: 2 })].sort(), ['else', 'in1', 'in2', 'when1', 'when2']);
  // nonsense counts are brought back to something sensible
  assert.equal(branchCount({ type: 'branch', branches: 0 }), 1);
  assert.equal(branchCount({ type: 'branch', branches: 999 }), 16);
  assert.equal(branchCount({ type: 'branch' }), 1);
  // the first condition is the "if", the others "else if" with their number; outputs are numbered the same way
  const labels = portsOf({ type: 'branch', branches: 2 }).map((q) => `${q.label}${q.n ?? ''}`);
  assert.deepEqual(labels, ['port.in', 'port.if', 'port.elif2', 'port.then1', 'port.then2', 'port.else']);
  assert.equal(portsOf({ type: 'branch', branches: 2 }).filter((q) => q.gapBefore).length, 1, 'one gap so that if lines up with then');
});

test('conditions that combine can show several input points like the mixer; the others take one link', () => {
  for (const type of ['all', 'any'] as const) {
    const port = entryPortOf({ type });
    assert.equal(port?.flow, 'condition');
    assert.equal(port?.name, 'in');
  }
  assert.equal(entryPortOf({ type: 'mixer' })?.flow, 'audio');
  for (const type of ['not', 'hold', 'after', 'change', 'gain', 'variable', 'branch'] as const) assert.equal(entryPortOf(defaultKind(type)), undefined, type);
  // three conditions into `all`, on two points: the daemon sees three links into `in`
  const g: GraphSpec = {
    nodes: [
      { id: 'a', type: 'variable', initial: true },
      { id: 'b', type: 'variable', initial: true },
      { id: 'c', type: 'variable', initial: true },
      { id: 'all', type: 'all' },
    ],
    edges: ['a', 'b', 'c'].map((n) => ({ from: { node: n, port: 'value' }, to: { node: 'all', port: 'in' } })),
  };
  const flow = toFlow(g, { all: { x: 0, y: 0, inputs: 2, slots: { 'a.value->all.in': 1, 'b.value->all.in': 1, 'c.value->all.in': 2 } } });
  assert.deepEqual(flow.edges.map((e) => e.targetHandle), ['in', 'in', 'in#2']);
  assert.equal(flow.nodes.find((n) => n.id === 'all')?.inputs, 2);
  assert.deepEqual(fromFlow(flow.nodes, flow.edges), g);
});

test('connecting into the numbered ports of a branch', () => {
  const nodes = [
    { id: 'src', spec: { id: 'src', type: 'app_capture', target: { pid: 1 } } as const },
    { id: 'v', spec: { id: 'v', type: 'variable', initial: true } as const },
    { id: 'br', spec: { id: 'br', type: 'branch', branches: 2 } as const },
    { id: 'mic', spec: { id: 'mic', type: 'virtual_mic', name: 'M' } as const },
  ];
  const can = (source: string, sh: string, target: string, th: string) => canConnect(nodes, [], { source, target, sourceHandle: sh, targetHandle: th });
  assert.ok(can('src', 'out', 'br', 'in'));
  assert.ok(can('v', 'value', 'br', 'when1'));
  assert.ok(can('v', 'value', 'br', 'when2'));
  assert.ok(!can('v', 'value', 'br', 'when3'), 'only two conditions on this block');
  assert.ok(!can('src', 'out', 'br', 'when1'), 'a sound is not a condition');
  assert.ok(can('br', 'out2', 'mic', 'in'));
  assert.ok(can('br', 'else', 'mic', 'in'));
  assert.ok(!can('br', 'out3', 'mic', 'in'));
  assert.ok(!can('v', 'value', 'br', 'out1'), 'an output is not an input');
});

test('changing a block into a branching one keeps the links that still fit', () => {
  const nodes = [
    { id: 'src', spec: { id: 'src', type: 'app_capture', target: { pid: 1 } } as const },
    { id: 'g', spec: { id: 'g', type: 'gain', db: 0 } as const },
    { id: 'mic', spec: { id: 'mic', type: 'virtual_mic', name: 'M' } as const },
  ];
  const edges = toFlow(
    { nodes: nodes.map((n) => n.spec), edges: [{ from: { node: 'src', port: 'out' }, to: { node: 'g', port: 'in' } }, { from: { node: 'g', port: 'out' }, to: { node: 'mic', port: 'in' } }] },
    {},
  ).edges;
  const asBranch = edgesAfterKindChange(nodes, edges, 'g', 'branch');
  assert.deepEqual(asBranch.map((e) => `${e.sourceHandle}->${e.targetHandle}`), ['out->in', 'out1->in'], 'the output moves to the first sound output');
  const asNot = edgesAfterKindChange(nodes, edges, 'g', 'not');
  assert.equal(asNot.length, 0, 'sounds cannot enter a condition block');
});

test('connection rules: flows must match, outputs go to inputs, one edge per condition input', () => {
  const flow = toFlow(gated, {});
  const link = (source: string, target: string, sh: string, th: string) => canConnect(flow.nodes, flow.edges, { source, target, sourceHandle: sh, targetHandle: th });
  assert.equal(link('key', 'gate', 'pressed', 'open'), false, 'an action cannot open a gate: it needs a condition');
  assert.equal(link('app', 'on', 'out', 'toggle'), false, 'audio is not an action');
  assert.equal(link('on', 'gate', 'value', 'open'), false, 'duplicate');
  assert.equal(link('mic', 'app', 'in', 'out'), false, 'a virtual microphone sends nothing');
  assert.equal(link('gate', 'gate', 'out', 'in'), false, 'no loop on one block');
  assert.equal(link('app', 'mic', 'out', 'in'), true, 'a second sound into the microphone is allowed: the sounds are added');
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

const mixerOf = (n: number): GraphSpec => ({
  nodes: [
    ...Array.from({ length: n }, (_, i) => ({ id: `s${i + 1}`, type: 'app_capture' as const, target: { pid: i + 1 } })),
    { id: 'mix', type: 'mixer' },
  ],
  edges: Array.from({ length: n }, (_, i) => ({ from: { node: `s${i + 1}`, port: 'out' }, to: { node: 'mix', port: 'in' } })),
});

test('a mixer starts with two input points, and links without a saved place take the first point with no link', () => {
  const empty = toFlow(mixerOf(0), {});
  assert.equal(empty.nodes.find((n) => n.id === 'mix')?.inputs, 2);
  assert.deepEqual(entryHandles('in', 2), ['in', 'in#2']);

  const g = mixerOf(3);
  const flow = toFlow(g, {});
  assert.deepEqual(flow.edges.map((e) => e.targetHandle), ['in', 'in#2', 'in#3']);
  assert.equal(flow.nodes.find((n) => n.id === 'mix')?.inputs, 3, 'a point for every link at least');
  assert.deepEqual(fromFlow(flow.nodes, flow.edges), g, 'the daemon only ever sees `in`');
  // only the sound input of the mixer has points to add; a variable's actions are one point that takes many links
  assert.equal(toFlow(g, {}).nodes.find((n) => n.id === 's1')?.inputs, undefined);
});

test('several links can stay on one point: the saved places are kept', () => {
  const g = mixerOf(3);
  const layout = {
    mix: { x: 0, y: 0, inputs: 2, slots: { 's1.out->mix.in': 1, 's2.out->mix.in': 1, 's3.out->mix.in': 2 } },
  };
  const flow = toFlow(g, layout);
  assert.deepEqual(flow.edges.map((e) => e.targetHandle), ['in', 'in', 'in#2']);
  assert.equal(flow.nodes.find((n) => n.id === 'mix')?.inputs, 2);
  assert.deepEqual(fromFlow(flow.nodes, flow.edges), g, 'still three links into `in` for the daemon');
  // what is saved is what is shown
  const saved = layoutOf(flow.nodes, flow.edges);
  assert.equal(saved.mix.inputs, 2);
  assert.deepEqual(saved.mix.slots, layout.mix.slots);
  assert.deepEqual(toFlow(g, saved).edges, flow.edges);
});

test('a new link goes to the first free point, the rest of the saved places stay', () => {
  const g = mixerOf(3);
  const layout = { mix: { x: 0, y: 0, inputs: 3, slots: { 's1.out->mix.in': 2, 's3.out->mix.in': 3 } } };
  // s2 has no saved place (made from the command line): the only point without a link is the first
  assert.deepEqual(toFlow(g, layout).edges.map((e) => e.targetHandle), ['in#2', 'in', 'in#3']);
});

test('a saved place that makes no sense is ignored, and the points never go below the links', () => {
  const g = mixerOf(2);
  const layout = { mix: { x: 0, y: 0, inputs: 1, slots: { 's1.out->mix.in': 0, 's2.out->mix.in': 5 } } };
  const flow = toFlow(g, layout);
  assert.deepEqual(flow.edges.map((e) => e.targetHandle), ['in', 'in#5']);
  assert.equal(flow.nodes.find((n) => n.id === 'mix')?.inputs, 5, 'the point with a link is always there');
  assert.equal(entryCount([], 'mix', 'in', undefined), 2);
  assert.equal(entryCount([], 'mix', 'in', 0), 1, 'at least one');
  assert.equal(entryCount([], 'mix', 'in', 999), 32, 'not unlimited');
  assert.equal(entryCount([{ target: 'other', targetHandle: 'in#9' }], 'mix', 'in', undefined), 2, 'another block does not count');
});

test('connecting into a point of the mixer works, the same source twice does not', () => {
  const nodes = [
    { id: 'a', spec: { id: 'a', type: 'app_capture', target: { pid: 1 } } as const },
    { id: 'b', spec: { id: 'b', type: 'app_capture', target: { pid: 2 } } as const },
    { id: 'mix', spec: { id: 'mix', type: 'mixer' } as const },
  ];
  const edges = [{ id: 'x', source: 'a', target: 'mix', sourceHandle: 'out', targetHandle: 'in' }];
  assert.ok(canConnect(nodes, edges, { source: 'b', target: 'mix', sourceHandle: 'out', targetHandle: 'in#2' }));
  assert.ok(!canConnect(nodes, edges, { source: 'a', target: 'mix', sourceHandle: 'out', targetHandle: 'in#2' }), 'no duplicate link');
  // any sound input adds the sounds that reach it, a gain's too
  const gain = [...nodes, { id: 'g', spec: { id: 'g', type: 'gain', db: 0 } as const }];
  const one = [{ id: 'y', source: 'a', target: 'g', sourceHandle: 'out', targetHandle: 'in' }];
  assert.ok(canConnect(gain, one, { source: 'b', target: 'g', sourceHandle: 'out', targetHandle: 'in' }));
});

test('a new block gets a random identifier, never the same twice, and a hand-made one is left alone', () => {
  const ids = Array.from({ length: 200 }, () => newId());
  assert.equal(new Set(ids).size, 200);
  for (const id of ids) {
    assert.match(id, /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/);
    assert.ok(isGeneratedId(id));
    assert.match(id, /^[A-Za-z0-9._-]+$/, 'accepted by the daemon as an identifier');
  }
  assert.ok(!isGeneratedId('mic'));
  assert.ok(!isGeneratedId('output_discord'));
});

test('the name people see is the display name; a chosen identifier stands in for it, a random one does not', () => {
  assert.equal(nameOf({ id: 'x', display: 'Micro du stream' }), 'Micro du stream');
  assert.equal(nameOf({ id: 'mon_micro' }), 'mon_micro');
  assert.equal(nameOf({ id: newId() }), '');
  assert.equal(nameOf({ id: 'a', display: '   ' }), 'a', 'a blank name is no name');
  const id = newId();
  const nodes = [{ id, display: 'Micro' }, { id: newId() }, { id: 'out', display: 'Sortie' }];
  assert.equal(namedProblem(`nodes "${id}" and "out" use the same device`, nodes, '(?)'), 'nodes "Micro" and "Sortie" use the same device');
  assert.equal(namedProblem(`node "${nodes[1].id}" has no port`, nodes, '(?)'), 'node "(?)" has no port');
  assert.equal(namedProblem('nothing to rename', nodes, '(?)'), 'nothing to rename');
});

test('the display name travels with the block and a graph without one stays as it is', () => {
  const g: GraphSpec = {
    nodes: [
      { id: 'app', type: 'app_capture', target: { pid: 1 } },
      { id: 'c3a4', display: "Mon micro : l'original", type: 'virtual_mic', name: 'Stream' },
    ],
    edges: [{ from: { node: 'app', port: 'out' }, to: { node: 'c3a4', port: 'in' } }],
  };
  const flow = toFlow(g, {});
  assert.deepEqual(fromFlow(flow.nodes, flow.edges), g);
  assert.equal('display' in fromFlow(flow.nodes, flow.edges).nodes[0], false, 'no empty property is invented');
});

test('every sound input takes several links; a condition input only when it combines them', () => {
  const nodes = [
    { id: 'a', spec: { id: 'a', type: 'app_capture', target: { pid: 1 } } as const },
    { id: 'b', spec: { id: 'b', type: 'app_capture', target: { pid: 2 } } as const },
    { id: 'v', spec: { id: 'v', type: 'variable', initial: true } as const },
    { id: 'w', spec: { id: 'w', type: 'variable', initial: true } as const },
    { id: 'mic', spec: { id: 'mic', type: 'virtual_mic', name: 'M' } as const },
    { id: 'gate', spec: { id: 'gate', type: 'gate' } as const },
    { id: 'all', spec: { id: 'all', type: 'all' } as const },
  ];
  const edges = [
    { id: '1', source: 'a', target: 'mic', sourceHandle: 'out', targetHandle: 'in' },
    { id: '2', source: 'v', target: 'gate', sourceHandle: 'value', targetHandle: 'open' },
    { id: '3', source: 'v', target: 'all', sourceHandle: 'value', targetHandle: 'in' },
  ];
  const can = (source: string, sh: string, target: string, th: string) => canConnect(nodes, edges, { source, target, sourceHandle: sh, targetHandle: th });
  assert.ok(can('b', 'out', 'mic', 'in'), 'a second sound into a microphone: they are added');
  assert.ok(!can('a', 'out', 'mic', 'in'), 'but the same one twice is a mistake');
  assert.ok(!can('w', 'value', 'gate', 'open'), 'two conditions on one gate input would be ambiguous');
  assert.ok(can('w', 'value', 'all', 'in'), 'unless the block combines them');
  // the + for extra input points stays where it was designed: the mixer, `all` and `any`
  for (const type of ['gain', 'virtual_mic', 'gate', 'branch', 'delay'] as const) assert.equal(entryPortOf(defaultKind(type)), undefined, type);
  assert.equal(entryPortOf({ type: 'mixer' })?.name, 'in');
});
