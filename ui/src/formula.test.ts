import assert from 'node:assert/strict';
import { test } from 'node:test';
import { tokenizeFormula } from './formula.ts';

const kinds = (source: string, inputs = ['a', 'b']) => tokenizeFormula(source, inputs).filter((t) => t.kind !== 'space').map((t) => `${t.kind}:${t.text}`);

test('the pieces put end to end give the text back', () => {
  for (const text of ['a + b', 'c * log(a)', '  min( a , 2.5e3 )  ', 'a >= 3 and b != 1', '¤ #']) assert.equal(tokenizeFormula(text, ['a', 'b']).map((t) => t.text).join(''), text);
});

test('inputs, functions, numbers and operators each have their colour', () => {
  assert.deepEqual(kinds('a + b * 2'), ['variable:a', 'operator:+', 'variable:b', 'operator:*', 'number:2']);
  assert.deepEqual(kinds('log(a)'), ['function:log', 'paren:(', 'variable:a', 'paren:)']);
  assert.deepEqual(kinds('a = b'), ['variable:a', 'operator:=', 'variable:b']);
  assert.deepEqual(kinds('a >= 1.5 and b <= .5'), ['variable:a', 'operator:>=', 'number:1.5', 'keyword:and', 'variable:b', 'operator:<=', 'number:.5']);
  assert.deepEqual(kinds('a || b && a'), ['variable:a', 'operator:||', 'variable:b', 'operator:&&', 'variable:a']);
});

test('a word that is not an input, and a function without its parenthesis, are marked as unknown', () => {
  assert.deepEqual(kinds('a + z'), ['variable:a', 'operator:+', 'unknown:z']);
  assert.deepEqual(kinds('c', ['a', 'b']), ['unknown:c']);
  assert.deepEqual(kinds('c', ['a', 'b', 'c']), ['variable:c']);
  assert.deepEqual(kinds('log + 1'), ['unknown:log', 'operator:+', 'number:1']);
  assert.deepEqual(kinds('e() * a'), ['function:e', 'paren:(', 'paren:)', 'operator:*', 'variable:a']);
  assert.deepEqual(kinds('a # b'), ['variable:a', 'unknown:#', 'variable:b']);
});
