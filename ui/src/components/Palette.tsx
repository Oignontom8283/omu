import { KINDS, NODE_TYPES } from '../graph.ts';
import type { NodeType } from '../types.ts';

/** The nodes that can be added to the graph. */
export function Palette({ onAdd }: { onAdd: (type: NodeType) => void }) {
  return (
    <aside className="palette">
      <h2>Add</h2>
      {NODE_TYPES.map((t) => (
        <button key={t} onClick={() => onAdd(t)} title={KINDS[t].hint} style={{ ['--accent' as string]: KINDS[t].color }}>
          <span className="swatch" />
          <span>
            {KINDS[t].label}
            <small>{KINDS[t].hint}</small>
          </span>
        </button>
      ))}
    </aside>
  );
}
