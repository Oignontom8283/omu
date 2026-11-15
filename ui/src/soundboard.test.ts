import assert from 'node:assert/strict';
import { test } from 'node:test';
import { KINDS_BY_CATEGORY, PORTS, canConnect, categoryOf, defaultKind } from './graph.ts';
import {
  boardInGraph,
  cleanCacheMb,
  formatBytes,
  formatDuration,
  nameFromFile,
  newBoardName,
  padFor,
  padsUsing,
  partChange,
  withPad,
  withPadMoved,
  withPadRemoved,
  withPadsAdded,
} from './soundboard.ts';
import type { Board, GraphSpec, SoundInfo } from './types.ts';

const sound = (id: string, name: string): SoundInfo => ({ id, name, channels: 2, frames: 96_000, bytes: 384_000 });
const board = (id: string, pads: string[]): Board => ({ id, name: id, pads: pads.map((p) => ({ id: p, name: p, sound: null })) });

test('the soundboard block is an input with one sound output', () => {
  assert.equal(categoryOf('soundboard'), 'input');
  assert.ok(KINDS_BY_CATEGORY.input.includes('soundboard'));
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

test('the memory limit is a whole number of megabytes, 16 at least', () => {
  assert.equal(cleanCacheMb('256'), 256);
  assert.equal(cleanCacheMb(' 64.4 '), 64);
  assert.equal(cleanCacheMb('15'), null);
  assert.equal(cleanCacheMb(''), null);
  assert.equal(cleanCacheMb('abc'), null);
});
