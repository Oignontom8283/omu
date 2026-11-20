import assert from 'node:assert/strict';
import { test } from 'node:test';
import { evaluate, written } from './expression.ts';

test('plain numbers, with a point or a comma', () => {
  assert.equal(evaluate('12'), 12);
  assert.equal(evaluate(' -6.5 '), -6.5);
  assert.equal(evaluate('0,25'), 0.25);
  assert.equal(evaluate('1e3'), 1000);
});

test('arithmetic with the usual order, signs and brackets', () => {
  assert.equal(evaluate('2+3*4'), 14);
  assert.equal(evaluate('(2+3)*4'), 20);
  assert.equal(evaluate('10 / 4'), 2.5);
  assert.equal(evaluate('-(3 - 5)'), 2);
  assert.equal(evaluate('2 × 3 ÷ 4'), 1.5);
  assert.equal(evaluate('--2'), 2);
});

test('what is not an expression gives nothing, and nothing is ever run', () => {
  for (const bad of ['', 'abc', '2+', '(2', '2 3', '1/0', 'alert(1)', '2**3']) assert.equal(evaluate(bad), null, bad);
});

test('a number is written without useless decimals', () => {
  assert.equal(written(2.5, false), '2.5');
  assert.equal(written(3, false), '3');
  assert.equal(written(1 / 3, false), '0.3333');
  assert.equal(written(2.6, true), '3');
});
