import { Handle, Position, useEdges, useUpdateNodeInternals, type Node, type NodeProps } from '@xyflow/react';
import { useEffect, useState } from 'react';
import { useEditor } from '../context.ts';
import { KINDS_BY_CATEGORY, MAX_BRANCHES, MAX_ENTRIES, branchCount, categoryOf, entryCount, entryHandles, hasEntries, levelDb, linksPerEntry, meterFill, portsOf, type PortMeta } from '../graph.ts';
import { useT } from '../i18n/index.tsx';
import type { DeviceRef, LogicOp, NodeSpec, NodeType } from '../types.ts';
import { ShortcutField } from './ShortcutField.tsx';

/** `inputs`: how many input points the block shows, for a block that sums its inputs (the mixer). */
export type OmuNode = Node<{ spec: NodeSpec; inputs?: number }, 'omu'>;

type Of<T extends NodeType> = Extract<NodeSpec, { type: T }>;

function Meter({ peak }: { peak: number }) {
  const db = levelDb(peak);
  return (
    <div className="meter" title={db <= -60 ? '' : `${db.toFixed(0)} dB`}>
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
    case 'virtual_mic':
      return (
        <label className="field">
          <span>{t('field.micName')}</span>
          <input className="nodrag" value={spec.name} onChange={(e) => update(spec.id, { type: 'virtual_mic', name: e.target.value })} />
        </label>
      );
    case 'gain':
      return (
        <label className="field">
          <span>
            {t('field.gain')}: {spec.db > 0 ? '+' : ''}
            {spec.db} dB
          </span>
          <input className="nodrag" type="range" min={-60} max={24} step={0.5} value={spec.db} onChange={(e) => update(spec.id, { type: 'gain', db: Number(e.target.value) })} />
        </label>
      );
    case 'variable':
      return (
        <label className="check">
          <input className="nodrag" type="checkbox" checked={spec.initial} onChange={(e) => update(spec.id, { type: 'variable', initial: e.target.checked })} />
          {t('field.startsOn')}
        </label>
      );
    case 'level':
      return (
        <label className="field">
          <span>
            {t('field.threshold')}: {spec.threshold_db} dB
          </span>
          <input className="nodrag" type="range" min={-80} max={0} step={1} value={spec.threshold_db} onChange={(e) => update(spec.id, { type: 'level', threshold_db: Number(e.target.value) })} />
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
    case 'timer':
      return (
        <label className="field">
          <span>{t('field.interval')}</span>
          <input className="nodrag" type="number" min={10} step={10} value={spec.interval_ms} onChange={(e) => update(spec.id, { type: 'timer', interval_ms: Math.max(10, Number(e.target.value) || 10) })} />
        </label>
      );
    case 'delay':
      return <MsField label="field.delay" value={spec.ms} max={5000} onChange={(ms) => update(spec.id, { type: 'delay', ms })} />;
    case 'hold':
      return <MsField label="field.hold" value={spec.ms} max={60000} onChange={(ms) => update(spec.id, { type: 'hold', ms })} />;
    case 'after':
      return <MsField label="field.after" value={spec.ms} max={60000} onChange={(ms) => update(spec.id, { type: 'after', ms })} />;
    case 'branch':
    case 'select':
      return (
        <>
          <Branches spec={spec} />
          <small className="hint">{t(`kind.${spec.type}.hint`)}</small>
        </>
      );
    case 'mixer':
    case 'gate':
    case 'not':
    case 'all':
    case 'any':
    case 'change':
      return <small className="hint">{t(`kind.${spec.type}.hint`)}</small>;
  }
}

/** A number of milliseconds. */
function MsField({ label, value, max, onChange }: { label: string; value: number; max: number; onChange: (ms: number) => void }) {
  const t = useT();
  return (
    <label className="field">
      <span>{t(label)}</span>
      <input className="nodrag" type="number" min={0} max={max} step={10} value={value} onChange={(e) => onChange(Math.max(0, Math.min(max, Math.round(Number(e.target.value) || 0))))} />
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

function IdField({ id }: { id: string }) {
  const { rename } = useEditor();
  const t = useT();
  const [draft, setDraft] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const commit = () => {
    setError(draft !== null && draft !== id ? rename(id, draft) : null);
    setDraft(null);
  };
  return (
    <>
      <input
        className="node-id nodrag"
        value={draft ?? id}
        spellCheck={false}
        title={t('node.nameHint')}
        onChange={(e) => setDraft(e.target.value)}
        onBlur={commit}
        onKeyDown={(e) => e.key === 'Enter' && (e.target as HTMLInputElement).blur()}
      />
      {error && <small className="warn">{t(error)}</small>}
    </>
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
function PortRow({ port, node, inputs }: { port: PortMeta; node: string; inputs: number | undefined }) {
  const t = useT();
  const isInput = port.dir === 'in';
  if (hasEntries(port)) return <EntriesRow port={port} node={node} asked={inputs} />;
  return (
    <>
      {port.gapBefore && <div className="port gap" aria-hidden="true" />}
      <div className={`port ${isInput ? 'in' : 'out'}`} title={t(`flow.${port.flow}`)}>
        <Handle id={port.name} type={isInput ? 'target' : 'source'} position={isInput ? Position.Left : Position.Right} className={`flow-${port.flow}`} />
        <span>{t(port.label, { n: port.n ?? '' }).trim()}</span>
      </div>
    </>
  );
}

function Footer({ spec }: { spec: NodeSpec }) {
  const { statusOf } = useEditor();
  const t = useT();
  const status = statusOf(spec.id);
  if (!status) return <small className="hint">{t('status.notRunning')}</small>;
  const failed = status.state === 'failed';
  const category = categoryOf(spec.type);
  return (
    <>
      <span className={`dot ${failed ? 'bad' : 'ok'}`} />
      {category === 'trigger' && <small>{t('status.actions', { count: status.events ?? 0 })}</small>}
      {category === 'condition' && status.active != null && <span className={`lamp ${status.active ? 'on' : ''}`}>{t(status.active ? 'status.yes' : 'status.no')}</span>}
      {(category === 'input' || category === 'output' || category === 'processing') && <Meter peak={status.peak} />}
      {(status.underruns > 0 || status.overruns > 0) && (
        <small className="warn" title={t('status.underruns')}>
          {status.underruns}/{status.overruns}
        </small>
      )}
      {failed && <small className="warn full">{status.message}</small>}
    </>
  );
}

export function NodeCard({ id, data, selected }: NodeProps<OmuNode>) {
  const { statusOf, changeKind } = useEditor();
  const t = useT();
  const spec = data.spec;
  const category = categoryOf(spec.type);
  const ports = portsOf(spec);
  const failed = statusOf(spec.id)?.state === 'failed';
  return (
    <div className={`node cat-${category}${selected ? ' selected' : ''}${failed ? ' failed' : ''}`}>
      <header>
        <select className="kind nodrag" value={spec.type} title={t(`category.${category}.hint`)} onChange={(e) => changeKind(spec.id, e.target.value as NodeType)}>
          {KINDS_BY_CATEGORY[category].map((k) => (
            <option key={k} value={k}>
              {t(`category.${category}`)} · {t(`kind.${k}`)}
            </option>
          ))}
        </select>
        <IdField id={spec.id} />
      </header>
      <div className="ports">
        <div>{ports.filter((q) => q.dir === 'in').map((q) => <PortRow key={q.name} port={q} node={id} inputs={data.inputs} />)}</div>
        <div>{ports.filter((q) => q.dir === 'out').map((q) => <PortRow key={q.name} port={q} node={id} inputs={data.inputs} />)}</div>
      </div>
      <div className="body">
        <Params spec={spec} />
      </div>
      <footer>
        <Footer spec={spec} />
      </footer>
    </div>
  );
}
