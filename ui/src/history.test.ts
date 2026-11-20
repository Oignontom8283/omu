import assert from 'node:assert/strict';
import { test } from 'node:test';
import { History } from './history.ts';

const make = (capacity?: number) => new History<{ n: number }>((s) => String(s.n), capacity);

test('steps go back and forth', () => {
  const h = make();
  h.record({ n: 1 });
  h.record({ n: 2 });
  h.record({ n: 3 });
  assert.deepEqual(h.undo(), { n: 2 });
  assert.deepEqual(h.undo(), { n: 1 });
  assert.equal(h.undo(), null, 'nothing before the start');
  assert.deepEqual(h.redo(), { n: 2 });
  assert.deepEqual(h.redo(), { n: 3 });
  assert.equal(h.redo(), null);
});

test('the same state twice is one step, and what the editor shows after an undo is not a new step', () => {
  const h = make();
  assert.equal(h.record({ n: 1 }), true);
  assert.equal(h.record({ n: 1 }), false);
  h.record({ n: 2 });
  const back = h.undo()!;
  assert.equal(h.record(back), false, 'the editor now shows the state that was brought back');
  assert.equal(h.canRedo, true, 'and the way forward is still there');
});

test('doing something after going back ends the way forward', () => {
  const h = make();
  h.record({ n: 1 });
  h.record({ n: 2 });
  h.undo();
  h.record({ n: 9 });
  assert.equal(h.canRedo, false);
  assert.deepEqual(h.undo(), { n: 1 });
});

test('the oldest steps are forgotten past the capacity, and a reset forgets all', () => {
  const h = make(3);
  for (let i = 1; i <= 6; i++) h.record({ n: i });
  let steps = 0;
  while (h.undo()) steps++;
  assert.equal(steps, 3);
  h.reset({ n: 0 });
  assert.equal(h.canUndo, false);
  assert.equal(h.canRedo, false);
});
