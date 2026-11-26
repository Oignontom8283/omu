import { test } from 'node:test';
import assert from 'node:assert/strict';
import { clickRow, keepExisting, moveCursor, movedIndex, NOTHING, selectAll } from './selectionModel.ts';

const keys = ['a', 'b', 'c', 'd', 'e'];

test('a click chooses one row and sets the anchor', () => {
  assert.deepEqual(clickRow(keys, NOTHING, 'c'), { selected: ['c'], focus: 'c', anchor: 'c' });
});

test('Ctrl adds and takes away, in the order of the list', () => {
  let s = clickRow(keys, NOTHING, 'd');
  s = clickRow(keys, s, 'b', { ctrl: true });
  assert.deepEqual(s.selected, ['b', 'd']);
  s = clickRow(keys, s, 'd', { ctrl: true });
  assert.deepEqual(s.selected, ['b']);
});

test('Shift chooses the range from the anchor, either way', () => {
  let s = clickRow(keys, NOTHING, 'b');
  s = clickRow(keys, s, 'd', { shift: true });
  assert.deepEqual(s.selected, ['b', 'c', 'd']);
  s = clickRow(keys, s, 'a', { shift: true });
  assert.deepEqual(s.selected, ['a', 'b']);
  assert.equal(s.anchor, 'b');
});

test('Ctrl+Shift adds a range to what was chosen', () => {
  let s = clickRow(keys, NOTHING, 'a');
  s = clickRow(keys, s, 'd', { ctrl: true });
  s = clickRow(keys, s, 'e', { ctrl: true, shift: true });
  assert.deepEqual(s.selected, ['a', 'd', 'e']);
});

test('the arrows move the cursor, Shift grows the range, the ends are kept', () => {
  let s = moveCursor(keys, NOTHING, 1);
  assert.deepEqual(s.selected, ['a']);
  s = moveCursor(keys, s, 1, { shift: true });
  s = moveCursor(keys, s, 1, { shift: true });
  assert.deepEqual(s.selected, ['a', 'b', 'c']);
  s = moveCursor(keys, s, Infinity);
  assert.deepEqual(s.selected, ['e']);
  s = moveCursor(keys, s, 10);
  assert.equal(s.focus, 'e');
  s = moveCursor(keys, s, -Infinity, { ctrl: true });
  assert.deepEqual(s, { selected: ['e'], focus: 'a', anchor: 'e' });
  assert.deepEqual(moveCursor([], s, 1), NOTHING);
});

test('up from nothing starts at the last row', () => {
  assert.equal(moveCursor(keys, NOTHING, -1).focus, 'e');
});

test('select all, and rows that are gone leave the selection', () => {
  const all = selectAll(keys, NOTHING);
  assert.deepEqual(all.selected, keys);
  const kept = keepExisting(['a', 'c'], { selected: ['a', 'b', 'c'], focus: 'b', anchor: 'a' });
  assert.deepEqual(kept, { selected: ['a', 'c'], focus: null, anchor: 'a' });
  const same = { selected: ['a'], focus: 'a', anchor: 'a' };
  assert.equal(keepExisting(keys, same), same);
});

test('a moved item stays inside the list', () => {
  assert.equal(movedIndex(5, 0, -1), 0);
  assert.equal(movedIndex(5, 4, 1), 4);
  assert.equal(movedIndex(5, 2, 1), 3);
});
