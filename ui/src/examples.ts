// Small graphs to start from: each one is a clip that is inserted where the pointer is, like blocks that were copied.

import type { Clip, ClipNode } from './selection.ts';
import type { NodeSpec } from './types.ts';

/** What a block is made of, without its identifier (for every kind of block). */
type Body = NodeSpec extends infer T ? (T extends unknown ? Omit<T, 'id'> : never) : never;

const node = (id: string, x: number, y: number, spec: Body): ClipNode => ({ id, position: { x: x * 24, y: y * 24 }, spec: { id, ...spec } as NodeSpec });
const link = (source: string, sourceHandle: string, target: string, targetHandle: string) => ({ source, sourceHandle, target, targetHandle });

export type Example = { id: string; name: string; about: string; clip: Clip };

export const EXAMPLES: Example[] = [
  {
    id: 'clean-mic',
    name: 'example.cleanMic',
    about: 'example.cleanMic.about',
    clip: {
      nodes: [
        node('mic', 0, 0, { type: 'device_capture', device: 'default' }),
        node('clean', 13, 0, { type: 'noise_reduction' }),
        node('out', 26, 0, { type: 'virtual_mic', name: 'Clean microphone' }),
      ],
      edges: [link('mic', 'out', 'clean', 'in'), link('clean', 'out', 'out', 'in')],
    },
  },
  {
    id: 'mute-key',
    name: 'example.muteKey',
    about: 'example.muteKey.about',
    clip: {
      nodes: [
        node('key', 0, 14, { type: 'hotkey', keys: 'F9' }),
        node('open', 13, 14, { type: 'variable', initial: true }),
        node('mic', 0, 0, { type: 'device_capture', device: 'default' }),
        node('gate', 26, 0, { type: 'gate' }),
        node('out', 39, 0, { type: 'virtual_mic', name: 'Microphone with a mute key' }),
      ],
      edges: [link('key', 'pressed', 'open', 'toggle'), link('mic', 'out', 'gate', 'in'), link('open', 'value', 'gate', 'open'), link('gate', 'out', 'out', 'in')],
    },
  },
  {
    id: 'duck-music',
    name: 'example.duckMusic',
    about: 'example.duckMusic.about',
    clip: {
      nodes: [
        node('mic', 0, 0, { type: 'device_capture', device: 'default' }),
        node('voice', 13, 0, { type: 'level', threshold_db: -40 }),
        node('pick', 26, 0, { type: 'choose', inputs: { then: -18, else: 0 } }),
        node('music', 0, 14, { type: 'app_capture', target: { process_name: 'spotify.exe' } }),
        node('gain', 39, 14, { type: 'gain', db: 0 }),
        node('out', 52, 14, { type: 'device_render', device: 'default' }),
      ],
      edges: [link('mic', 'out', 'voice', 'in'), link('voice', 'active', 'pick', 'when'), link('pick', 'out', 'gain', 'db'), link('music', 'out', 'gain', 'in'), link('gain', 'out', 'out', 'in')],
    },
  },
];
