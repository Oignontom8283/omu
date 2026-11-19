import assert from 'node:assert/strict';
import { test } from 'node:test';
import { fuzzyMatch, rank } from './fuzzy.ts';

test('the letters have to appear in order, not side by side', () => {
  assert.ok(fuzzyMatch('mxr', 'Mixer'));
  assert.equal(fuzzyMatch('xm', 'Mixer'), null);
  assert.equal(fuzzyMatch('z', 'Mixer'), null);
});

test('case and accents do not matter, positions are those of the shown text', () => {
  const m = fuzzyMatch('reg', 'Réglages');
  assert.deepEqual(m?.indices, [0, 1, 2]);
  assert.ok(fuzzyMatch('REGLAGES', 'Réglages'));
});

test('an empty query matches everything', () => {
  assert.deepEqual(fuzzyMatch('', 'anything'), { score: 0, indices: [] });
});

test('the best match comes first: whole word, then start of a word, then scattered letters', () => {
  const items = ['Delay', 'Noise reduction', 'Gain', 'Gate'];
  assert.deepEqual(
    rank('gain', items, (s) => s).map((r) => r.item),
    ['Gain'],
  );
  assert.equal(rank('ga', items, (s) => s)[0].item, 'Gain');
  assert.deepEqual(
    rank('nr', items, (s) => s).map((r) => r.item),
    ['Noise reduction'],
  );
  assert.deepEqual(
    rank('a', ['Banana', 'Apple', 'Gate'], (s) => s).map((r) => r.item),
    ['Apple', 'Gate', 'Banana'],
  );
});

test('ties keep the given order', () => {
  assert.deepEqual(
    rank('', ['b', 'a', 'c'], (s) => s).map((r) => r.item),
    ['b', 'a', 'c'],
  );
});
