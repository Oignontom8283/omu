import { Handle, Position, useEdges, useUpdateNodeInternals, type Node, type NodeProps } from '@xyflow/react';
import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from 'react';
import { useEditor } from '../context.ts';
import { GAP, GRID, MAX_BRANCHES, MAX_ENTRIES, MAX_EXPRESSION_ARGS, MAX_JOIN_ARGS, inputValue, branchCount, categoryOf, entryCount, entryHandles, hasEntries, isGeneratedId, levelDb, linksPerEntry, meterFill, portsOf, variableName, zonesOf, type PortMeta } from '../graph.ts';
import { useT } from '../i18n/index.tsx';
import type { CompareOp, DeviceRef, InputValue, LogicOp, MathOp, NodeKind, NodeSpec, NodeType, RoundMode, TextOp } from '../types.ts';
import { sizeOf } from '../grouping.ts';
import { Button, ComboInput, NumberField, Select, Slider, Switch, TextInput } from '../kit/controls.tsx';
import { controlNamed, defaultControl } from '../kit/registry.ts';
import '../kit/fields.tsx';
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
    <div className="field">
      <span>{t('field.program')}</span>
      <ComboInput
        value={value}
        options={names}
        placeholder="chrome.exe"
        onChange={(v) => update(spec.id, { type: 'app_capture', target: /^\d+$/.test(v.trim()) ? { pid: Number(v.trim()) } : { process_name: v } })}
      />
      {value && !/^\d+$/.test(value) && apps.length > 0 && !known && <small className="warn">{t('field.programNotPlaying')}</small>}
    </div>
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
  const options = [
    { value: 'default', label: t('field.systemDefault') },
    ...list.map((d) => ({ value: `id:${d.id}`, label: d.name })),
    ...(known ? [] : [{ value: current, label: `${t('field.notFound')} ${current.replace(/^(id|name):/, '')}` }]),
  ];
  return (
    <div className="field">
      <span>{t(direction === 'capture' ? 'field.recordingDevice' : 'field.playbackDevice')}</span>
      <Select value={current} options={options} onChange={(v) => update(spec.id, { ...spec, device: optionToRef(v) })} />
    </div>
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
      <div className="field">
        <span>{t('field.soundboard')}</span>
        <Select
          value={spec.board}
          options={[...(known ? [] : [{ value: spec.board, label: spec.board ? t('field.notFound') : t('field.chooseSoundboard') }]), ...library.boards.map((b) => ({ value: b.id, label: b.name }))]}
          onChange={(board) => update(spec.id, { type: 'soundboard', board, ...(spec.triggers?.length ? { triggers: [] } : {}) })}
        />
      </div>
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
            <Select
              label={t('soundboards.trigger', { n: i + 1 })}
              value={pad}
              options={[...(board?.pads.some((p) => p.id === pad) ? [] : [{ value: pad, label: t('field.notFound') }]), ...(board?.pads.map((p) => ({ value: p.id, label: p.name || t('soundboards.unnamedPad') })) ?? [])]}
              onChange={(id) => set(triggers.map((x, j) => (j === i ? id : x)))}
            />
            {last && !linked && (
              <Button size="small" title={t('soundboards.removeTrigger')} onClick={() => set(triggers.slice(0, -1))}>
                ×
              </Button>
            )}
          </div>
        );
      })}
      <Button size="small" disabled={!free || triggers.length >= MAX_BRANCHES} title={t('soundboards.addTriggerHint')} onClick={() => free && set([...triggers, free.id])}>
        + {t('soundboards.addTrigger')}
      </Button>
      <Button size="small" wide onClick={() => openSoundboards(known ? spec.board : undefined)}>
        {t('soundboards.open')}
      </Button>
    </>
  );
}

function TextField({ label, value, placeholder, disabled, title, onChange }: { label: string; value: string; placeholder?: string; disabled?: boolean; title?: string; onChange: (text: string) => void }) {
  const t = useT();
  return (
    <div className="field" title={title}>
      <span>{t(label)}</span>
      <TextInput value={value} placeholder={placeholder} disabled={disabled} onChange={onChange} />
    </div>
  );
}

function DeepFilterEditor({ spec }: { spec: Of<'deep_filter'> }) {
  const { update, openModules } = useEditor();
  const t = useT();
  const limit = spec.limit_db ?? 100;
  const set = (limit_db: number) => update(spec.id, { type: 'deep_filter', limit_db });
  return (
    <>
      <div className="field">
        <span>
          {t('field.maxReduction')}: {limit >= 100 ? t('field.noLimit') : `${limit} dB`}
        </span>
        <span className="field-pair">
          <Slider value={limit} min={6} max={100} label={t('field.maxReduction')} onChange={set} />
          <NumberField value={limit} whole min={6} max={100} unit="dB" onChange={set} />
        </span>
      </div>
      <Button size="small" wide onClick={openModules}>
        {t('modules.open')}
      </Button>
      <CalibrateButton id={spec.id} />
    </>
  );
}

export function Params({ spec }: { spec: NodeSpec }) {
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
        <div className="field">
          <span>{t('field.speakerName')}</span>
          <TextInput value={spec.name} onChange={(name) => update(spec.id, { type: 'virtual_speaker', name })} />
        </div>
      );
    case 'deep_filter':
      return <DeepFilterEditor spec={spec} />;
    case 'soundboard':
      return <SoundboardEditor spec={spec} />;
    case 'virtual_mic':
      return (
        <div className="field">
          <span>{t('field.micName')}</span>
          <TextInput value={spec.name} onChange={(name) => update(spec.id, { type: 'virtual_mic', name })} />
        </div>
      );
    case 'variable':
      return (
        <div className="check">
          <Switch checked={spec.initial} onChange={(initial) => update(spec.id, { type: 'variable', initial })}>
            {t('field.startsOn')}
          </Switch>
        </div>
      );
    case 'logic':
      return <OperationField value={spec.op} options={['and', 'or', 'xor', 'not']} prefix="logic" onChange={(op) => update(spec.id, { type: 'logic', op: op as LogicOp })} />;
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
      return <ValueField label="field.integer" value={spec.value} whole onChange={(value) => update(spec.id, { type: 'integer', value })} />;
    case 'number':
      return <ValueField label="field.number" value={spec.value} onChange={(value) => update(spec.id, { type: 'number', value })} />;
    case 'join':
      return <InputCount spec={spec} max={MAX_JOIN_ARGS} handle={(n) => `arg${n}`} add="node.addArgument" remove="node.removeArgument" />;
    case 'math_expr':
    case 'cond_expr':
      return <InputCount spec={spec} max={MAX_EXPRESSION_ARGS} handle={(n) => variableName(n - 1)} add="node.addVariable" remove="node.removeVariable" />;
    case 'math':
      return <OperationField value={spec.op} options={['add', 'subtract', 'multiply', 'divide', 'min', 'max']} prefix="math" onChange={(op) => update(spec.id, { type: 'math', op: op as MathOp })} />;
    case 'noise_reduction':
      return <CalibrateButton id={spec.id} />;
    case 'compare':
      return <OperationField value={spec.op} options={['greater', 'greater_or_equal', 'equal', 'not_equal', 'less_or_equal', 'less']} prefix="compare" onChange={(op) => update(spec.id, { type: 'compare', op: op as CompareOp })} />;
    case 'text_compare':
      return <OperationField value={spec.op} options={['equals', 'contains', 'starts_with', 'ends_with']} prefix="textop" onChange={(op) => update(spec.id, { type: 'text_compare', op: op as TextOp })} />;
    case 'counter':
      return <ValueField label="field.counterStart" value={spec.initial ?? 0} whole onChange={(initial) => update(spec.id, { type: 'counter', initial })} />;
    case 'group':
      return <GroupBody spec={spec} />;
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
    case 'web_in':
    case 'web_out':
    case 'stream_in':
    case 'stream_out':
    case 'gain':
    case 'level':
    case 'hold':
    case 'after':
    case 'timer':
      return null;
  }
}

/** A group: how many blocks it holds, and the way to take it apart. */
function GroupBody({ spec }: { spec: Of<'group'> }) {
  const { ungroup } = useEditor();
  const t = useT();
  return (
    <>
      <small className="muted">{t('group.contains', { count: sizeOf(spec) })}</small>
      <Button size="small" wide onClick={() => ungroup(spec.id)}>
        {t('group.take')}
      </Button>
    </>
  );
}

/** The button that opens the calibration window of a noise reduction block. */
function CalibrateButton({ id }: { id: string }) {
  const { openCalibration } = useEditor();
  const t = useT();
  return (
    <Button
      size="small"
      wide
      onClick={(e) => {
        const r = (e.currentTarget.closest('.node') ?? e.currentTarget).getBoundingClientRect();
        openCalibration(id, { x: r.right + 12, y: r.top });
      }}
    >
      {t('calibration.open')}
    </Button>
  );
}

function OperationField({ value, options, prefix, onChange }: { value: string; options: string[]; prefix: string; onChange: (value: string) => void }) {
  const t = useT();
  return (
    <div className="field">
      <span>{t('field.operation')}</span>
      <Select value={value} options={options.map((o) => ({ value: o, label: t(`${prefix}.${o}`) }))} onChange={onChange} />
    </div>
  );
}

function ValueField({ label, value, whole = false, onChange }: { label: string; value: number; whole?: boolean; onChange: (value: number) => void }) {
  const t = useT();
  return (
    <div className="field">
      <span>{t(label)}</span>
      <NumberField value={value} whole={whole} step={whole ? 1 : 0.5} onChange={onChange} />
    </div>
  );
}

/**
 * The buttons of a block that has as many inputs as it is asked for (a join, an expression): the "+" that adds one, and the "−" that
 * takes the last one away when nothing is linked to it.
 */
function InputCount({ spec, max, handle, add, remove }: { spec: Of<'join'> | Of<'math_expr'> | Of<'cond_expr'>; max: number; handle: (n: number) => string; add: string; remove: string }) {
  const { update } = useEditor();
  const t = useT();
  const edges = useEdges();
  const count = portsOf(spec).filter((q) => q.dir === 'in' && !q.param).length;
  const used = edges.some((e) => e.target === spec.id && e.targetHandle === handle(count));
  const set = (args: number) => update(spec.id, { ...spec, args });
  return (
    <div className="adders">
      <Button size="small" icon="plus" title={t(add)} disabled={count >= max} onClick={() => set(count + 1)} />
      {count > 1 && !used && <Button size="small" icon="minus" title={t(remove)} onClick={() => set(count - 1)} />}
    </div>
  );
}

/** A number of milliseconds. */
function MsField({ label, value, max, disabled, title, onChange }: { label: string; value: number; max: number; disabled?: boolean; title?: string; onChange: (ms: number) => void }) {
  const t = useT();
  return (
    <div className="field" title={title}>
      <span>{t(label)}</span>
      <NumberField whole min={0} max={max} step={10} unit="ms" value={value} disabled={disabled} onChange={onChange} />
    </div>
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
      <Button size="small" icon="plus" title={t('node.addCondition')} disabled={count >= MAX_BRANCHES} onClick={() => set(count + 1)} />
      {count > 1 && !used && <Button size="small" icon="minus" title={t('node.removeCondition')} onClick={() => set(count - 1)} />}
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
    <TextInput
      className="node-id"
      code={false}
      value={spec.display ?? ''}
      placeholder={isGeneratedId(spec.id) ? t('node.namePlaceholder') : spec.id}
      title={`${t('node.nameHint')} · ${t('node.idLabel')} ${spec.id}`}
      onChange={(name) => setDisplay(spec.id, name)}
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
              <Button size="small" icon="minus" title={t('node.removeInput')} onClick={() => setInputs(node, count - 1)} />
            )}
          </div>
        );
      })}
      <div className="adders in-ports">
        <Button size="small" icon="plus" title={t('node.addInput')} disabled={count >= MAX_ENTRIES} onClick={() => setInputs(node, count + 1)} />
      </div>
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

/**
 * The value of an input, typed next to its connection point, drawn by the control that the definition of the input names (or
 * the one that suits its sort of value). A flow linked to the input wins over it, and the field is then only shown.
 */
export function InlineValue({ spec, port }: { spec: NodeSpec; port: PortMeta }) {
  const { update, setInput } = useEditor();
  const t = useT();
  const edges = useEdges();
  const linked = edges.some((e) => e.target === spec.id && e.targetHandle === port.name);
  const write = (v: InputValue) => (port.field !== undefined ? update(spec.id, { ...spec, [port.field]: v } as NodeKind) : setInput(spec.id, port.name, v));
  const Control = controlNamed(port.control ?? defaultControl(port)) ?? controlNamed('omu.text');
  if (!Control) return null;
  return <Control spec={spec} port={port} value={inputValue(spec, port)} disabled={linked} title={linked ? t('node.fromFlow') : undefined} write={write} />;
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
          {status?.value === '' ? t('status.empty') : (status?.value ?? '–')}
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
    <GridBox className={`node cat-${category}${selected ? ' selected' : ''}${failed ? ' failed' : ''}${spec.muted ? ' muted' : ''}`}>
      <header>
        <div className="bar">
          <span className="bar-label">{t(`kind.${spec.type}`)}</span>
          <span className="bar-actions nodrag">
            <Button
              variant="bar"
              icon="help"
              title={t('block.help')}
              onClick={(e) => {
                // next to the block, not over it
                const r = (e.currentTarget.closest('.node') ?? e.currentTarget).getBoundingClientRect();
                openHelp(spec.id, { x: r.right + 12, y: r.top });
              }}
            />
            <Button variant="bar" icon="close" className="close" title={t('block.remove')} onClick={() => removeNode(spec.id)} />
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
