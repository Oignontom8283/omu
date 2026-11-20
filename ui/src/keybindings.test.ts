import assert from 'node:assert/strict';
import { test } from 'node:test';
import { COMMANDS, bindings, commandOf, commandsFor } from './keybindings.ts';

const key = (code: string, extra: Partial<{ key: string; ctrlKey: boolean; shiftKey: boolean; altKey: boolean }> = {}) => ({ key: code.replace('Key', '').toLowerCase(), code, ctrlKey: false, altKey: false, shiftKey: false, metaKey: false, ...extra });

test('every command has a shortcut as shipped, and no two share one', () => {
  const given = COMMANDS.filter((c) => c.keys !== '').map((c) => c.keys.toLowerCase());
  assert.equal(new Set(given).size, given.length);
});

test('a key press finds its command', () => {
  const map = bindings({});
  assert.equal(commandOf(key('KeyZ', { ctrlKey: true }), map), 'edit.undo');
  assert.equal(commandOf(key('KeyP', { ctrlKey: true, shiftKey: true }), map), 'palette.openAlt');
  assert.equal(commandOf({ ...key('F1'), key: 'F1' }, map), 'palette.open');
  assert.equal(commandOf(key('KeyQ'), map), null, 'a letter nobody asked for');
});

test('what the person changed wins, an empty text takes a shortcut away', () => {
  const map = bindings({ 'edit.undo': 'Ctrl+Alt+U', 'edit.redo': '' });
  assert.equal(commandOf(key('KeyU', { ctrlKey: true, altKey: true }), map), 'edit.undo');
  assert.equal(commandOf(key('KeyZ', { ctrlKey: true }), map), null, 'the old shortcut is free');
  assert.deepEqual(commandsFor('', map), [], 'no command has no shortcut');
});

test('two commands on one shortcut are reported', () => {
  const map = bindings({ 'edit.copy': 'Ctrl+V' });
  assert.deepEqual(commandsFor('Ctrl+V', map), ['edit.copy', 'edit.paste']);
});
