// The calibration of a voice: from how loud the microphone was while the person kept quiet, then while they spoke, what
// settings to suggest. Pure arithmetic, tested with Node (no React here).

/** What a calibration suggests. All levels are in decibels (0 is full scale). */
export type Suggestion = {
  /** How loud the room is when nobody speaks. */
  noiseFloor: number;
  /** How loud the voice is (the loud part of what was said). */
  voice: number;
  /** The voice above the noise. */
  margin: number;
  quality: 'excellent' | 'good' | 'fair' | 'poor';
  /** The gain that brings the voice to a comfortable level. */
  gainDb: number;
  /** Where a voice detector should open: between the noise and the voice. */
  thresholdDb: number;
  /** The most a noise reduction should lower the noise by (100: no limit). */
  limitDb: number;
};

/** Why no suggestion could be made. */
export type Problem = 'too_short' | 'no_signal' | 'no_voice';

/** The level the loud part of the voice is brought to. */
export const TARGET_VOICE_DB = -12;
/** How much above the noise a moment has to be to count as speech. */
const SPEECH_ABOVE_NOISE = 8;
/** The shortest measure that counts: one second of ten-millisecond cycles. */
const MIN_CYCLES = 100;

export const median = (values: number[]): number => percentile(values, 50);

/** The value below which `p` percent of `values` lie (nearest rank). Zero for no values. */
export function percentile(values: number[], p: number): number {
  if (values.length === 0) return 0;
  const sorted = [...values].sort((a, b) => a - b);
  const rank = Math.ceil((p / 100) * sorted.length) - 1;
  return sorted[Math.min(sorted.length - 1, Math.max(0, rank))];
}

const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v));

/** Suggests settings from the levels heard while quiet and while speaking, or says what went wrong. */
export function analyse(quiet: number[], speech: number[]): Suggestion | { problem: Problem } {
  if (quiet.length < MIN_CYCLES || speech.length < MIN_CYCLES) return { problem: 'too_short' };
  // digital silence all along: the microphone is muted, unplugged or not the one in use
  if (Math.max(...speech) < -100) return { problem: 'no_signal' };
  const noiseFloor = median(quiet);
  const spoken = speech.filter((db) => db > noiseFloor + SPEECH_ABOVE_NOISE);
  if (spoken.length / speech.length < 0.15) return { problem: 'no_voice' };
  const voice = percentile(spoken, 90);
  const margin = voice - noiseFloor;
  return {
    noiseFloor: Math.round(noiseFloor),
    voice: Math.round(voice),
    margin: Math.round(margin),
    quality: margin >= 30 ? 'excellent' : margin >= 20 ? 'good' : margin >= 12 ? 'fair' : 'poor',
    gainDb: Math.round(clamp(TARGET_VOICE_DB - voice, -20, 30)),
    thresholdDb: Math.round(clamp(noiseFloor + margin / 3, -80, -10)),
    // a clean voice needs little reduction (and sounds more natural); a noisy one needs all it can get
    limitDb: margin < 15 ? 100 : margin < 30 ? 24 : 12,
  };
}
