import { Handle, Position, type Node, type NodeProps } from '@xyflow/react';
import { useState } from 'react';
import { useEditor } from '../context.ts';
import { KINDS, levelDb, meterFill } from '../graph.ts';
import type { DeviceRef, NodeSpec } from '../types.ts';

export type OmuNode = Node<{ spec: NodeSpec }, 'omu'>;

function Meter({ peak }: { peak: number }) {
  const db = levelDb(peak);
  return (
    <div className="meter" title={db <= -60 ? 'silence' : `${db.toFixed(0)} dB`}>
      <div className={`meter-fill${peak > 0.97 ? ' hot' : ''}`} style={{ width: `${meterFill(peak) * 100}%` }} />
    </div>
  );
}

function AppEditor({ spec }: { spec: Extract<NodeSpec, { type: 'app_capture' }> }) {
  const { apps, update } = useEditor();
  const value = 'pid' in spec.target ? String(spec.target.pid) : spec.target.process_name;
  const names = [...new Set(apps.map((a) => a.name))];
  const listId = `apps-${spec.id}`;
  return (
    <label className="field">
      <span>Program (name or process id)</span>
      <input
        className="nodrag"
        list={listId}
        value={value}
        placeholder="chrome.exe"
        onChange={(e) => {
          const v = e.target.value.trim();
          update(spec.id, { type: 'app_capture', target: /^\d+$/.test(v) ? { pid: Number(v) } : { process_name: e.target.value } });
        }}
      />
      <datalist id={listId}>
        {names.map((n) => (
          <option key={n} value={n} />
        ))}
      </datalist>
      {value && !/^\d+$/.test(value) && apps.length > 0 && !apps.some((a) => a.name.toLowerCase() === value.toLowerCase()) && (
        <small className="warn">not playing sound right now</small>
      )}
    </label>
  );
}

function refToOption(d: DeviceRef): string {
  return d === 'default' ? 'default' : 'id' in d ? `id:${d.id}` : `name:${d.name}`;
}
function optionToRef(v: string): DeviceRef {
  if (v === 'default') return 'default';
  return v.startsWith('id:') ? { id: v.slice(3) } : { name: v.slice(5) };
}

function DeviceEditor({ spec }: { spec: Extract<NodeSpec, { type: 'device_capture' | 'device_render' }> }) {
  const { devices, update } = useEditor();
  const direction = spec.type === 'device_capture' ? 'capture' : 'render';
  const list = devices.filter((d) => d.direction === direction);
  const current = refToOption(spec.device);
  const known = current === 'default' || list.some((d) => `id:${d.id}` === current);
  return (
    <label className="field">
      <span>{direction === 'capture' ? 'Recording device' : 'Playback device'}</span>
      <select className="nodrag" value={current} onChange={(e) => update(spec.id, { ...spec, device: optionToRef(e.target.value) })}>
        <option value="default">System default</option>
        {list.map((d) => (
          <option key={d.id} value={`id:${d.id}`}>
            {d.name}
          </option>
        ))}
        {!known && <option value={current}>(not found) {current.replace(/^(id|name):/, '')}</option>}
      </select>
    </label>
  );
}

function GainEditor({ spec }: { spec: Extract<NodeSpec, { type: 'gain' }> }) {
  const { update } = useEditor();
  return (
    <label className="field">
      <span>Gain: {spec.db > 0 ? '+' : ''}{spec.db} dB</span>
      <input className="nodrag" type="range" min={-60} max={24} step={0.5} value={spec.db} onChange={(e) => update(spec.id, { type: 'gain', db: Number(e.target.value) })} />
    </label>
  );
}

function MicEditor({ spec }: { spec: Extract<NodeSpec, { type: 'virtual_mic' }> }) {
  const { update } = useEditor();
  return (
    <label className="field">
      <span>Name other programs see</span>
      <input className="nodrag" value={spec.name} onChange={(e) => update(spec.id, { type: 'virtual_mic', name: e.target.value })} />
    </label>
  );
}

function IdField({ id }: { id: string }) {
  const { rename } = useEditor();
  const [draft, setDraft] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const commit = () => {
    if (draft !== null && draft !== id) setError(rename(id, draft));
    else setError(null);
    setDraft(null);
  };
  return (
    <>
      <input
        className="node-id nodrag"
        value={draft ?? id}
        spellCheck={false}
        title="Name of this node in the graph"
        onChange={(e) => setDraft(e.target.value)}
        onBlur={commit}
        onKeyDown={(e) => e.key === 'Enter' && (e.target as HTMLInputElement).blur()}
      />
      {error && <small className="warn">{error}</small>}
    </>
  );
}

export function NodeCard({ data, selected }: NodeProps<OmuNode>) {
  const { statusOf } = useEditor();
  const spec = data.spec;
  const meta = KINDS[spec.type];
  const status = statusOf(spec.id);
  const failed = status?.state === 'failed';
  return (
    <div className={`node${selected ? ' selected' : ''}${failed ? ' failed' : ''}`} style={{ ['--accent' as string]: meta.color }}>
      {meta.hasInput && <Handle id="in" type="target" position={Position.Left} />}
      <header>
        <span className="kind">{meta.label}</span>
        <IdField id={spec.id} />
      </header>
      <div className="body">
        {spec.type === 'app_capture' && <AppEditor spec={spec} />}
        {(spec.type === 'device_capture' || spec.type === 'device_render') && <DeviceEditor spec={spec} />}
        {spec.type === 'gain' && <GainEditor spec={spec} />}
        {spec.type === 'virtual_mic' && <MicEditor spec={spec} />}
        {spec.type === 'mixer' && <small className="hint">{meta.hint}</small>}
      </div>
      <footer>
        {status ? (
          <>
            <span className={`dot ${failed ? 'bad' : 'ok'}`} />
            <Meter peak={status.peak} />
            {(status.underruns > 0 || status.overruns > 0) && <small className="warn" title="Buffer underruns / overruns">{status.underruns}/{status.overruns}</small>}
          </>
        ) : (
          <small className="hint">not running</small>
        )}
        {failed && <small className="warn full">{status?.message}</small>}
      </footer>
      {meta.hasOutput && <Handle id="out" type="source" position={Position.Right} />}
    </div>
  );
}
