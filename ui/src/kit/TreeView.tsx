import { useEffect, useRef, useState, type KeyboardEvent, type ReactNode } from 'react';
import { Icon, type IconName } from '../components/Icon.tsx';
import { isInteractive } from './DataTable.tsx';

/** A line of a tree: what it shows, and the lines under it. */
export type TreeNode = {
  id: string;
  /** Its name: what typing its first letters finds, and what renaming starts from. */
  text: string;
  /** What is drawn instead of the bare name (the name with the words of a search marked). */
  label?: ReactNode;
  icon?: IconName;
  /** Something small on the right: a count, a dot. */
  badge?: ReactNode;
  title?: string;
  className?: string;
  /** Shown faded (nothing in it matches the search). */
  dim?: boolean;
  children?: TreeNode[];
};

type Props = {
  nodes: TreeNode[];
  label: string;
  /** The lines that are chosen (one, in most trees). */
  selected: string[];
  /** The lines that have their children shown. */
  expanded: string[];
  onToggle: (id: string) => void;
  onSelect: (id: string, e: { ctrl: boolean; shift: boolean }) => void;
  /** A double click or Enter on a line (a line with children opens or closes when nothing is given). */
  onActivate?: (id: string) => void;
  onMenu?: (at: { x: number; y: number }, id: string | null) => void;
  /** The keys the tree does not use itself (Delete, F2…). True when the key was used. */
  onKey?: (e: KeyboardEvent, id: string) => boolean;
  /** The line whose name is being changed, and what happens to the new name (null: given up). */
  renaming?: string | null;
  onRename?: (id: string, text: string | null) => void;
  /** Changes when the tree must take the focus (the page's shortcut). */
  focusKey?: number;
};

type Line = { node: TreeNode; level: number; parent: string | null; open: boolean };

/**
 * A tree like the file explorer of a code editor: lines that open and close with their arrow, indented along a guide, one tab stop for
 * the whole tree (the arrows move through it: up and down, right opens or goes into a line, left closes it or goes back to its parent),
 * Enter chooses, typing the first letters of a name goes to it, and a name can be changed right in the tree.
 */
export function TreeView({ nodes, label, selected, expanded, onToggle, onSelect, onActivate, onMenu, onKey, renaming, onRename, focusKey }: Props) {
  const rows = useRef(new Map<string, HTMLElement>());
  const tree = useRef<HTMLDivElement>(null);
  const [cursor, setCursor] = useState<string | null>(null);
  const typed = useRef({ text: '', at: 0 });
  // Escape gives up a new name: the blur that follows must not keep it
  const givenUp = useRef(false);

  const lines: Line[] = [];
  const walk = (list: TreeNode[], level: number, parent: string | null) => {
    for (const node of list) {
      const open = !!node.children?.length && expanded.includes(node.id);
      lines.push({ node, level, parent, open });
      if (open) walk(node.children!, level + 1, node.id);
    }
  };
  walk(nodes, 1, null);
  const ids = lines.map((l) => l.node.id);
  const stop = cursor && ids.includes(cursor) ? cursor : (selected.find((s) => ids.includes(s)) ?? ids[0]);

  const focusOn = (id: string | null | undefined) => {
    if (!id) return;
    setCursor(id);
    rows.current.get(id)?.focus();
  };
  useEffect(() => {
    if (focusKey) focusOn(stop);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [focusKey]);
  // the chosen line stays in view (chosen from elsewhere: the list scrolled, a search)
  useEffect(() => {
    const id = selected[0];
    if (id) rows.current.get(id)?.scrollIntoView({ block: 'nearest' });
  }, [selected]);

  const press = (e: KeyboardEvent, at: number) => {
    if (isInteractive(e.target)) return;
    const line = lines[at];
    const id = line.node.id;
    const has = !!line.node.children?.length;
    const mods = { ctrl: e.ctrlKey || e.metaKey, shift: e.shiftKey };
    const goSelect = (to: string | undefined) => {
      if (!to) return;
      focusOn(to);
      if (!mods.ctrl) onSelect(to, mods);
    };
    switch (e.key) {
      case 'ArrowDown':
        goSelect(ids[Math.min(at + 1, ids.length - 1)]);
        break;
      case 'ArrowUp':
        goSelect(ids[Math.max(at - 1, 0)]);
        break;
      case 'Home':
        goSelect(ids[0]);
        break;
      case 'End':
        goSelect(ids[ids.length - 1]);
        break;
      case 'ArrowRight':
        if (has && !line.open) onToggle(id);
        else if (has) goSelect(ids[at + 1]);
        break;
      case 'ArrowLeft':
        if (has && line.open) onToggle(id);
        else goSelect(line.parent ?? undefined);
        break;
      case 'Enter':
        if (onActivate) onActivate(id);
        else if (has) onToggle(id);
        else onSelect(id, mods);
        break;
      case ' ':
        onSelect(id, mods);
        break;
      case 'ContextMenu':
      case 'F10': {
        if (e.key === 'F10' && !e.shiftKey) return;
        const r = rows.current.get(id)?.getBoundingClientRect();
        if (r) onMenu?.({ x: r.left + 24, y: r.bottom }, id);
        break;
      }
      default: {
        if (onKey?.(e, id)) break;
        // typing the first letters of a name goes to it
        if (e.key.length !== 1 || e.ctrlKey || e.altKey || e.metaKey || !/\S/.test(e.key)) return;
        const now = Date.now();
        typed.current = { text: (now - typed.current.at < 700 ? typed.current.text : '') + e.key.toLowerCase(), at: now };
        const want = typed.current.text;
        const order = [...lines.slice(at + (want.length === 1 ? 1 : 0)), ...lines.slice(0, at + 1)];
        const found = order.find((l) => l.node.text.toLowerCase().startsWith(want));
        if (found) goSelect(found.node.id);
      }
    }
    e.preventDefault();
  };

  return (
    <div
      ref={tree}
      role="tree"
      aria-label={label}
      aria-multiselectable={selected.length > 1 || undefined}
      className="tree"
      onContextMenu={(e) => {
        if (e.target !== e.currentTarget) return;
        e.preventDefault();
        onMenu?.({ x: e.clientX, y: e.clientY }, null);
      }}
    >
      {lines.map((line, at) => {
        const { node, level, open } = line;
        const has = !!node.children?.length;
        const on = selected.includes(node.id);
        return (
          <div
            key={node.id}
            ref={(el) => {
              if (el) rows.current.set(node.id, el);
              else rows.current.delete(node.id);
            }}
            role="treeitem"
            aria-level={level}
            aria-expanded={has ? open : undefined}
            aria-selected={on}
            tabIndex={node.id === stop ? 0 : -1}
            title={node.title}
            className={`tree-row${on ? ' selected' : ''}${node.dim ? ' dim' : ''} ${node.className ?? ''}`}
            style={{ paddingInlineStart: 6 + (level - 1) * 14 }}
            onFocus={(e) => e.target === e.currentTarget && setCursor(node.id)}
            onMouseDown={(e) => {
              if (e.button === 2 && !on) onSelect(node.id, { ctrl: false, shift: false });
            }}
            onClick={(e) => {
              if (isInteractive(e.target)) return;
              setCursor(node.id);
              onSelect(node.id, { ctrl: e.ctrlKey || e.metaKey, shift: e.shiftKey });
            }}
            onDoubleClick={(e) => {
              if (isInteractive(e.target)) return;
              if (onActivate) onActivate(node.id);
              else if (has) onToggle(node.id);
            }}
            onKeyDown={(e) => press(e, at)}
            onContextMenu={(e) => {
              e.preventDefault();
              e.stopPropagation();
              onMenu?.({ x: e.clientX, y: e.clientY }, node.id);
            }}
          >
            {/* the guides of the levels above */}
            {Array.from({ length: level - 1 }, (_, i) => (
              <span key={i} className="tree-guide" style={{ insetInlineStart: 12 + i * 14 }} aria-hidden />
            ))}
            <span
              className={`tree-twisty${has ? '' : ' leaf'}${open ? ' open' : ''}`}
              aria-hidden
              onClick={(e) => {
                if (!has) return;
                e.stopPropagation();
                onToggle(node.id);
              }}
            >
              {has && <Icon name="chevron" size={12} />}
            </span>
            {node.icon && <Icon name={node.icon} size={14} />}
            {renaming === node.id ? (
              <input
                className="tree-rename"
                autoFocus
                defaultValue={node.text}
                aria-label={node.text}
                onFocus={(e) => {
                  givenUp.current = false;
                  e.currentTarget.select();
                }}
                onClick={(e) => e.stopPropagation()}
                onBlur={(e) => !givenUp.current && onRename?.(node.id, e.currentTarget.value)}
                onKeyDown={(e) => {
                  e.stopPropagation();
                  if (e.key === 'Enter') {
                    e.currentTarget.blur();
                    // the line takes the focus back, so the keyboard goes on in the tree
                    window.setTimeout(() => focusOn(node.id), 0);
                  }
                  if (e.key === 'Escape') {
                    givenUp.current = true;
                    onRename?.(node.id, null);
                    window.setTimeout(() => focusOn(node.id), 0);
                  }
                }}
              />
            ) : (
              <span className="tree-label">{node.label ?? node.text}</span>
            )}
            {node.badge !== undefined && node.badge !== null && <span className="tree-badge">{node.badge}</span>}
          </div>
        );
      })}
    </div>
  );
}
