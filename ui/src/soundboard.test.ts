import assert from 'node:assert/strict';
import { test } from 'node:test';
import { KINDS_BY_CATEGORY, PORTS, canConnect, categoryOf, defaultKind, portsOf } from './graph.ts';
import {
  boardInGraph,
  cleanCacheMb,
  cleanCompressMb,
  formatBytes,
  formatDuration,
  nameFromFile,
  orphanSounds,
  soundsInGraph,
  newBoardName,
  padFor,
  padsUsing,
  partChange,
  withPad,
  withPadMoved,
  withPadRemoved,
  withPadsAdded,
  formatDb,
  importFraction,
  hasUsualSettings,
  usualPadSettings,
} from './soundboard.ts';
import type { Board, GraphSpec, NodeSpec, Pad, SoundInfo } from './types.ts';

const sound = (id: string, name: string): SoundInfo => ({ id, name, channels: 2, frames: 96_000, bytes: 384_000 });
const board = (id: string, pads: string[]): Board => ({ id, name: id, pads: pads.map((p) => ({ id: p, name: p, sound: null })) });

test('the soundboard block makes sound by itself and has one sound output', () => {
  assert.equal(categoryOf('soundboard'), 'generated');
  assert.ok(KINDS_BY_CATEGORY.generated.includes('soundboard'));
  assert.deepEqual(PORTS.soundboard.map((q) => `${q.dir}:${q.name}:${q.flow}`), ['out:out:audio']);
  assert.deepEqual(defaultKind('soundboard'), { type: 'soundboard', board: '' });
});

test('a soundboard can be linked to any sound input, and only out of its output', () => {
  const nodes = [
    { id: 'sb', spec: { id: 'sb', type: 'soundboard', board: 'b' } as const },
    { id: 'mic', spec: { id: 'mic', type: 'virtual_mic', name: 'x' } as const },
    { id: 'mx', spec: { id: 'mx', type: 'mixer' } as const },
  ];
  const link = (source: string, target: string, targetHandle: string) => canConnect(nodes, [], { source, target, sourceHandle: 'out', targetHandle });
  assert.equal(link('sb', 'mic', 'in'), true);
  assert.equal(link('sb', 'mx', 'in'), true);
  // nothing can feed a soundboard: it has no input
  assert.equal(canConnect(nodes, [], { source: 'mx', target: 'sb', sourceHandle: 'out', targetHandle: 'in' }), false);
});

test('lengths and sizes read well', () => {
  assert.equal(formatDuration(0), '0:00');
  assert.equal(formatDuration(48_000 * 65), '1:05');
  assert.equal(formatDuration(48_000 * 3725), '1:02:05');
  assert.equal(formatBytes(512), '512 B');
  assert.equal(formatBytes(2048), '2.0 KB');
  assert.equal(formatBytes(150 * 1024 * 1024), '150 MB');
  assert.equal(formatBytes(1536 * 1024 * 1024), '1.5 GB');
});

test('a pad is named after its file, without folders or extension, and not too long', () => {
  assert.equal(nameFromFile(String.raw`C:\Users\me\Music\Door bell.mp3`), 'Door bell');
  assert.equal(nameFromFile('/home/me/a.b.flac'), 'a.b');
  assert.equal(nameFromFile('.hidden'), '.hidden');
  assert.equal(nameFromFile('x'.repeat(100) + '.wav').length, 40);
  const pad = padFor(sound('s1', 'Clap.ogg'));
  assert.deepEqual({ ...pad, id: '' }, { id: '', name: 'Clap', sound: 's1', gain_db: 0, looped: false });
});

test('pads are added, changed, removed and moved without touching other boards', () => {
  const boards = [board('a', ['p1', 'p2', 'p3']), board('b', ['q1'])];
  assert.deepEqual(withPadRemoved(boards, 'a', 'p2')[0].pads.map((p) => p.id), ['p1', 'p3']);
  assert.deepEqual(withPadsAdded(boards, 'b', [padFor(sound('s', 'x.wav'))])[1].pads.length, 2);
  assert.equal(withPad(boards, 'a', 'p1', { name: 'Boom' })[0].pads[0].name, 'Boom');
  assert.deepEqual(withPadMoved(boards, 'a', 0, 2)[0].pads.map((p) => p.id), ['p2', 'p3', 'p1']);
  assert.deepEqual(withPadMoved(boards, 'a', 0, 9), boards, 'out of range changes nothing');
  assert.equal(withPadRemoved(boards, 'a', 'p2')[1], boards[1], 'the other board is the same object');
});

test('how many pads play a sound, and whether the graph has a block for a board', () => {
  const boards = [board('a', ['p1', 'p2']), board('b', ['q1'])];
  boards[0].pads[0].sound = 's';
  boards[1].pads[0].sound = 's';
  assert.equal(padsUsing(boards, 's'), 2);
  assert.equal(padsUsing(boards, 'zzz'), 0);
  const graph: GraphSpec = { nodes: [{ id: 'n', type: 'soundboard', board: 'a' }], edges: [] };
  assert.equal(boardInGraph(graph, 'a'), true);
  assert.equal(boardInGraph(graph, 'b'), false);
});

test('a new soundboard gets a name nobody has', () => {
  assert.equal(newBoardName([], 'Soundboard'), 'Soundboard 1');
  const taken = [board('a', []), board('b', [])];
  taken[0].name = 'Soundboard 3';
  // two boards exist: the next number is 3, which is taken, so it is 4
  assert.equal(newBoardName(taken, 'Soundboard'), 'Soundboard 4');
});

test('the part of a sound that plays: an end that is not after the start means to the end', () => {
  assert.deepEqual(partChange({}, 'start_ms', '1.5'), { start_ms: 1500, end_ms: 0 });
  assert.deepEqual(partChange({ start_ms: 1000 }, 'end_ms', '3'), { start_ms: 1000, end_ms: 3000 });
  assert.deepEqual(partChange({ start_ms: 1000 }, 'end_ms', '0.5'), { start_ms: 1000, end_ms: 0 }, 'ends before it starts');
  assert.deepEqual(partChange({ start_ms: 1000, end_ms: 3000 }, 'start_ms', '4'), { start_ms: 4000, end_ms: 0 }, 'start moved past the end');
  assert.deepEqual(partChange({}, 'start_ms', '-2'), { start_ms: 0, end_ms: 0 });
});

test('the size above which sounds are compressed: a whole number of megabytes, 0 means never', () => {
  assert.equal(cleanCompressMb('20'), 20);
  assert.equal(cleanCompressMb('0'), 0);
  assert.equal(cleanCompressMb('-1'), null);
  assert.equal(cleanCompressMb(''), null);
  assert.equal(cleanCompressMb('x'), null);
});

test('the memory limit is a whole number of megabytes, 16 at least', () => {
  assert.equal(cleanCacheMb('256'), 256);
  assert.equal(cleanCacheMb(' 64.4 '), 64);
  assert.equal(cleanCacheMb('15'), null);
  assert.equal(cleanCacheMb(''), null);
  assert.equal(cleanCacheMb('abc'), null);
});

test('a soundboard block only sends sound, and a block that plays one sound also sends how far it has got', () => {
  const names = (type: 'soundboard' | 'soundboard_sound') => portsOf({ type } as never).map((q) => `${q.dir}:${q.name}:${q.flow}`);
  assert.deepEqual(names('soundboard'), ['out:out:audio']);
  assert.deepEqual(names('soundboard_sound'), ['in:play:action', 'in:stop:action', 'out:out:audio', 'out:progress:float']);
  // the number can feed a block that takes numbers, a sound cannot
  const node = (spec: NodeSpec) => ({ id: spec.id, spec });
  const nodes = [node({ id: 'ss', type: 'soundboard_sound', board: 'b', pad: 'p' }), node({ id: 'm', type: 'math', op: 'add' }), node({ id: 'sb', type: 'soundboard', board: 'b' })];
  assert.equal(canConnect(nodes, [], { source: 'ss', target: 'm', sourceHandle: 'progress', targetHandle: 'a' }), true);
  assert.equal(canConnect(nodes, [], { source: 'ss', target: 'sb', sourceHandle: 'out', targetHandle: 'play1' }), false);
});

test('how far an import is: a fraction when the total is known, nothing when it is not', () => {
  assert.equal(importFraction(50, 200), 0.25);
  assert.equal(importFraction(300, 200), 1, 'never past the whole');
  assert.equal(importFraction(-5, 200), 0);
  assert.equal(importFraction(10, 0), null, 'the file does not say how long it is');
});

test('a volume is written with its sign', () => {
  assert.equal(formatDb(0), '0 dB');
  assert.equal(formatDb(-3), '−3 dB');
  assert.equal(formatDb(2.5), '+2.5 dB');
  assert.equal(formatDb(-0.04), '0 dB');
});

test('the sounds blocks play are found, in groups too', () => {
  const inner: GraphSpec = { nodes: [{ id: 'b', type: 'audio_file', sound: 's2' } as NodeSpec], edges: [] };
  const graph: GraphSpec = { nodes: [{ id: 'a', type: 'audio_file', sound: 's1' } as NodeSpec, { id: 'g', type: 'group', graph: inner, exposed: [] } as NodeSpec], edges: [] };
  assert.deepEqual([...soundsInGraph(graph)].sort(), ['s1', 's2']);
});

test('only the sounds nothing else uses are orphans', () => {
  const pads = [
    { id: 'p1', name: 'one', sound: 'a' },
    { id: 'p2', name: 'two', sound: 'b' },
    { id: 'p3', name: 'three', sound: 'c' },
    { id: 'p4', name: 'four', sound: 'a' },
  ];
  const left: Board[] = [{ id: 'other', name: 'other', pads: [{ id: 'x', name: 'x', sound: 'b' }] }];
  // a is used by no pad left, b by a pad left, c by a block
  assert.deepEqual(orphanSounds(pads, left, new Set(['c'])), ['a']);
});

test('the usual settings of a pad are put back, and the name, the file and the shortcut stay', () => {
  const pad: Pad = { id: 'p', name: 'Applause', sound: 's', key: 'F5', gain_db: -6, looped: true, exclusive: true, hold: true, fade_ms: 500, start_ms: 1000, end_ms: 4000, color: '#ff0000' };
  assert.equal(hasUsualSettings(pad), false);
  const back = { ...pad, ...usualPadSettings() };
  assert.equal(hasUsualSettings(back), true);
  assert.deepEqual([back.name, back.sound, back.key], ['Applause', 's', 'F5']);
  // a pad that was never touched has nothing to put back
  assert.equal(hasUsualSettings({ id: 'q', name: 'x' }), true);
});
