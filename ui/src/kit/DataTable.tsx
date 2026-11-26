import { useEffect, useLayoutEffect, useMemo, useRef, useState, type CSSProperties, type KeyboardEvent, type MouseEvent as ReactMouseEvent, type ReactNode } from 'react';
import { ContextMenu, SEPARATOR, type MenuItem } from '../components/ContextMenu.tsx';
import { useT } from '../i18n/index.tsx';
import { usePersistent } from './persist.ts';
import { clickRow, keepExisting, moveCursor, selectAll, type ListSelection } from './selectionModel.ts';

/** A column of a table: its name, its usual width, how it is drawn and, when it can be sorted, the value it is sorted by. */
export type Column<R> = {
  id: string;
  label: string;
  width: number;
  min?: number;
  align?: 'start' | 'end' | 'center';
  sort?: (row: R) => string | number;
  render: (row: R) => ReactNode;
  /** It is always shown (the name of the row). */
  fixed?: boolean;
  /** It starts hidden; the menu of the header shows it. */
  hidden?: boolean;
  /** What the header says when the pointer rests on it. */
  title?: string;
};

type Sort = { by: string; down: boolean } | null;
type Layout = { widths: Record<string, number>; hidden: string[]; shown: string[]; sort: Sort };

/** Whether a key or a click happened on something that takes it itself (a field, a button inside a row). */
export const isInteractive = (target: EventTarget | null): boolean =>
  target instanceof HTMLElement && !!target.closest('input, textarea, select, button, [contenteditable="true"], .kit-select, .kit-slider, a[href]');

/** The rows of a table in the order of its sort (the order they come in when it has none). */
function sortRows<R>(rows: R[], columns: Column<R>[], sort: Sort): R[] {
  const col = sort && columns.find((c) => c.id === sort.by);
  if (!col?.sort) return rows;
  const key = col.sort;
  const order = rows.map((r, at) => ({ r, at, k: key(r) }));
  order.sort((a, b) => {
    const c = typeof a.k === 'string' || typeof b.k === 'string' ? String(a.k).localeCompare(String(b.k), undefined, { numeric: true, sensitivity: 'base' }) : a.k - b.k;
    return (sort!.down ? -c : c) || a.at - b.at;
  });
  return order.map((o) => o.r);
}

export type DataTableProps<R> = {
  /** Where its columns, their widths and its sort are kept in this browser. */
  id: string;
  label: string;
  rows: R[];
  rowKey: (row: R) => string;
  columns: Column<R>[];
  selection: ListSelection;
  onSelection: (selection: ListSelection) => void;
  /** A double click, or Enter, on a row. */
  onActivate?: (row: R) => void;
  /** The keys the table does not use itself (Delete, F2, Space…), with the rows that are chosen. True when the key was used. */
  onKey?: (e: KeyboardEvent, rows: R[]) => boolean;
  /** The menu of a right click or of the menu key: on the chosen rows, or on the empty space (no rows). */
  onMenu?: (at: { x: number; y: number }, rows: R[]) => void;
  rowClass?: (row: R) => string;
  rowStyle?: (row: R) => CSSProperties | undefined;
  /** What is shown when there are no rows. */
  empty?: ReactNode;
  rowHeight?: number;
  initialSort?: Sort;
};

/**
 * A table the way an IDE draws one: a header that stays at the top, columns that are widened by dragging their edge, sorted by a click on
 * their name and shown or hidden from the menu of the header; rows chosen with the mouse (Ctrl, Shift) or the keyboard (arrows, Home,
 * End, Page up/down, Ctrl+A), and only the rows in view exist, so a list of thousands costs the same as a short one.
 */
export function DataTable<R>({ id, label, rows, rowKey, columns, selection, onSelection, onActivate, onKey, onMenu, rowClass, rowStyle, empty, rowHeight = 28, initialSort = null }: DataTableProps<R>) {
  const t = useT();
  const usual: Layout = { widths: {}, hidden: columns.filter((c) => c.hidden).map((c) => c.id), shown: [], sort: initialSort };
  const [layout, setLayout] = usePersistent<Layout>(`omu-table-${id}`, usual, (v) => {
    const o = v as Partial<Layout> | null;
    if (!o || typeof o !== 'object') return undefined;
    const widths = Object.fromEntries(Object.entries(o.widths ?? {}).filter(([, w]) => typeof w === 'number' && w >= 24 && w <= 2000));
    const list = (x: unknown) => (Array.isArray(x) ? x.filter((s): s is string => typeof s === 'string') : []);
    const sort = o.sort && typeof o.sort.by === 'string' ? { by: o.sort.by, down: !!o.sort.down } : null;
    return { widths, hidden: list(o.hidden), shown: list(o.shown), sort };
  });
  // a column that starts hidden is shown once the person asks for it, and the other way round
  const visible = columns.filter((c) => c.fixed || (c.hidden ? layout.shown.includes(c.id) : !layout.hidden.includes(c.id)));
  const widthOf = (c: Column<R>) => layout.widths[c.id] ?? c.width;
  // the columns keep their widths; what is left of the table is an empty column at the end, as in the tables of an IDE
  const template = `${visible.map((c) => `${widthOf(c)}px`).join(' ')} minmax(0, 1fr)`;
  const total = visible.reduce((n, c) => n + widthOf(c), 0);

  const shown = useMemo(() => sortRows(rows, columns, layout.sort), [rows, columns, layout.sort]);
  const keys = useMemo(() => shown.map(rowKey), [shown, rowKey]);
  const index = useMemo(() => new Map(keys.map((k, i) => [k, i])), [keys]);
  const picked = useMemo(() => new Set(selection.selected), [selection.selected]);
  const chosenRows = () => shown.filter((_, i) => picked.has(keys[i]));

  // rows that went away leave the selection
  useEffect(() => {
    const kept = keepExisting(keys, selection);
    if (kept !== selection) onSelection(kept);
  }, [keys, selection, onSelection]);

  const box = useRef<HTMLDivElement>(null);
  const [view, setView] = useState({ top: 0, height: 400 });
  useLayoutEffect(() => {
    const el = box.current;
    if (!el) return;
    const measure = () => setView({ top: el.scrollTop, height: el.clientHeight });
    measure();
    const watch = new ResizeObserver(measure);
    watch.observe(el);
    return () => watch.disconnect();
  }, []);
  const head = rowHeight;
  const first = Math.max(0, Math.floor((view.top - head) / rowHeight) - 8);
  const last = Math.min(shown.length, Math.ceil((view.top + view.height) / rowHeight) + 8);

  // the row under the cursor stays in view
  useEffect(() => {
    const el = box.current;
    const at = selection.focus ? index.get(selection.focus) : undefined;
    if (!el || at === undefined) return;
    const top = head + at * rowHeight;
    if (top < el.scrollTop + head) el.scrollTop = top - head;
    else if (top + rowHeight > el.scrollTop + el.clientHeight) el.scrollTop = top + rowHeight - el.clientHeight;
  }, [selection.focus, index, rowHeight, head]);

  const [headerMenu, setHeaderMenu] = useState<{ x: number; y: number } | null>(null);
  const toggleColumn = (c: Column<R>) =>
    setLayout((l) => {
      const on = visible.includes(c);
      return c.hidden ? { ...l, shown: on ? l.shown.filter((x) => x !== c.id) : [...l.shown, c.id] } : { ...l, hidden: on ? [...l.hidden, c.id] : l.hidden.filter((x) => x !== c.id) };
    });
  const headerItems: MenuItem[] = [
    ...columns.filter((c) => !c.fixed).map((c): MenuItem => ({ label: c.label, checked: visible.includes(c), run: () => toggleColumn(c) })),
    SEPARATOR,
    { label: t('table.resetColumns'), run: () => setLayout(usual) },
  ];

  const sortBy = (c: Column<R>) => {
    if (!c.sort) return;
    // ascending, descending, then back to the order the rows come in
    setLayout((l) => ({ ...l, sort: l.sort?.by !== c.id ? { by: c.id, down: false } : l.sort.down ? null : { by: c.id, down: true } }));
  };
  const resize = (c: Column<R>, e: ReactMouseEvent) => {
    e.preventDefault();
    e.stopPropagation();
    const from = widthOf(c);
    const at = e.clientX;
    const move = (ev: MouseEvent) => setLayout((l) => ({ ...l, widths: { ...l.widths, [c.id]: Math.max(c.min ?? 40, Math.round(from + ev.clientX - at)) } }));
    const stop = () => {
      window.removeEventListener('mousemove', move);
      window.removeEventListener('mouseup', stop);
      document.body.classList.remove('resizing-col');
    };
    window.addEventListener('mousemove', move);
    window.addEventListener('mouseup', stop);
    document.body.classList.add('resizing-col');
  };

  const pointer = (e: ReactMouseEvent, key: string) => {
    if (!isInteractive(e.target)) box.current?.focus({ preventScroll: true });
    if (e.button === 2) {
      if (!picked.has(key)) onSelection(clickRow(keys, selection, key));
      return;
    }
    if (e.button !== 0) return;
    // a click on a field or a button inside a row chooses the row too, unless it is already among the chosen ones
    if (isInteractive(e.target) && picked.has(key)) return;
    onSelection(clickRow(keys, selection, key, { ctrl: e.ctrlKey || e.metaKey, shift: e.shiftKey && !isInteractive(e.target) }));
  };
  const menuOnRows = (x: number, y: number, list: R[]) => onMenu?.({ x, y }, list);

  const key = (e: KeyboardEvent<HTMLDivElement>) => {
    if (isInteractive(e.target)) return;
    const mods = { ctrl: e.ctrlKey || e.metaKey, shift: e.shiftKey };
    const page = Math.max(1, Math.floor((box.current?.clientHeight ?? 400) / rowHeight) - 1);
    const move = (step: number) => onSelection(moveCursor(keys, selection, step, mods));
    switch (e.key) {
      case 'ArrowDown':
        move(1);
        break;
      case 'ArrowUp':
        move(-1);
        break;
      case 'PageDown':
        move(page);
        break;
      case 'PageUp':
        move(-page);
        break;
      case 'Home':
        move(-Infinity);
        break;
      case 'End':
        move(Infinity);
        break;
      case 'a':
      case 'A':
        if (!mods.ctrl || mods.shift || e.altKey) return void (onKey?.(e, chosenRows()) && e.preventDefault());
        onSelection(selectAll(keys, selection));
        break;
      case 'Enter': {
        const at = selection.focus ? index.get(selection.focus) : undefined;
        if (at === undefined || !onActivate) return void (onKey?.(e, chosenRows()) && e.preventDefault());
        onActivate(shown[at]);
        break;
      }
      case 'Escape':
        if (selection.selected.length === 0) return;
        onSelection({ selected: [], focus: selection.focus, anchor: null });
        break;
      case 'ContextMenu':
      case 'F10': {
        if (e.key === 'F10' && !e.shiftKey) return;
        const at = selection.focus ? index.get(selection.focus) : undefined;
        const r = box.current?.getBoundingClientRect();
        if (!r) return;
        const y = at === undefined ? r.top + head : r.top + head + at * rowHeight - (box.current?.scrollTop ?? 0) + rowHeight;
        menuOnRows(r.left + 24, Math.min(y, r.bottom - 8), chosenRows());
        break;
      }
      default:
        if (!onKey?.(e, chosenRows())) return;
    }
    e.preventDefault();
  };

  const active = selection.focus && index.has(selection.focus) ? `${id}-row-${index.get(selection.focus)}` : undefined;
  const align = (c: Column<R>) => (c.align === 'end' ? ' end' : c.align === 'center' ? ' center' : '');
  const out: ReactNode[] = [];
  for (let i = first; i < last; i++) {
    const r = shown[i];
    const k = keys[i];
    const on = picked.has(k);
    out.push(
      <div
        key={k}
        id={`${id}-row-${i}`}
        role="row"
        aria-rowindex={i + 2}
        aria-selected={on}
        className={`dt-row${on ? ' selected' : ''}${selection.focus === k ? ' cursor' : ''}${i % 2 ? ' odd' : ''} ${rowClass?.(r) ?? ''}`}
        style={{ top: head + i * rowHeight, height: rowHeight, gridTemplateColumns: template, ...rowStyle?.(r) }}
        onMouseDown={(e) => pointer(e, k)}
        onDoubleClick={(e) => !isInteractive(e.target) && onActivate?.(r)}
        onContextMenu={(e) => {
          e.preventDefault();
          e.stopPropagation();
          menuOnRows(e.clientX, e.clientY, picked.has(k) ? chosenRows() : [r]);
        }}
      >
        {visible.map((c) => (
          <div key={c.id} role="gridcell" className={`dt-cell${align(c)}`}>
            {c.render(r)}
          </div>
        ))}
        <div className="dt-cell dt-fill" aria-hidden />
      </div>,
    );
  }

  return (
    <div
      ref={box}
      className="dt"
      role="grid"
      aria-label={label}
      aria-multiselectable
      aria-rowcount={shown.length + 1}
      aria-activedescendant={active}
      tabIndex={0}
      onKeyDown={key}
      onScroll={(e) => setView({ top: e.currentTarget.scrollTop, height: e.currentTarget.clientHeight })}
      onMouseDown={(e) => {
        // a click on the empty space under the rows chooses nothing
        if (e.target === e.currentTarget || (e.target as HTMLElement).classList.contains('dt-space')) {
          box.current?.focus({ preventScroll: true });
          if (e.button === 0 && !e.ctrlKey && !e.shiftKey) onSelection({ selected: [], focus: null, anchor: null });
        }
      }}
      onContextMenu={(e) => {
        e.preventDefault();
        menuOnRows(e.clientX, e.clientY, []);
      }}
    >
      <div className="dt-space" style={{ minWidth: total, height: head + Math.max(shown.length * rowHeight, 1) }}>
        <div
          className="dt-head"
          role="row"
          aria-rowindex={1}
          style={{ gridTemplateColumns: template, height: head }}
          onContextMenu={(e) => {
            e.preventDefault();
            e.stopPropagation();
            setHeaderMenu({ x: e.clientX, y: e.clientY });
          }}
        >
          {visible.map((c) => {
            const sorted = layout.sort?.by === c.id ? (layout.sort.down ? 'descending' : 'ascending') : undefined;
            return (
              <div key={c.id} role="columnheader" aria-sort={c.sort ? (sorted ?? 'none') : undefined} className={`dt-hcell${align(c)}${c.sort ? ' sortable' : ''}`} title={c.title ?? c.label} onClick={() => sortBy(c)}>
                <span className="dt-hlabel">{c.label}</span>
                {sorted && <span className="dt-sort" aria-hidden>{sorted === 'descending' ? '▼' : '▲'}</span>}
                <span className="dt-grip" aria-hidden onMouseDown={(e) => resize(c, e)} onClick={(e) => e.stopPropagation()} onDoubleClick={() => setLayout((l) => ({ ...l, widths: Object.fromEntries(Object.entries(l.widths).filter(([k]) => k !== c.id)) }))} />
              </div>
            );
          })}
          <div className="dt-hcell dt-fill" aria-hidden />
        </div>
        {out}
      </div>
      {shown.length === 0 && empty && <div className="dt-empty">{empty}</div>}
      {headerMenu && <ContextMenu x={headerMenu.x} y={headerMenu.y} items={headerItems} onClose={() => setHeaderMenu(null)} />}
    </div>
  );
}
