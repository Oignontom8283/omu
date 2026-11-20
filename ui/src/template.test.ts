import assert from 'node:assert/strict';
import { test } from 'node:test';
import { tokenizeTemplate, type TemplateToken } from './template.ts';

const known = (name: string) => /^(arg)?[12]$/.test(name);
const kinds = (source: string): [string, TemplateToken['kind']][] => tokenizeTemplate(source, known).map((t) => [t.text, t.kind]);

test('plain text stays text, joined into one piece', () => {
  assert.deepEqual(kinds('Hello there'), [['Hello there', 'text']]);
  assert.deepEqual(kinds(''), []);
});

test('a value is a dollar sign and a name, whether short or braced', () => {
  assert.deepEqual(kinds('hi $arg1!'), [['hi ', 'text'], ['$', 'dollar'], ['arg1', 'variable'], ['!', 'text']]);
  assert.deepEqual(kinds('${arg2}x'), [['${', 'dollar'], ['arg2', 'variable'], ['}', 'dollar'], ['x', 'text']]);
  assert.deepEqual(kinds('$1'), [['$', 'dollar'], ['1', 'variable']]);
});

test('a name that is no value of the block is an error, unless the template gives it a default', () => {
  assert.deepEqual(kinds('$arg9'), [['$', 'dollar'], ['arg9', 'error']]);
  assert.deepEqual(kinds('${arg1:nobody}'), [['${', 'dollar'], ['arg1', 'variable'], [':', 'dollar'], ['nobody', 'default'], ['}', 'dollar']]);
  assert.deepEqual(kinds('${arg9:nobody}')[1], ['arg9', 'variable']);
});

test('a backslash writes the next character as it is, and anything else after it is an error', () => {
  assert.deepEqual(kinds('cost \\$5'), [['cost ', 'text'], ['\\$', 'escape'], ['5', 'text']]);
  assert.deepEqual(kinds('a\\q'), [['a', 'text'], ['\\q', 'error']]);
  assert.deepEqual(kinds('end\\'), [['end', 'text'], ['\\', 'error']]);
});

test('what the daemon would refuse is an error: a dollar sign alone, a brace left open', () => {
  assert.deepEqual(kinds('price $'), [['price ', 'text'], ['$', 'error']]);
  assert.deepEqual(kinds('${arg1'), [['${arg1', 'error']]);
  assert.deepEqual(kinds('${}'), [['${', 'error'], ['}', 'text']]);
});
