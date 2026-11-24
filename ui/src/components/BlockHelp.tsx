import { FORMULA_FUNCTIONS, TEXT_METHODS, TEXT_SIZE } from '../formula.ts';
import { CATEGORIES, KINDS_BY_CATEGORY, categoryOf, defaultKind, portsOf, type PortMeta } from '../graph.ts';
import { useT } from '../i18n/index.tsx';
import type { FlowType, NodeSpec, NodeType } from '../types.ts';
import { Markdown } from './Markdown.tsx';

/** What a documentation window is about: a block of the graph, a kind of block, or a kind of flow. */
export type DocRef = { kind: 'node'; node: string } | { kind: 'kind'; type: NodeType } | { kind: 'type'; flow: FlowType };

export const docKey = (ref: DocRef): string => (ref.kind === 'node' ? `node:${ref.node}` : ref.kind === 'kind' ? `kind:${ref.type}` : `type:${ref.flow}`);

/** Makes a connection point of a block stand out for a moment. Says whether it found it. */
export function flashPort(node: string, port: string): boolean {
  const handle = document.querySelector(`.react-flow__node[data-id="${CSS.escape(node)}"] .react-flow__handle[data-handleid="${CSS.escape(port)}"]`);
  const row = handle?.closest('.port');
  if (!row) return false;
  row.classList.remove('flash');
  void (row as HTMLElement).offsetWidth; // so that the effect starts again when asked again
  row.classList.add('flash');
  window.setTimeout(() => row.classList.remove('flash'), 1800);
  return true;
}

/** The settings of each kind of block: the translation key of the field's label (its explanation is that key + `.help`) and its type. */
const SETTINGS: Partial<Record<NodeType, { key: string; type: string }[]>> = {
  app_capture: [{ key: 'field.program', type: 'Text' }],
  device_capture: [{ key: 'field.recordingDevice', type: 'Device' }],
  device_render: [{ key: 'field.playbackDevice', type: 'Device' }],
  virtual_mic: [{ key: 'field.micName', type: 'Text' }],
  virtual_speaker: [{ key: 'field.speakerName', type: 'Text' }],
  variable: [{ key: 'field.startsOn', type: 'Condition' }],
  logic: [{ key: 'field.operation', type: 'Operation' }],
  hotkey: [{ key: 'field.shortcut', type: 'Keys' }],
  delay: [{ key: 'field.delay', type: 'ms' }],
  soundboard: [{ key: 'field.soundboard', type: 'Soundboard' }],
  soundboard_sound: [
    { key: 'field.soundboard', type: 'Soundboard' },
    { key: 'field.sound', type: 'Sound' },
  ],
  audio_file: [
    { key: 'field.sound', type: 'Sound' },
    { key: 'field.loop', type: 'Condition' },
    { key: 'field.level', type: 'dB' },
  ],
  tone: [
    { key: 'field.wave', type: 'Wave' },
    { key: 'field.pitch', type: 'Hz' },
    { key: 'field.level', type: 'dB' },
  ],
  noise: [
    { key: 'field.noiseColor', type: 'Colour' },
    { key: 'field.level', type: 'dB' },
  ],
  deep_filter: [{ key: 'field.maxReduction', type: 'dB' }],
  text: [{ key: 'field.text', type: 'Text' }],
  integer: [{ key: 'field.integer', type: 'Integer' }],
  number: [{ key: 'field.number', type: 'Decimal' }],
  math: [{ key: 'field.operation', type: 'Operation' }],
  compare: [{ key: 'field.operation', type: 'Operation' }],
  text_compare: [{ key: 'field.operation', type: 'Operation' }],
  counter: [
    { key: 'field.counterStart', type: 'Integer' },
    { key: 'field.counterUp', type: 'Integer' },
    { key: 'field.counterDown', type: 'Integer' },
  ],
  slider: [
    { key: 'field.sliderMin', type: 'Decimal' },
    { key: 'field.sliderMax', type: 'Decimal' },
  ],
  round: [{ key: 'field.rounding', type: 'Mode' }],
};

const FLOWS: FlowType[] = ['audio', 'condition', 'action', 'text', 'int', 'float'];

/** The title of the window of a documentation. */
export function docTitle(t: ReturnType<typeof useT>, ref: DocRef, spec?: NodeSpec): string {
  if (ref.kind === 'type') return t(`type.${ref.flow}`);
  return t(`kind.${ref.kind === 'node' ? (spec?.type ?? 'gain') : ref.type}`);
}

/**
 * The documentation of a block, a kind of block or a kind of flow, written in markdown (and translated, so each language
 * says it in its own words). What it names — a connection point, a kind of block, a type of flow — is a link to that object.
 */
export function Doc({ doc, spec, onOpen }: { doc: DocRef; spec?: NodeSpec; onOpen: (ref: DocRef) => void }) {
  const t = useT();
  const has = (key: string) => t(key) !== key;
  const type = doc.kind === 'node' ? spec?.type : doc.kind === 'kind' ? doc.type : undefined;
  const ports: PortMeta[] = doc.kind === 'node' ? (spec ? portsOf(spec) : []) : doc.kind === 'kind' ? portsOf(defaultKind(doc.type)) : [];
  const flowOf = (port: string) => (ports.find((p) => p.name === port) ?? ports.find((p) => p.name === port.replace(/\d+$/, '')))?.flow ?? 'audio';
  const label = (p: PortMeta) => t(p.label, { n: p.n ?? '' }).trim();
  // what an input is for: what its setting says, with the range it can take; otherwise just its name
  const about = (p: PortMeta) => {
    const text = p.help ? t(`${p.help}.help`) : label(p);
    return p.limits ? `${text} ${t('help.range', { min: p.limits.min, max: p.limits.max })}` : text;
  };
  const row = (p: PortMeta) => `| [${p.name}](port:${p.name}) | [${t(`type.${p.flow}`)}](type:${p.flow}) | ${about(p)} |`;
  const head = `| ${t('help.col.name')} | ${t('help.col.type')} | ${t('help.col.description')} |\n|---|---|---|`;

  const lines: string[] = [];
  if (type) {
    lines.push(`# ${t(`kind.${type}`)}`, '', t(`kind.${type}.hint`), '');
    const ins = ports.filter((p) => p.dir === 'in');
    const outs = ports.filter((p) => p.dir === 'out');
    if (ins.length) lines.push(`## ${t('help.inputs')}`, head, ...ins.map(row), '');
    if (outs.length) lines.push(`## ${t('help.outputs')}`, head, ...outs.map(row), '');
    const settings = SETTINGS[type] ?? [];
    if (settings.length) {
      lines.push(`## ${t('help.settings')}`, head, ...settings.map((s) => `| ${t(s.key)} | ${s.type === 'Condition' ? `[Condition](type:condition)` : `\`${s.type}\``} | ${t(`${s.key}.help`)} |`), '');
    }
    // what can be written in the block, with examples, and the functions that can be called
    if (has(`kind.${type}.examples`)) lines.push(`## ${t('help.examples')}`, t(`kind.${type}.examples`), '');
    const functions = type === 'text_expr' ? [...TEXT_METHODS, TEXT_SIZE].map((f) => ({ f, key: `test.fn.${f.name}` })) : type === 'math_expr' || type === 'cond_expr' ? FORMULA_FUNCTIONS.map((f) => ({ f, key: `formula.fn.${f.name}` })) : [];
    if (functions.length) lines.push(`## ${t('help.functions')}`, `| ${t('help.col.function')} | ${t('help.col.does')} |`, '|---|---|', ...functions.map(({ f, key }) => `| \`${f.signature}\` | ${t(key)} |`), '');
    if (has(`kind.${type}.warning`)) lines.push(`## ${t('help.warning')}`, `> ${t(`kind.${type}.warning`)}`, '');
    const family = KINDS_BY_CATEGORY[categoryOf(type)].filter((k) => k !== type);
    if (family.length) lines.push(`## ${t('help.seeAlso')}`, ...family.map((k) => `- [${t(`kind.${k}`)}](kind:${k})`), '');
  } else if (doc.kind === 'type') {
    const users = CATEGORIES.flatMap((c) => KINDS_BY_CATEGORY[c]).filter((k) => portsOf(defaultKind(k)).some((p) => p.flow === doc.flow));
    lines.push(`# ${t(`type.${doc.flow}`)}`, '', t(`help.type.${doc.flow}`), '', `## ${t('help.usedBy')}`, ...users.map((k) => `- [${t(`kind.${k}`)}](kind:${k})`), '');
  }
  const node = doc.kind === 'node' ? doc.node : undefined;
  return (
    <Markdown
      source={lines.join('\n')}
      links={{
        flowOf,
        port: node ? (port) => void flashPort(node, port) : undefined,
        kind: (id) => onOpen({ kind: 'kind', type: id as NodeType }),
        type: (id) => onOpen({ kind: 'type', flow: FLOWS.includes(id as FlowType) ? (id as FlowType) : 'audio' }),
      }}
    />
  );
}
