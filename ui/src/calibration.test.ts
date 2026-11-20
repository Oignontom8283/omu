import assert from 'node:assert/strict';
import { test } from 'node:test';
import { analyse, median, percentile, type Suggestion } from './calibration.ts';

const repeat = (db: number, n: number) => Array.from({ length: n }, () => db);
/** A voice that speaks at `loud` dB a third of the time and pauses at `quiet` dB the rest. */
const speaking = (loud: number, quiet: number, n = 600) => Array.from({ length: n }, (_, i) => (i % 3 === 0 ? loud : quiet));

test('median and percentiles pick values of the list', () => {
  assert.equal(median([3, 1, 2]), 2);
  assert.equal(percentile([1, 2, 3, 4, 5, 6, 7, 8, 9, 10], 90), 9);
  assert.equal(percentile([], 50), 0);
});

test('a clean voice over a quiet room gets a light reduction and a gain that brings it to the target', () => {
  const r = analyse(repeat(-70, 400), speaking(-30, -70)) as Suggestion;
  assert.equal(r.noiseFloor, -70);
  assert.equal(r.voice, -30);
  assert.equal(r.margin, 40);
  assert.equal(r.quality, 'excellent');
  assert.equal(r.gainDb, 18, 'from -30 to -12');
  assert.equal(r.limitDb, 12);
  assert.ok(r.thresholdDb > r.noiseFloor && r.thresholdDb < r.voice, 'the detector opens between the noise and the voice');
});

test('a voice not far above the noise needs all the reduction there is', () => {
  const r = analyse(repeat(-40, 400), speaking(-30, -40)) as Suggestion;
  assert.equal(r.margin, 10);
  assert.equal(r.quality, 'poor');
  assert.equal(r.limitDb, 100);
  assert.equal((analyse(repeat(-50, 400), speaking(-30, -50)) as Suggestion).limitDb, 24, 'a margin of 20 dB: a moderate limit');
});

test('the gain suggested stays within reason', () => {
  assert.equal((analyse(repeat(-110, 400), speaking(-90, -110)) as Suggestion).gainDb, 30);
  assert.equal((analyse(repeat(-60, 400), speaking(-1, -60)) as Suggestion).gainDb, -11);
});

test('a measure that is too short, silent, or without a voice says so', () => {
  assert.deepEqual(analyse(repeat(-70, 10), speaking(-30, -70)), { problem: 'too_short' });
  assert.deepEqual(analyse(repeat(-120, 400), repeat(-120, 400)), { problem: 'no_signal' });
  assert.deepEqual(analyse(repeat(-60, 400), repeat(-60, 400)), { problem: 'no_voice' });
});
