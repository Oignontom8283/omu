import assert from 'node:assert/strict';
import { test } from 'node:test';
import { getSnapshot, registerCommand, registerPanel, resetExtensions, subscribe } from './extensionRegistry.ts';

test('an extension adds a command and a panel, replaces its own, and takes them away', () => {
  resetExtensions();
  let told = 0;
  const stop = subscribe(() => told++);
  const remove = registerCommand({ id: 'ext.hello', title: 'Hello', run: () => undefined });
  registerCommand({ id: 'ext.hello', title: 'Hello again', run: () => undefined });
  assert.deepEqual(getSnapshot().commands.map((c) => c.title), ['Hello again'], 'the same id replaces');
  const closePanel = registerPanel({ id: 'ext.panel', title: 'Panel', mount: () => undefined });
  assert.equal(getSnapshot().panels.length, 1);
  assert.equal(told, 3, 'whoever shows them is told each time');
  remove(); // takes away the first registration, which was already replaced: the current command stays
  assert.equal(getSnapshot().commands.length, 1);
  closePanel();
  assert.equal(getSnapshot().panels.length, 0);
  stop();
  resetExtensions();
});
