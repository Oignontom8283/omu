import assert from 'node:assert/strict';
import { test } from 'node:test';
import {
  CATEGORIES,
  LIMITS,
  accepts,
  cropTo,
  defaultInput,
  inputValue,
  prunedInputs,
  KINDS_BY_CATEGORY,
  MAX_EXPRESSION_ARGS,
  RETIRED_KINDS,
  NODE_TYPES,
  PORTS,
  isGeneratedId,
  namedProblem,
  nameOf,
  newId,
  branchCount,
  entryPortOf,
  portsOf,
  zonesOf,
  canConnect,
  entryCount,
  entryHandles,
  layoutOf,
  canonical,
  categoryOf,
  keepLive,
  variableName,
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

test('the families hold every kind exactly once, and each kind has ports', () => {
  assert.deepEqual(CATEGORIES, ['input', 'generated', 'output', 'processing', 'condition', 'trigger', 'number', 'text', 'conversion', 'group']);
  assert.equal(new Set(NODE_TYPES).size, NODE_TYPES.length);
  assert.equal(NODE_TYPES.length + Object.keys(RETIRED_KINDS).length, Object.keys(PORTS).length);
  for (const t of NODE_TYPES) {
    assert.ok(PORTS[t].length > 0, t);
    assert.equal(defaultKind(t).type, t);
    assert.ok(KINDS_BY_CATEGORY[categoryOf(t)].includes(t));
  }
});

test('the families send and receive the flows they should', () => {
  const sends = (t: (typeof NODE_TYPES)[number]) => PORTS[t].filter((q) => q.dir === 'out').map((q) => q.flow);
  // the input that gives a setting of a block from a flow is not part of what a family receives
  const receives = (t: (typeof NODE_TYPES)[number]) => PORTS[t].filter((q) => q.dir === 'in' && !q.param).map((q) => q.flow);
  for (const t of KINDS_BY_CATEGORY.input) assert.deepEqual([sends(t), receives(t)], [['audio'], []]);
  // what makes sound by itself receives no sound: at most actions (to play it, to stop it)
  for (const t of KINDS_BY_CATEGORY.generated) {
    // the sound, and for the blocks that play one sound how far it has got
    assert.equal(sends(t)[0], 'audio', t);
    assert.ok(sends(t).slice(1).every((flow) => flow === 'float'), t);
    assert.ok(receives(t).every((flow) => flow === 'action'), t);
  }
  for (const t of KINDS_BY_CATEGORY.output) assert.deepEqual([sends(t), receives(t)], [[], ['audio']]);
  const only = (flows: string[], flow: string) => flows.length > 0 && flows.every((f) => f === flow);
  for (const t of KINDS_BY_CATEGORY.trigger) {
    assert.ok(only(sends(t), 'action'), `${t}: a trigger only sends actions`);
    assert.ok(receives(t).every((f) => f === 'condition'), `${t}: and receives at most a condition`);
  }
  for (const t of KINDS_BY_CATEGORY.condition) assert.ok(only(sends(t), 'condition'), t);
  for (const t of KINDS_BY_CATEGORY.processing) assert.ok(only(sends(t), 'audio'), t);
  for (const t of KINDS_BY_CATEGORY.number) assert.ok(sends(t).length > 0 && sends(t).every((f) => f === 'int' || f === 'float'), `${t}: a number block sends numbers`);
  for (const t of KINDS_BY_CATEGORY.text) assert.ok(only(sends(t), 'text'), `${t}: a text block sends text`);
  for (const t of KINDS_BY_CATEGORY.conversion) assert.ok(sends(t).length > 0 && sends(t).every((f) => f !== 'audio' && f !== 'action'), `${t}: a conversion sends a value`);
});

test('an input takes its own flow, a whole number where a decimal one is wanted, and numbers or yes/no where text is wanted', () => {
  const all = ['audio', 'condition', 'action', 'text', 'int', 'float'] as const;
  for (const f of all) assert.ok(accepts(f, f));
  assert.ok(accepts('float', 'int'));
  assert.ok(!accepts('int', 'float'));
  for (const f of ['int', 'float', 'condition'] as const) assert.ok(accepts('text', f), f);
  assert.ok(!accepts('text', 'audio') && !accepts('text', 'action') && !accepts('audio', 'text') && !accepts('condition', 'text'));
});

test('a setting that a flow can give has an input of its own, and a join has as many value inputs as it asks', () => {
  const params = Object.fromEntries(NODE_TYPES.map((t) => [t, PORTS[t].filter((q) => q.param).map((q) => `${q.name}:${q.flow}`)]).filter(([, v]) => (v as string[]).length > 0));
  assert.deepEqual(params, {
    gain: ['db:float'],
    level: ['threshold_db:float'],
    hold: ['ms:int'],
    after: ['ms:int'],
    timer: ['interval_ms:int'],
    join: ['template:text'],
    math_expr: ['expression:text'],
    cond_expr: ['expression:text'],
    text_expr: ['expression:text'],
    network_in: ['port:int', 'stream:text', 'from:text'],
    network_out: ['host:text', 'port:int', 'stream:text'],
    web_in: ['name:text'],
    web_out: ['name:text'],
    stream_in: ['url:text'],
    stream_out: ['url:text'],
  });
  const names = (args?: number) => portsOf({ type: 'join', args }).filter((q) => q.dir === 'in' && !q.param).map((q) => q.name);
  assert.deepEqual(names(3), ['arg1', 'arg2', 'arg3']);
  assert.deepEqual(names(0), ['arg1']);
  assert.equal(names(99).length, 16);
});

test('an expression block has as many inputs, named a, b, c..., as it asks, and a number or a condition for its result', () => {
  const names = (kind: Parameters<typeof portsOf>[0], dir: 'in' | 'out' = 'in') => portsOf(kind).filter((q) => q.dir === dir && !q.param).map((q) => q.name);
  assert.deepEqual(names({ type: 'math_expr', args: 3 }), ['a', 'b', 'c']);
  assert.deepEqual(names({ type: 'math_expr' }), ['a', 'b'], 'two by default');
  assert.deepEqual(names({ type: 'cond_expr', args: 0 }), ['a'], 'at least one');
  assert.equal(names({ type: 'math_expr', args: 99 }).length, MAX_EXPRESSION_ARGS);
  assert.equal(portsOf({ type: 'math_expr' }).find((q) => q.dir === 'out')?.flow, 'float');
  assert.equal(portsOf({ type: 'cond_expr' }).find((q) => q.dir === 'out')?.flow, 'condition');
  // the test of texts has text inputs and says yes or no
  assert.deepEqual(portsOf({ type: 'text_expr', args: 3 }).filter((q) => q.dir === 'in' && !q.param).map((q) => `${q.name}:${q.flow}`), ['a:text', 'b:text', 'c:text']);
  assert.equal(portsOf({ type: 'text_expr' }).find((q) => q.dir === 'out')?.flow, 'condition');
  assert.equal(categoryOf('text_compare'), 'condition');
  assert.equal(variableName(0), 'a');
  assert.equal(variableName(15), 'p');
  // the older blocks are not offered any more, but a graph that holds one keeps its family
  assert.equal(categoryOf('math'), 'number');
  assert.equal(categoryOf('compare'), 'condition');
  assert.ok(!NODE_TYPES.includes('math') && !NODE_TYPES.includes('compare'));
});

test('the blocks of the panel send an action, a yes/no and a number, and are changed while the graph runs', () => {
  assert.deepEqual(portsOf({ type: 'button' }).map((q) => `${q.name}:${q.dir}:${q.flow}`), ['pressed:out:action']);
  assert.deepEqual(portsOf({ type: 'switch' }).map((q) => `${q.name}:${q.dir}:${q.flow}`), ['out:out:condition']);
  assert.deepEqual(portsOf({ type: 'slider' }).map((q) => `${q.name}:${q.dir}:${q.flow}`), ['out:out:float']);
  assert.equal(categoryOf('button'), 'trigger');
  assert.equal(categoryOf('switch'), 'condition');
  assert.equal(categoryOf('slider'), 'number');
  // where a switch or a slider is set is not an edit: the graph is not "modified" when one moves
  const graph = (on: boolean, value: number): GraphSpec => ({ nodes: [{ id: 's', type: 'switch', on }, { id: 'v', type: 'slider', min: 0, max: 10, value }], edges: [] });
  assert.equal(canonical(graph(false, 1)), canonical(graph(true, 9)));
  assert.notEqual(canonical(graph(false, 1)), canonical({ nodes: [{ id: 's', type: 'switch', on: false }, { id: 'v', type: 'slider', min: 0, max: 20, value: 1 }], edges: [] }), 'its limits are');
  // and a step of the history keeps them where they are now
  assert.deepEqual(keepLive({ id: 'v', type: 'slider', min: 0, max: 10, value: 1 }, { id: 'v', type: 'slider', min: 0, max: 10, value: 7 }), { id: 'v', type: 'slider', min: 0, max: 10, value: 7 });
  assert.deepEqual(keepLive({ id: 's', type: 'switch', on: false }, { id: 's', type: 'switch', on: true }), { id: 's', type: 'switch', on: true });
  assert.deepEqual(defaultKind('counter'), { type: 'counter', initial: 0, up_by: 1, down_by: 1 });
});

test('a branch draws one orange zone over its conditions and the else; a select one per sound and its condition', () => {
  assert.deepEqual(zonesOf({ type: 'branch', branches: 1 }), [{ start: 1, rows: 2 }]);
  assert.deepEqual(zonesOf({ type: 'branch', branches: 3 }), [{ start: 1, rows: 4 }]);
  assert.deepEqual(zonesOf({ type: 'select', branches: 2 }), [{ start: 0, rows: 2 }, { start: 2, rows: 2 }, { start: 4, rows: 1 }]);
  assert.deepEqual(zonesOf({ type: 'gain' }), []);
  // a zone never reaches past the rows the block draws: left column of a select, right column of a branch
  const rowsOfLeft = (kind: Parameters<typeof portsOf>[0]) => portsOf(kind).filter((q) => q.dir === 'in').length;
  const select = { type: 'select', branches: 3 } as const;
  const last = zonesOf(select).at(-1)!;
  assert.equal(last.start + last.rows, rowsOfLeft(select));
  const branch = { type: 'branch', branches: 3 } as const;
  const rightRows = portsOf(branch).filter((q) => q.dir === 'out').length + 1; // the gap row
  const z = zonesOf(branch)[0]!;
  assert.equal(z.start + z.rows, rightRows);
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
  // the letter is the layout's, not the key's place: AZERTY puts Z where QWERTY has W
  assert.equal(comboFromEvent(ev('z', 'KeyW', { ctrlKey: true })), 'Ctrl+Z');
  assert.equal(comboFromEvent(ev('w', 'KeyZ', { ctrlKey: true })), 'Ctrl+W');
  assert.equal(comboFromEvent(ev('я', 'KeyZ', { ctrlKey: true })), 'Ctrl+Z', 'no Latin letter on the layout: the place of the key');
  // the operators of the keypad, the keys of punctuation (the same place whatever they write) and the locks
  assert.equal(comboFromEvent(ev('+', 'NumpadAdd')), 'NumAdd');
  assert.equal(comboFromEvent(ev('*', 'NumpadMultiply', { ctrlKey: true })), 'Ctrl+NumMultiply');
  assert.equal(comboFromEvent(ev(';', 'Semicolon')), 'Semicolon');
  assert.equal(comboFromEvent(ev('m', 'Semicolon')), 'M', 'a letter on this layout is a letter');
  assert.equal(comboFromEvent(ev(')', 'Minus', { altKey: true })), 'Alt+Minus');
  assert.equal(comboFromEvent(ev('CapsLock', 'CapsLock')), 'CapsLock');
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

test('every input that is not a sound, an action or a collector carries a value typed next to it', () => {
  for (const t of NODE_TYPES) {
    for (const q of PORTS[t]) {
      const wanted = q.dir === 'in' && ['text', 'int', 'float'].includes(q.flow) && !q.multi && !q.entries;
      assert.equal(q.inline === true, wanted, `${t}.${q.name}`);
    }
  }
  const math = portsOf(defaultKind('math'));
  assert.deepEqual(math.filter((q) => q.inline).map((q) => q.name), ['a', 'b']);
  assert.ok(portsOf(defaultKind('gain')).find((q) => q.name === 'db')?.field === 'db', 'a setting of the kind: its field is named');
  assert.equal(portsOf(defaultKind('math')).find((q) => q.name === 'a')?.field, undefined, 'a typed value is kept with the block');
});

test('a number is cropped to the limits of its setting, and the slider stays inside them', () => {
  const gain = LIMITS.db;
  assert.equal(cropTo(gain, 1000), 40);
  assert.equal(cropTo(gain, -1000), -120);
  assert.equal(cropTo(gain, 3), 3);
  assert.equal(cropTo(gain, Number.NaN), -120);
  for (const l of Object.values(LIMITS)) assert.ok(l.softMin >= l.min && l.softMax <= l.max && l.softMax >= l.softMin);
  // every setting a flow can give that is a number has limits
  for (const t of NODE_TYPES) for (const q of PORTS[t].filter((x) => x.param && x.flow !== 'text')) assert.ok(q.limits, `${t}.${q.name}`);
});

test('the value of an input is the setting of the kind, or what was typed for it, or a default', () => {
  const gain = { id: 'g', type: 'gain', db: -6 } as const;
  const db = portsOf(gain).find((q) => q.name === 'db')!;
  assert.equal(inputValue(gain, db), -6);
  const math = { id: 'm', type: 'math', op: 'add', inputs: { a: 2.5 } } as const;
  const [a, b] = portsOf(math).filter((q) => q.inline);
  assert.equal(inputValue(math, a), 2.5);
  assert.equal(inputValue(math, b), 0, 'nothing typed: zero');
  assert.equal(defaultInput(portsOf(defaultKind('text_compare'))[0]), '', 'text starts empty');
});

test('the values typed in a block stay only for the inputs it still has', () => {
  const join = { id: 'j', type: 'join', template: '$arg1', args: 3, inputs: { arg1: 'a', arg3: 'c', gone: 'x' } } as const;
  assert.deepEqual(prunedInputs(join).inputs, { arg1: 'a', arg3: 'c' });
  const fewer = { ...join, args: 2 };
  assert.deepEqual(prunedInputs(fewer).inputs, { arg1: 'a' });
  assert.equal(prunedInputs({ id: 'j', type: 'join', template: '', args: 1, inputs: { zzz: 1 } }).inputs, undefined, 'nothing left: nothing kept');
});

test('a block that is no longer offered still keeps its family and its ports for the graphs that hold it', () => {
  assert.ok(!NODE_TYPES.includes('logic'));
  assert.equal(categoryOf('logic'), 'condition');
  assert.ok(portsOf(defaultKind('logic')).length > 0);
});
