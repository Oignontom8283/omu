import { ReactFlow, ReactFlowProvider, useStore } from '@xyflow/react';
import { useEffect, useLayoutEffect, useMemo, useRef, useState, type PointerEvent as ReactPointerEvent, type ReactNode } from 'react';
import { CATEGORIES, GRID, KINDS_BY_CATEGORY, NODE_WIDTH, defaultKind } from '../graph.ts';
import { useT } from '../i18n/index.tsx';
import { useTheme } from '../theme.tsx';
import type { Category, NodeType } from '../types.ts';
import { NodeCard, type OmuNode } from './NodeCard.tsx';

const nodeTypes = { omu: NodeCard };
const COLUMN = NODE_WIDTH + 2 * GRID;
const noop = () => {};
/** The identifier of the n-th block of the tray: shaped like the ones the editor makes, so that a block shows the same empty name field. */
const trayId = (n: number) => `00000000-0000-4000-8000-${String(n).padStart(12, '0')}`;

/** What the graph does with a block dragged out of the tray: it adds it where the pointer is, then moves it with the pointer. */
export type DragIn = {
  begin: (type: NodeType, x: number, y: number) => string;
  move: (id: string, x: number, y: number) => void;
  end: (id: string) => void;
};

/** The size blocks of the tray are shown at: readable, whatever their number (the tray wraps instead of shrinking them). */
const SHELF_ZOOM = 0.85;

/** Reports the height of each block once the blocks are measured. */
function Heights({ ids, onHeights }: { ids: string[]; onHeights: (heights: number[]) => void }) {
  // the measures are in the editor's own table of blocks, whether or not the blocks are given by the caller
  const key = useStore((s) => ids.map((id) => Math.round(s.nodeLookup.get(id)?.measured.height ?? 0)).join(','));
  useEffect(() => {
    const heights = key.split(',').map(Number);
    if (heights.every((h) => h > 0)) onHeights(heights);
  }, [key, onHeights]);
  return null;
}

/**
 * The blocks of a family: the very blocks of the graph (the same component, in a small instance of the same editor), shown
 * at a size that stays readable. They are laid out in rows as wide as the tray, one row under the other when the family is
 * large (the tray scrolls when even that is too tall). Nothing in them can be changed here.
 */
function Shelf({ category }: { category: Category }) {
  const { resolved } = useTheme();
  const types = KINDS_BY_CATEGORY[category];
  const box = useRef<HTMLDivElement>(null);
  const [width, setWidth] = useState(0);
  const [heights, setHeights] = useState<number[]>([]);
  useLayoutEffect(() => {
    const el = box.current;
    if (!el) return;
    const measure = () => setWidth(el.clientWidth);
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(el);
    return () => observer.disconnect();
  }, []);
  const columns = Math.max(1, Math.floor((width - 24) / (COLUMN * SHELF_ZOOM)));
  const ids = useMemo(() => types.map((_, i) => trayId(i)), [types]);
  // each row is as tall as its tallest block
  const rowHeight = (row: number) => Math.max(120, ...types.slice(row * columns, (row + 1) * columns).map((_, k) => heights[row * columns + k] ?? 240));
  const rows = Math.ceil(types.length / columns);
  const tops = Array.from({ length: rows }, (_, r) => Array.from({ length: r }, (_, k) => rowHeight(k) + GRID).reduce((a, b) => a + b, 0));
  const total = rows ? tops[rows - 1] + rowHeight(rows - 1) : 0;
  // the same objects until the layout changes, or the editor would measure the blocks again at every turn
  const layout = tops.join(',');
  const nodes: OmuNode[] = useMemo(
    () =>
      types.map((type, i) => {
        const id = ids[i];
        return { id, type: 'omu', position: { x: (i % columns) * COLUMN, y: tops[Math.floor(i / columns)] }, data: { spec: { id, ...defaultKind(type) } }, draggable: false, selectable: false, connectable: false };
      }),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [types, ids, columns, layout],
  );
  return (
    <div className="shelf" ref={box}>
      {width > 0 && (
        <div style={{ height: total * SHELF_ZOOM + 20 }}>
          <ReactFlowProvider>
            <ReactFlow
              nodes={nodes}
              edges={[]}
              nodeTypes={nodeTypes}
              viewport={{ x: 12, y: 10, zoom: SHELF_ZOOM }}
              onViewportChange={noop}
              panOnDrag={false}
              zoomOnScroll={false}
              zoomOnPinch={false}
              zoomOnDoubleClick={false}
              preventScrolling={false}
              nodesDraggable={false}
              nodesConnectable={false}
              elementsSelectable={false}
              onNodeClick={noop}
              colorMode={resolved}
              proOptions={{ hideAttribution: true }}
            >
              <Heights ids={ids} onHeights={setHeights} />
            </ReactFlow>
          </ReactFlowProvider>
        </div>
      )}
    </div>
  );
}

/**
 * The bar of the blocks: a solid coloured button per family. Hovering one shows, under the bar and over the graph, all the
 * blocks of that family (no click needed). A block is dragged from there onto the graph: it appears as soon as the pointer
 * leaves the tray and follows the pointer until it is let go. `children` are the bar's other buttons, on its right.
 */
export function BlocksBar({ drag, children }: { drag: DragIn; children: ReactNode }) {
  const t = useT();
  const [shown, setShown] = useState<Category | null>(null);
  const [dragging, setDragging] = useState(false);
  // the copy of the block that follows the pointer while it is still over the tray (the real block only exists over the graph)
  const [ghost, setGhost] = useState<{ html: string; scale: number; x: number; y: number } | null>(null);
  const tray = useRef<HTMLDivElement>(null);
  const closing = useRef<number | undefined>(undefined);
  const cancelClose = () => window.clearTimeout(closing.current);
  const closeSoon = () => {
    cancelClose();
    closing.current = window.setTimeout(() => setShown(null), 180);
  };
  useEffect(() => cancelClose, []);

  const pick = (e: ReactPointerEvent) => {
    const block = (e.target as Element).closest('.react-flow__node');
    if (e.button !== 0 || !block) return;
    const type = shown ? KINDS_BY_CATEGORY[shown][Number(block.getAttribute('data-id')?.slice(-12))] : undefined;
    if (!type) return;
    e.preventDefault();
    // what is picked up: the block as it is drawn in the tray, with the point of it that was grabbed
    const card = block.querySelector<HTMLElement>('.node');
    const rect = card?.getBoundingClientRect();
    const copy = card && rect ? { html: card.outerHTML, scale: rect.width / (card.offsetWidth || 1), dx: e.clientX - rect.left, dy: e.clientY - rect.top } : null;
    const from = { x: e.clientX, y: e.clientY };
    let id: string | null = null;
    const move = (ev: PointerEvent) => {
      if (id) return drag.move(id, ev.clientX, ev.clientY);
      // the block is made once the pointer is out of the tray, over the graph
      if (ev.clientY > (tray.current?.getBoundingClientRect().bottom ?? 0) + 4) {
        id = drag.begin(type, ev.clientX, ev.clientY);
        setGhost(null);
        setDragging(true);
      } else if (copy && Math.hypot(ev.clientX - from.x, ev.clientY - from.y) > 4) {
        setGhost({ html: copy.html, scale: copy.scale, x: ev.clientX - copy.dx, y: ev.clientY - copy.dy });
      }
    };
    const stop = () => {
      window.removeEventListener('pointermove', move);
      window.removeEventListener('pointerup', stop);
      window.removeEventListener('pointercancel', stop);
      setDragging(false);
      setGhost(null);
      setShown(null);
      if (id) drag.end(id);
    };
    window.addEventListener('pointermove', move);
    window.addEventListener('pointerup', stop);
    window.addEventListener('pointercancel', stop);
  };

  return (
    <div className="catbar" onMouseEnter={cancelClose} onMouseLeave={closeSoon}>
      <div className="cats">
        {CATEGORIES.map((c) => (
          <button
            key={c}
            className={`cat cat-${c}${shown === c ? ' open' : ''}`}
            aria-expanded={shown === c}
            onMouseEnter={() => setShown(c)}
            onFocus={() => setShown(c)}
            onClick={() => setShown(shown === c ? null : c)}
          >
            {t(`category.${c}`)}
          </button>
        ))}
      </div>
      <span className="spacer" />
      <div className="catbar-actions" onMouseEnter={() => setShown(null)}>
        {children}
      </div>
      {ghost && (
        <div
          className="ghost"
          style={{ left: ghost.x, top: ghost.y, transform: `scale(${ghost.scale})` }}
          aria-hidden="true"
          dangerouslySetInnerHTML={{ __html: ghost.html }}
        />
      )}
      {shown && (
        <div ref={tray} className={`tray cat-${shown}${dragging ? ' dragging' : ''}`} onPointerDown={pick}>
          <p className="tray-title">
            <strong>{t(`category.${shown}`)}</strong> · {t(`category.${shown}.hint`)}
          </p>
          <Shelf key={shown} category={shown} />
        </div>
      )}
    </div>
  );
}
