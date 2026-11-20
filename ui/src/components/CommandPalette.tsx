import { useEffect, useMemo, useRef, useState, type KeyboardEvent, type PointerEvent as ReactPointerEvent } from 'react';
import { fuzzyMatch, rank } from '../fuzzy.ts';
import { useT } from '../i18n/index.tsx';
import type { Category } from '../types.ts';
import { Icon, type IconName } from './Icon.tsx';

/** One thing the palette can do: run a command, go to a setting, add a block. */
export type PaletteItem = {
  id: string;
  /** A command, a setting, a kind of block to add, or a block that is in the graph. */
  kind: 'command' | 'setting' | 'block' | 'node';
  label: string;
  /** The family of a block: its colour. What is not a block has an icon instead. */
  family?: Category;
  icon?: IconName;
  /** Other words that find it. */
  keywords?: string;
  /** The keys that do it, shown beside it. */
  shortcut?: string;
  run: () => void;
};

/** Where the bar was last put: it opens there again, like a window. */
let lastPosition: { x: number; y: number } | null = null;

const WIDTH = 600;
/** How near (in pixels) the bar has to come to a guide to be pulled onto it. */
const SNAP = 14;

const barBottom = (selector: string): number | null => {
  const el = document.querySelector(selector);
  return el ? el.getBoundingClientRect().bottom : null;
};

/** Where it opens when it was never moved: in the middle, right under the bar of actions. */
function startPosition(): { x: number; y: number } {
  const width = Math.min(WIDTH, window.innerWidth - 16);
  return { x: (window.innerWidth - width) / 2, y: (barBottom('.catbar') ?? barBottom('.topbar') ?? 44) + 4 };
}

/**
 * The places the bar is pulled to while it is moved, as guides: the middle of the window, its margins, the line under the
 * top bar and under the bar of actions, the middle of the height.
 */
function guides(width: number, height: number) {
  const xs = [(window.innerWidth - width) / 2, 8, window.innerWidth - width - 8];
  const under = (selector: string): number[] => {
    const bottom = barBottom(selector);
    return bottom === null ? [] : [bottom + 4];
  };
  const ys = [...under('.topbar'), ...under('.catbar'), (window.innerHeight - height) / 2];
  return { xs, ys };
}

/** The nearest guide within reach, or null. */
const nearest = (value: number, among: number[]): number | null => {
  let best: number | null = null;
  for (const g of among) if (Math.abs(g - value) <= SNAP && (best === null || Math.abs(g - value) < Math.abs(best - value))) best = g;
  return best;
};

const KIND_ORDER: PaletteItem['kind'][] = ['command', 'node', 'setting', 'block'];

/** A first character that narrows the search to one sort of thing, as in a code editor. */
const PREFIXES: Record<string, PaletteItem['kind']> = { '>': 'command', '@': 'node', '#': 'setting', '+': 'block' };

function Highlighted({ text, indices }: { text: string; indices: number[] }) {
  const on = new Set(indices);
  return (
    <>
      {[...text].map((c, i) => (on.has(i) ? <b key={i}>{c}</b> : c))}
    </>
  );
}

/**
 * The search bar over the editor, in the manner of the quick-open bar of a code editor: type to find a command, a setting or a
 * block (the letters only have to appear in order), the best result first; Enter does it. A block shows the colour of its
 * family, anything else an icon. It can be moved by its edges, and opens where it was left. With `mode` "block" it only
 * offers the blocks to add.
 */
export function CommandPalette({ items, mode, initial = '', onClose }: { items: PaletteItem[]; mode: 'all' | 'block'; initial?: string; onClose: () => void }) {
  const t = useT();
  const [query, setQuery] = useState(initial);
  const [active, setActive] = useState(0);
  const [position, setPosition] = useState(() => lastPosition ?? startPosition());
  // the guides shown while it is moved: a vertical line (x) and a horizontal one (y)
  const [shown, setShown] = useState<{ x?: number; y?: number } | null>(null);
  const box = useRef<HTMLDivElement>(null);
  const list = useRef<HTMLUListElement>(null);

  const results = useMemo(() => {
    const only = mode === 'block' ? 'block' : PREFIXES[query[0] ?? ''];
    const text = only && query[0] in PREFIXES && mode !== 'block' ? query.slice(1) : query;
    const pool = only ? items.filter((i) => i.kind === only) : items;
    const found = rank(text, pool, (i) => `${i.label} ${i.keywords ?? ''}`)
      .slice(0, 60)
      // the letters are shown on the name only: those found in the other words are not marked
      .map(({ item }) => ({ item, match: fuzzyMatch(text, item.label) }));
    // with nothing typed, the things of a sort stay together
    return text === '' ? found.sort((a, b) => KIND_ORDER.indexOf(a.item.kind) - KIND_ORDER.indexOf(b.item.kind)) : found;
  }, [items, mode, query]);

  useEffect(() => setActive(0), [query, mode]);
  useEffect(() => {
    list.current?.children[active]?.scrollIntoView({ block: 'nearest' });
  }, [active]);
  // a click anywhere else, or the window losing the focus, puts it away
  useEffect(() => {
    window.addEventListener('blur', onClose);
    return () => window.removeEventListener('blur', onClose);
  }, [onClose]);

  const choose = (item: PaletteItem) => {
    onClose();
    item.run();
  };
  const onKey = (e: KeyboardEvent) => {
    if (e.key === 'Escape') {
      e.preventDefault();
      onClose();
    } else if (e.key === 'ArrowDown') {
      e.preventDefault();
      setActive((a) => Math.min(results.length - 1, a + 1));
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      setActive((a) => Math.max(0, a - 1));
    } else if (e.key === 'Enter') {
      e.preventDefault();
      const picked = results[active]?.item;
      if (picked) choose(picked);
    }
  };

  // moved by what is not the field or the list: the edges and the grip
  const grab = (e: ReactPointerEvent) => {
    if (e.button !== 0 || (e.target as Element).closest('input, ul')) return;
    e.preventDefault();
    const from = { x: e.clientX - position.x, y: e.clientY - position.y };
    const move = (ev: PointerEvent) => {
      let x = Math.max(0, Math.min(window.innerWidth - 120, ev.clientX - from.x));
      let y = Math.max(0, Math.min(window.innerHeight - 60, ev.clientY - from.y));
      const size = box.current?.getBoundingClientRect();
      const lines: { x?: number; y?: number } = {};
      // pulled onto the guides, unless Alt is held
      if (size && !ev.altKey) {
        const g = guides(size.width, size.height);
        const gx = nearest(x, g.xs);
        const gy = nearest(y, g.ys);
        if (gx !== null) {
          x = gx;
          lines.x = gx === g.xs[0] ? window.innerWidth / 2 : gx === g.xs[1] ? gx : gx + size.width;
        }
        if (gy !== null) {
          y = gy;
          lines.y = gy;
        }
      }
      lastPosition = { x, y };
      setPosition(lastPosition);
      setShown(lines);
    };
    const stop = () => {
      setShown(null);
      window.removeEventListener('pointermove', move);
      window.removeEventListener('pointerup', stop);
      window.removeEventListener('pointercancel', stop);
    };
    window.addEventListener('pointermove', move);
    window.addEventListener('pointerup', stop);
    window.addEventListener('pointercancel', stop);
  };

  return (
    <>
      <div className="palette-away" onPointerDown={onClose} />
      {shown?.x !== undefined && <div className="palette-guide vertical" style={{ left: shown.x }} aria-hidden="true" />}
      {shown?.y !== undefined && <div className="palette-guide horizontal" style={{ top: shown.y }} aria-hidden="true" />}
      <div ref={box} className="palette" role="dialog" aria-label={t('palette.title')} style={{ left: position.x, top: position.y }} onPointerDown={grab} onKeyDown={onKey}>
        <div className="palette-grip" aria-hidden="true" />
        <div className="palette-head">
          <input
            autoFocus
            className="palette-input"
            role="combobox"
            aria-expanded="true"
            aria-controls="palette-list"
            aria-activedescendant={results[active] ? `palette-${results[active].item.id}` : undefined}
            placeholder={t(mode === 'block' ? 'palette.placeholderBlock' : 'palette.placeholder')}
            value={query}
            spellCheck={false}
            onChange={(e) => setQuery(e.target.value)}
          />
        </div>
        <ul id="palette-list" ref={list} className="palette-list" role="listbox">
          {results.length === 0 && <li className="palette-none">{t('palette.none')}</li>}
          {results.map(({ item, match }, i) => (
            <li
              key={item.id}
              id={`palette-${item.id}`}
              role="option"
              aria-selected={i === active}
              className={`palette-item${i === active ? ' active' : ''}`}
              onPointerMove={() => setActive(i)}
              onClick={() => choose(item)}
            >
              {item.family ? <span className={`swatch cat-${item.family}`} aria-hidden="true" /> : <Icon name={item.icon ?? (item.kind === 'setting' ? 'gear' : 'command')} />}
              <span className="palette-label">
                <Highlighted text={item.label} indices={match?.indices ?? []} />
              </span>
              {item.shortcut ? <kbd className="palette-keys">{item.shortcut}</kbd> : mode === 'all' && <span className="palette-kind">{t(`palette.kind.${item.kind}`)}</span>}
            </li>
          ))}
        </ul>
      </div>
    </>
  );
}
