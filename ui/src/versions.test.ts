import assert from 'node:assert/strict';
import { test } from 'node:test';
import { KEPT, differences, same, withVersion, type Version } from './versions.ts';
import type { GraphSpec } from './types.ts';

const graph = (...ids: string[]): GraphSpec => ({ nodes: ids.map((id) => ({ id, type: 'mixer' as const })), edges: [] });
const version = (at: string, g: GraphSpec): Version => ({ at, graph: g, layout: {} });

test('a version is added in front, and the same graph twice is kept once', () => {
  const first = withVersion([], version('1', graph('a')));
  assert.equal(withVersion(first, version('2', graph('a'))).length, 1);
  const two = withVersion(first, version('3', graph('a', 'b')));
  assert.deepEqual(two.map((v) => v.at), ['3', '1']);
});

test('only the newest versions are kept', () => {
  let list: Version[] = [];
  for (let i = 0; i < KEPT + 5; i++) list = withVersion(list, version(String(i), graph(`n${i}`)));
  assert.equal(list.length, KEPT);
  assert.equal(list[0].at, String(KEPT + 4));
});

test('the differences name the blocks that are new, gone or changed, and count the links', () => {
  const before: GraphSpec = { nodes: [{ id: 'a', type: 'gain', db: 0 }, { id: 'b', type: 'mixer' }], edges: [{ from: { node: 'a', port: 'out' }, to: { node: 'b', port: 'in' } }] };
  const after: GraphSpec = { nodes: [{ id: 'a', type: 'gain', db: -6 }, { id: 'c', type: 'mixer' }], edges: [] };
  assert.deepEqual(differences(before, after), { added: ['c'], removed: ['b'], changed: ['a'], links: 1 });
  assert.equal(same(differences(before, before)), true);
});
