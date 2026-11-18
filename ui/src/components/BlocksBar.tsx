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

/** Reports the height of the tallest block once the blocks are measured. */
function Tallest({ onHeight }: { onHeight: (height: number) => void }) {
  // the measures are in the editor's own table of blocks, whether or not the blocks are given by the caller
  const height = useStore((s) => Math.max(0, ...[...s.nodeLookup.values()].map((n) => n.measured.height ?? 0)));
  useEffect(() => {
    if (height) onHeight(height);
  }, [height, onHeight]);
  return null;
}

/**
 * The blocks of a family, side by side: the very blocks of the graph (the same component, in a small instance of the same
 * editor), shown smaller since the tray is not subject to the graph's zoom. Nothing in them can be changed here.
 */
function Shelf({ category }: { category: Category }) {
  const { resolved } = useTheme();
  const types = KINDS_BY_CATEGORY[category];
  const box = useRef<HTMLDivElement>(null);
  const [width, setWidth] = useState(0);
  const [tallest, setTallest] = useState(0);
  useLayoutEffect(() => {
    const el = box.current;
    if (!el) return;
    const measure = () => setWidth(el.clientWidth);
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(el);
    return () => observer.disconnect();
  }, []);
  // as large as the tray allows, so that all the blocks of the family are in view
  const zoom = Math.min(0.8, Math.max(0.5, (width - 24) / (types.length * COLUMN)));
  const nodes: OmuNode[] = useMemo(
    () =>
      types.map((type, i) => {
        const id = trayId(i);
        return { id, type: 'omu', position: { x: i * COLUMN, y: 0 }, data: { spec: { id, ...defaultKind(type) } }, draggable: false, selectable: false, connectable: false };
      }),
    [types],
  );
  return (
    <div className="shelf" ref={box} style={{ height: tallest ? tallest * zoom + 20 : 240 }}>
      {width > 0 && (
        <ReactFlowProvider>
          <ReactFlow
            nodes={nodes}
            edges={[]}
            nodeTypes={nodeTypes}
            viewport={{ x: 12, y: 10, zoom }}
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
            <Tallest onHeight={setTallest} />
          </ReactFlow>
        </ReactFlowProvider>
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
