import { Handle, Position, useEdges, useUpdateNodeInternals, type Node, type NodeProps } from '@xyflow/react';
import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from 'react';
import { useEditor } from '../context.ts';
import { GAP, GRID, MAX_BRANCHES, MAX_ENTRIES, MAX_JOIN_ARGS, cropTo, inputValue, branchCount, categoryOf, entryCount, entryHandles, hasEntries, isGeneratedId, levelDb, linksPerEntry, meterFill, portsOf, zonesOf, type PortMeta } from '../graph.ts';
import { useT } from '../i18n/index.tsx';
import type { CompareOp, DeviceRef, InputValue, LogicOp, MathOp, NodeKind, NodeSpec, NodeType, RoundMode, TextOp } from '../types.ts';
import { HighlightedInput } from './HighlightedInput.tsx';
import { ShortcutField } from './ShortcutField.tsx';

/** `inputs`: how many input points the block shows, for a block that sums its inputs (the mixer). */
export type OmuNode = Node<{ spec: NodeSpec; inputs?: number }, 'omu'>;

type Of<T extends NodeType> = Extract<NodeSpec, { type: T }>;

function Meter({ peak, off = false }: { peak: number; off?: boolean }) {
  const db = levelDb(peak);
  return (
    <div className={`meter${off ? ' off' : ''}`} title={db <= -60 ? '' : `${db.toFixed(0)} dB`}>
      <div className={`meter-fill${peak > 0.97 ? ' hot' : ''}`} style={{ width: `${meterFill(peak) * 100}%` }} />
    </div>
  );
}

function AppEditor({ spec }: { spec: Of<'app_capture'> }) {
  const { apps, update } = useEditor();
  const t = useT();
  const value = 'pid' in spec.target ? String(spec.target.pid) : spec.target.process_name;
  const names = [...new Set(apps.map((a) => a.name))];
  const known = apps.some((a) => a.name.toLowerCase() === value.toLowerCase());
  return (
    <label className="field">
      <span>{t('field.program')}</span>
      <input
        className="nodrag"
        list={`apps-${spec.id}`}
        value={value}
        placeholder="chrome.exe"
        onChange={(e) => {
          const v = e.target.value;
          update(spec.id, { type: 'app_capture', target: /^\d+$/.test(v.trim()) ? { pid: Number(v.trim()) } : { process_name: v } });
        }}
      />
      <datalist id={`apps-${spec.id}`}>
        {names.map((n) => (
          <option key={n} value={n} />
        ))}
      </datalist>
      {value && !/^\d+$/.test(value) && apps.length > 0 && !known && <small className="warn">{t('field.programNotPlaying')}</small>}
    </label>
  );
}

const refToOption = (d: DeviceRef): string => (d === 'default' ? 'default' : 'id' in d ? `id:${d.id}` : `name:${d.name}`);
const optionToRef = (v: string): DeviceRef => (v === 'default' ? 'default' : v.startsWith('id:') ? { id: v.slice(3) } : { name: v.slice(5) });

function DeviceEditor({ spec }: { spec: Of<'device_capture'> | Of<'device_render'> }) {
  const { devices, update } = useEditor();
  const t = useT();
  const direction = spec.type === 'device_capture' ? 'capture' : 'render';
  const list = devices.filter((d) => d.direction === direction);
  const current = refToOption(spec.device);
  const known = current === 'default' || list.some((d) => `id:${d.id}` === current);
  return (
    <label className="field">
      <span>{t(direction === 'capture' ? 'field.recordingDevice' : 'field.playbackDevice')}</span>
      <select className="nodrag" value={current} onChange={(e) => update(spec.id, { ...spec, device: optionToRef(e.target.value) })}>
        <option value="default">{t('field.systemDefault')}</option>
        {list.map((d) => (
          <option key={d.id} value={`id:${d.id}`}>
            {d.name}
          </option>
        ))}
        {!known && (
          <option value={current}>
            {t('field.notFound')} {current.replace(/^(id|name):/, '')}
          </option>
        )}
      </select>
    </label>
  );
}

/** A soundboard block: which of the soundboards it plays, and a way to the panel where the pads are. */
function SoundboardEditor({ spec }: { spec: Of<'soundboard'> }) {
  const { library, update, openSoundboards } = useEditor();
  const t = useT();
  const known = library.boards.some((b) => b.id === spec.board);
  const board = library.boards.find((b) => b.id === spec.board);
  const triggers = spec.triggers ?? [];
  const edges = useEdges();
  const set = (next: string[]) => update(spec.id, next.length ? { type: 'soundboard', board: spec.board, triggers: next } : { type: 'soundboard', board: spec.board });
  const free = board?.pads.find((p) => !triggers.includes(p.id));
  // the last trigger cannot go while a link is on its input
  return (
    <>
      <label className="field">
        <span>{t('field.soundboard')}</span>
        <select className="nodrag" value={spec.board} onChange={(e) => update(spec.id, { type: 'soundboard', board: e.target.value, ...(spec.triggers?.length ? { triggers: [] } : {}) })}>
          {!known && <option value={spec.board}>{spec.board ? t('field.notFound') : t('field.chooseSoundboard')}</option>}
          {library.boards.map((b) => (
            <option key={b.id} value={b.id}>
              {b.name}
            </option>
          ))}
        </select>
      </label>
      {!known && spec.board !== '' && (
        <small className="warn" title={t('field.soundboardMissing')}>
          {t('field.notFound')}
        </small>
      )}
      {triggers.map((pad, i) => {
        const linked = edges.some((e) => e.target === spec.id && e.targetHandle === `play${i + 1}`);
        const last = i === triggers.length - 1;
        return (
          <div key={i} className="trigger-row">
            <select className="nodrag" aria-label={t('soundboards.trigger', { n: i + 1 })} value={pad} onChange={(e) => set(triggers.map((x, j) => (j === i ? e.target.value : x)))}>
              {!board?.pads.some((p) => p.id === pad) && <option value={pad}>{t('field.notFound')}</option>}
              {board?.pads.map((p) => (
                <option key={p.id} value={p.id}>
                  {p.name || t('soundboards.unnamedPad')}
                </option>
              ))}
            </select>
            {last && !linked && (
              <button className="mini nodrag" title={t('soundboards.removeTrigger')} aria-label={t('soundboards.removeTrigger')} onClick={() => set(triggers.slice(0, -1))}>
                ×
              </button>
            )}
          </div>
        );
      })}
      <button className="mini nodrag" disabled={!free || triggers.length >= MAX_BRANCHES} title={t('soundboards.addTriggerHint')} onClick={() => free && set([...triggers, free.id])}>
        + {t('soundboards.addTrigger')}
      </button>
      <button className="mini wide nodrag" onClick={() => openSoundboards(known ? spec.board : undefined)}>
        {t('soundboards.open')}
      </button>
    </>
  );
}

/** A line of text (a host, a stream name). */
function TextField({ label, value, placeholder, disabled, title, onChange }: { label: string; value: string; placeholder?: string; disabled?: boolean; title?: string; onChange: (text: string) => void }) {
  const t = useT();
  return (
    <label className="field" title={title}>
      <span>{t(label)}</span>
      <input className="nodrag" value={value} placeholder={placeholder} disabled={disabled} spellCheck={false} onChange={(e) => onChange(e.target.value)} />
    </label>
  );
}

/** The larger noise reduction: how far the noise may be reduced, and a way to the module it needs. */
function DeepFilterEditor({ spec }: { spec: Of<'deep_filter'> }) {
  const { update, openModules } = useEditor();
  const t = useT();
  const limit = spec.limit_db ?? 100;
  return (
    <>
      <label className="field">
        <span>
          {t('field.maxReduction')}: {limit >= 100 ? t('field.noLimit') : `${limit} dB`}
        </span>
        <input className="nodrag" type="range" min={6} max={100} step={1} value={limit} onChange={(e) => update(spec.id, { type: 'deep_filter', limit_db: Number(e.target.value) })} />
      </label>
      <button className="mini wide nodrag" onClick={openModules}>
        {t('modules.open')}
      </button>
      <CalibrateButton id={spec.id} />
    </>
  );
}

function Params({ spec }: { spec: NodeSpec }) {
  const { update } = useEditor();
  const t = useT();
  switch (spec.type) {
    case 'app_capture':
      return <AppEditor spec={spec} />;
    case 'device_capture':
    case 'device_render':
      return <DeviceEditor spec={spec} />;
    case 'virtual_speaker':
      return (
        <label className="field">
          <span>{t('field.speakerName')}</span>
          <input className="nodrag" value={spec.name} onChange={(e) => update(spec.id, { type: 'virtual_speaker', name: e.target.value })} />
        </label>
      );
    case 'deep_filter':
      return <DeepFilterEditor spec={spec} />;
    case 'soundboard':
      return <SoundboardEditor spec={spec} />;
    case 'virtual_mic':
      return (
        <label className="field">
          <span>{t('field.micName')}</span>
          <input className="nodrag" value={spec.name} onChange={(e) => update(spec.id, { type: 'virtual_mic', name: e.target.value })} />
        </label>
      );
    case 'variable':
      return (
        <label className="check">
          <input className="nodrag" type="checkbox" checked={spec.initial} onChange={(e) => update(spec.id, { type: 'variable', initial: e.target.checked })} />
          {t('field.startsOn')}
        </label>
      );
    case 'logic':
      return (
        <label className="field">
          <span>{t('field.operation')}</span>
          <select className="nodrag" value={spec.op} onChange={(e) => update(spec.id, { type: 'logic', op: e.target.value as LogicOp })}>
            {(['and', 'or', 'xor', 'not'] as const).map((op) => (
              <option key={op} value={op}>
                {t(`logic.${op}`)}
              </option>
            ))}
          </select>
        </label>
      );
    case 'hotkey':
      return (
        <div className="field">
          <span>{t('field.shortcut')}</span>
          <ShortcutField value={spec.keys} onChange={(keys) => update(spec.id, { type: 'hotkey', keys })} />
        </div>
      );
    case 'delay':
      return <MsField label="field.delay" value={spec.ms} max={5000} onChange={(ms) => update(spec.id, { type: 'delay', ms })} />;
    case 'branch':
    case 'select':
      return <Branches spec={spec} />;
    case 'text':
      return <TextField label="field.text" value={spec.value} placeholder="" onChange={(value) => update(spec.id, { type: 'text', value })} />;
    case 'integer':
      return <NumberField label="field.integer" value={spec.value} whole onChange={(value) => update(spec.id, { type: 'integer', value })} />;
    case 'number':
      return <NumberField label="field.number" value={spec.value} onChange={(value) => update(spec.id, { type: 'number', value })} />;
    case 'join':
      return <JoinEditor spec={spec} />;
    case 'math':
      return (
        <label className="field">
          <span>{t('field.operation')}</span>
          <select className="nodrag" value={spec.op} onChange={(e) => update(spec.id, { type: 'math', op: e.target.value as MathOp })}>
            {(['add', 'subtract', 'multiply', 'divide', 'min', 'max'] as const).map((op) => (
              <option key={op} value={op}>
                {t(`math.${op}`)}
              </option>
            ))}
          </select>
        </label>
      );
    case 'noise_reduction':
      return <CalibrateButton id={spec.id} />;
    case 'compare':
      return <OperationField value={spec.op} options={['greater', 'greater_or_equal', 'equal', 'not_equal', 'less_or_equal', 'less']} prefix="compare" onChange={(op) => update(spec.id, { type: 'compare', op: op as CompareOp })} />;
    case 'text_compare':
      return <OperationField value={spec.op} options={['equals', 'contains', 'starts_with', 'ends_with']} prefix="textop" onChange={(op) => update(spec.id, { type: 'text_compare', op: op as TextOp })} />;
    case 'counter':
      return <NumberField label="field.counterStart" value={spec.initial ?? 0} whole onChange={(initial) => update(spec.id, { type: 'counter', initial })} />;
    case 'round':
      return <OperationField value={spec.mode} options={['nearest', 'down', 'up']} prefix="round" onChange={(mode) => update(spec.id, { type: 'round', mode: mode as RoundMode })} />;
    case 'mixer':
    case 'gate':
    case 'not':
    case 'all':
    case 'any':
    case 'change':
    case 'choose':
    case 'volume':
    case 'to_number':
    case 'from_condition':
    case 'to_condition':
    case 'network_in':
    case 'network_out':
    case 'gain':
    case 'level':
    case 'hold':
    case 'after':
    case 'timer':
      return null;
  }
}

/** The button that opens the calibration window of a noise reduction block. */
function CalibrateButton({ id }: { id: string }) {
  const { openCalibration } = useEditor();
  const t = useT();
  return (
    <button
      className="mini wide nodrag"
      onClick={(e) => {
        const r = (e.currentTarget.closest('.node') ?? e.currentTarget).getBoundingClientRect();
        openCalibration(id, { x: r.right + 12, y: r.top });
      }}
    >
      {t('calibration.open')}
    </button>
  );
}

/** The drop-down of a block that does one of several things (`prefix.option` is the translation of each). */
function OperationField({ value, options, prefix, onChange }: { value: string; options: string[]; prefix: string; onChange: (value: string) => void }) {
  const t = useT();
  return (
    <label className="field">
      <span>{t('field.operation')}</span>
      <select className="nodrag" value={value} onChange={(e) => onChange(e.target.value)}>
        {options.map((o) => (
          <option key={o} value={o}>
            {t(`${prefix}.${o}`)}
          </option>
        ))}
      </select>
    </label>
  );
}

/** A number typed in the block: whole, or with a decimal part. The field keeps what is being typed until it is a number. */
function NumberField({ label, value, whole = false, onChange }: { label: string; value: number; whole?: boolean; onChange: (value: number) => void }) {
  const t = useT();
  const [typed, setTyped] = useState(String(value));
  useEffect(() => setTyped((now) => (Number(now) === value ? now : String(value))), [value]);
  return (
    <label className="field">
      <span>{t(label)}</span>
      <input
        className="nodrag"
        inputMode={whole ? 'numeric' : 'decimal'}
        value={typed}
        onChange={(e) => {
          setTyped(e.target.value);
          const n = whole ? Math.trunc(Number(e.target.value)) : Number(e.target.value);
          if (e.target.value.trim() !== '' && Number.isFinite(n)) onChange(n);
        }}
      />
    </label>
  );
}

/**
 * The text of a join block, with `$1`, `$2`... where its arguments go, and the "+" that adds an argument (or the "×" that
 * takes the last one away when nothing is linked to it).
 */
function JoinEditor({ spec }: { spec: Of<'join'> }) {
  const { update } = useEditor();
  const t = useT();
  const edges = useEdges();
  const count = portsOf(spec).filter((q) => q.dir === 'in' && !q.param).length;
  const used = edges.some((e) => e.target === spec.id && e.targetHandle === `arg${count}`);
  const set = (args: number) => update(spec.id, { ...spec, args });
  return (
    <>
      <div className="adders">
        <button className="mini add nodrag" title={t('node.addArgument')} aria-label={t('node.addArgument')} disabled={count >= MAX_JOIN_ARGS} onClick={() => set(count + 1)}>
          +
        </button>
        {count > 1 && !used && (
          <button className="mini nodrag" title={t('node.removeArgument')} aria-label={t('node.removeArgument')} onClick={() => set(count - 1)}>
            ×
          </button>
        )}
      </div>
    </>
  );
}

/** A number of milliseconds. */
function MsField({ label, value, max, disabled, title, onChange }: { label: string; value: number; max: number; disabled?: boolean; title?: string; onChange: (ms: number) => void }) {
  const t = useT();
  return (
    <label className="field" title={title}>
      <span>{t(label)}</span>
      <input className="nodrag" type="number" min={0} max={max} step={10} value={value} disabled={disabled} onChange={(e) => onChange(Math.max(0, Math.min(max, Math.round(Number(e.target.value) || 0))))} />
    </label>
  );
}

/**
 * The "+" of a block that branches: adds a condition (an `else if`), with its own sound output (or sound input), or
 * takes the last one away when nothing is linked to it.
 */
function Branches({ spec }: { spec: Of<'branch'> | Of<'select'> }) {
  const { update } = useEditor();
  const t = useT();
  const edges = useEdges();
  const count = branchCount(spec);
  const last = portsOf(spec).filter((q) => q.name === `when${count}` || q.name === `out${count}` || q.name === `in${count}`);
  const used = edges.some((e) => (e.target === spec.id && last.some((q) => q.name === e.targetHandle)) || (e.source === spec.id && last.some((q) => q.name === e.sourceHandle)));
  const set = (branches: number) => update(spec.id, { type: spec.type, branches });
  return (
    <div className="adders">
      <button className="mini add nodrag" title={t('node.addCondition')} aria-label={t('node.addCondition')} disabled={count >= MAX_BRANCHES} onClick={() => set(count + 1)}>
        +
      </button>
      {count > 1 && !used && (
        <button className="mini nodrag" title={t('node.removeCondition')} aria-label={t('node.removeCondition')} onClick={() => set(count - 1)}>
          ×
        </button>
      )}
    </div>
  );
}

/**
 * The name people give a block: free text, kept with the graph, and separate from the block's identifier (which the
 * links use and which never changes). A block without a name shows an empty field; one made by hand under a chosen
 * identifier shows that identifier in grey as a hint.
 */
function NameField({ spec }: { spec: NodeSpec }) {
  const { setDisplay } = useEditor();
  const t = useT();
  return (
    <input
      className="node-id nodrag"
      value={spec.display ?? ''}
      placeholder={isGeneratedId(spec.id) ? t('node.namePlaceholder') : spec.id}
      spellCheck={false}
      title={`${t('node.nameHint')} · ${t('node.idLabel')} ${spec.id}`}
      onChange={(e) => setDisplay(spec.id, e.target.value)}
    />
  );
}

/**
 * The sound input of a block that sums: as many connection points as the user added with the "+" (each takes any
 * number of links, which are added together), and a "×" to take the last one away when nothing is linked to it.
 */
function EntriesRow({ port, node, asked }: { port: PortMeta; node: string; asked: number | undefined }) {
  const t = useT();
  const { setInputs } = useEditor();
  const edges = useEdges();
  const updateInternals = useUpdateNodeInternals();
  const count = entryCount(edges, node, port.name, asked);
  const links = linksPerEntry(edges, node, port.name);
  const handles = entryHandles(port.name, count);
  useEffect(() => {
    updateInternals(node);
  }, [count, node, updateInternals]);
  return (
    <>
      {handles.map((h, i) => {
        const n = i + 1;
        const last = n === count;
        return (
          <div key={h} className="port in entry" title={t(`flow.${port.flow}`)}>
            <Handle id={h} type="target" position={Position.Left} className={`flow-${port.flow}`} />
            <span>
              {t(port.label)} {n}
            </span>
            {(links.get(n) ?? 0) > 1 && (
              <small className="sum" title={t('node.summed')}>
                ×{links.get(n)}
              </small>
            )}
            {last && count > 1 && !links.has(n) && (
              <button className="mini nodrag" title={t('node.removeInput')} aria-label={t('node.removeInput')} onClick={() => setInputs(node, count - 1)}>
                ×
              </button>
            )}
          </div>
        );
      })}
      <button className="mini add nodrag" title={t('node.addInput')} aria-label={t('node.addInput')} disabled={count >= MAX_ENTRIES} onClick={() => setInputs(node, count + 1)}>
        +
      </button>
    </>
  );
}

/** One port: a label and the connection point (coloured by the flow it carries). */
function PortRow({ port, spec, inputs, text }: { port: PortMeta; spec: NodeSpec; inputs: number | undefined; text?: string }) {
  const t = useT();
  const isInput = port.dir === 'in';
  if (hasEntries(port)) return <EntriesRow port={port} node={spec.id} asked={inputs} />;
  return (
    <>
      {port.gapBefore && <div className="port gap" aria-hidden="true" />}
      <div className={`port ${isInput ? 'in' : 'out'}${port.inline ? ' inline' : ''}`} title={t(`flow.${port.flow}`)}>
        <Handle id={port.name} type={isInput ? 'target' : 'source'} position={isInput ? Position.Left : Position.Right} className={`flow-${port.flow}`} />
        <span className="port-name">{text ?? t(port.label, { n: port.n ?? '' }).trim()}</span>
        {port.inline && <InlineValue spec={spec} port={port} />}
      </div>
    </>
  );
}

/** A number typed in a field: what is being typed is kept until it is a number, and the number is cropped to the limits. */
function TypedNumber({ value, whole, min, max, disabled, onChange }: { value: number; whole: boolean; min: number; max: number; disabled: boolean; onChange: (n: number) => void }) {
  const [typed, setTyped] = useState(String(value));
  useEffect(() => setTyped((now) => (Number(now) === value ? now : String(value))), [value]);
  return (
    <input
      className="nodrag inline-number"
      inputMode={whole ? 'numeric' : 'decimal'}
      value={typed}
      disabled={disabled}
      onChange={(e) => {
        setTyped(e.target.value);
        const n = whole ? Math.trunc(Number(e.target.value)) : Number(e.target.value);
        if (e.target.value.trim() !== '' && Number.isFinite(n)) onChange(Math.min(max, Math.max(min, n)));
      }}
      onBlur={() => setTyped(String(value))}
    />
  );
}

/**
 * The value of an input, typed next to its connection point: a text, a number (with a slider over the range that is advised
 * when there is one). A flow linked to the input wins over it, and the field is then only shown.
 */
function InlineValue({ spec, port }: { spec: NodeSpec; port: PortMeta }) {
  const { update, setInput } = useEditor();
  const t = useT();
  const edges = useEdges();
  const linked = edges.some((e) => e.target === spec.id && e.targetHandle === port.name);
  const current = inputValue(spec, port);
  const write = (v: InputValue) => (port.field !== undefined ? update(spec.id, { ...spec, [port.field]: v } as NodeKind) : setInput(spec.id, port.name, v));
  const title = linked ? t('node.fromFlow') : undefined;
  if (port.flow === 'text') {
    const text = String(current);
    return port.name === 'template' ? (
      <HighlightedInput value={text} disabled={linked} title={title} known={(name) => /^(arg)?\d+$/.test(name) && Number(name.replace('arg', '')) <= portsOf(spec).filter((q) => q.name.startsWith('arg')).length} onChange={write} />
    ) : (
      <input className="nodrag inline-text" value={text} disabled={linked} title={title} placeholder={port.help && port.field === 'stream' ? t('field.anyStream') : undefined} spellCheck={false} onChange={(e) => write(e.target.value)} />
    );
  }
  const whole = port.flow === 'int';
  const n = Number(current) || 0;
  const l = port.limits;
  const advised = l && l.softMax > l.softMin;
  return (
    <span className="inline-control" title={title}>
      {advised && (
        <input
          className="nodrag inline-slider"
          type="range"
          min={l.softMin}
          max={l.softMax}
          step={whole ? 1 : l.softMax - l.softMin > 50 ? 1 : 0.5}
          value={Math.min(l.softMax, Math.max(l.softMin, n))}
          disabled={linked}
          onChange={(e) => write(cropTo(l, Number(e.target.value)))}
        />
      )}
      <TypedNumber value={n} whole={whole} min={l?.min ?? -1e9} max={l?.max ?? 1e9} disabled={linked} onChange={write} />
      {port.unit && <small className="inline-unit">{port.unit}</small>}
    </span>
  );
}

function Footer({ spec }: { spec: NodeSpec }) {
  const { statusOf } = useEditor();
  const t = useT();
  const status = statusOf(spec.id);
  const failed = status?.state === 'failed';
  const category = categoryOf(spec.type);
  // always the same elements, in the same place: those of a block that is not running are only switched off, so that the
  // block never changes size by itself
  const off = status ? '' : ' off';
  return (
    <>
      <span className={`dot ${status ? (failed ? 'bad' : 'ok') : 'idle'}`} title={status ? undefined : t('status.notRunning')} />
      {category === 'trigger' && <small className={off.trim()}>{t('status.actions', { count: status?.events ?? 0 })}</small>}
      {(category === 'number' || category === 'text' || category === 'conversion') && (
        <small className={`value${off}`} title={status?.value ?? undefined}>
          {status?.value ?? '–'}
        </small>
      )}
      {category === 'condition' && (
        <span className={`lamp${status?.active ? ' on' : ''}${off}`}>{status?.active != null ? t(status.active ? 'status.yes' : 'status.no') : '–'}</span>
      )}
      {failed ? (
        <small className="warn message" title={status.message}>
          {t('status.failed')}
        </small>
      ) : (
        (category === 'input' || category === 'output' || category === 'processing') && <Meter peak={status?.peak ?? 0} off={!status} />
      )}
      {status && (status.underruns > 0 || status.overruns > 0) && (
        <small className="warn" title={t('status.underruns')}>
          {status.underruns}/{status.overruns}
        </small>
      )}
    </>
  );
}

/**
 * The frame of a block. Its height is a whole number of grid cells, less the gap that keeps neighbours apart, whatever
 * its content needs: a line of text more takes a cell more. The block is inset by half the gap on every side, so that
 * the node React Flow measures is exactly those cells and sits on the points of the grid.
 */
function GridBox({ className, children }: { className: string; children: ReactNode }) {
  const inner = useRef<HTMLDivElement>(null);
  const [cells, setCells] = useState(0);
  useLayoutEffect(() => {
    const el = inner.current;
    if (!el) return;
    const measure = () => setCells(Math.max(2, Math.ceil((el.offsetHeight + GAP - 0.5) / GRID)));
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(el);
    return () => observer.disconnect();
  }, []);
  return (
    <div className={className} style={cells ? { height: cells * GRID - GAP } : undefined}>
      <div ref={inner}>{children}</div>
    </div>
  );
}

export function NodeCard({ data, selected }: NodeProps<OmuNode>) {
  const { statusOf } = useEditor();
  const t = useT();
  const spec = data.spec;
  const category = categoryOf(spec.type);
  const { library, openHelp, removeNode } = useEditor();
  const ports = portsOf(spec);
  const failed = statusOf(spec.id)?.state === 'failed';
  // the action inputs of a soundboard block are named after the pads they play
  const padName = (port: PortMeta): string | undefined => {
    if (spec.type !== 'soundboard') return undefined;
    if (port.name === 'stop') return t('soundboards.stopAction');
    const n = port.name.startsWith('play') ? Number(port.name.slice(4)) : 0;
    const pad = library.boards.find((b) => b.id === spec.board)?.pads.find((p) => p.id === spec.triggers?.[n - 1]);
    return n ? `${t('soundboards.playAction')} ${pad?.name || t('soundboards.unnamedPad')}` : undefined;
  };
  return (
    <GridBox className={`node cat-${category}${selected ? ' selected' : ''}${failed ? ' failed' : ''}`}>
      <header>
        <div className="bar">
          <span className="bar-label">{t(`kind.${spec.type}`)}</span>
          <span className="bar-actions nodrag">
            <button
              className="bar-btn"
              title={t('block.help')}
              aria-label={t('block.help')}
              onClick={(e) => {
                // next to the block, not over it
                const r = (e.currentTarget.closest('.node') ?? e.currentTarget).getBoundingClientRect();
                openHelp(spec.id, { x: r.right + 12, y: r.top });
              }}
            >
              <svg viewBox="0 0 12 12" width="12" height="12" aria-hidden="true">
                <circle cx="6" cy="6" r="4.9" fill="none" stroke="currentColor" strokeWidth="1" />
                <circle cx="6" cy="3.5" r="0.75" fill="currentColor" />
                <path d="M6 5.4v3.3" stroke="currentColor" strokeWidth="1.1" fill="none" />
              </svg>
            </button>
            <button className="bar-btn close" title={t('block.remove')} aria-label={t('block.remove')} onClick={() => removeNode(spec.id)}>
              <svg viewBox="0 0 12 12" width="10" height="10" aria-hidden="true">
                <path d="M1.5 1.5l9 9M10.5 1.5l-9 9" stroke="currentColor" strokeWidth="1.1" fill="none" />
              </svg>
            </button>
          </span>
        </div>
        <div className="name">
          <NameField spec={spec} />
        </div>
      </header>
      <div className={`ports${ports.some((q) => q.inline) ? ' stacked' : ''}`}>
        {zonesOf(spec).map((z) => (
          <div key={z.start} className="zone" style={{ top: z.start * GRID, height: z.rows * GRID }} aria-hidden="true" />
        ))}
        <div>{ports.filter((q) => q.dir === 'in').map((q) => <PortRow key={q.name} port={q} spec={spec} inputs={data.inputs} text={padName(q)} />)}</div>
        <div>{ports.filter((q) => q.dir === 'out').map((q) => <PortRow key={q.name} port={q} spec={spec} inputs={data.inputs} />)}</div>
      </div>
      <div className="body">
        <Params spec={spec} />
      </div>
      <footer>
        <Footer spec={spec} />
      </footer>
    </GridBox>
  );
}
