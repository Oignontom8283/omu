import { useRef, useState, type KeyboardEvent } from 'react';
import { Icon } from '../components/Icon.tsx';
import { subCategories, textOf, topCategories, type SettingCategory, type Snapshot, type Translate } from './registry.ts';

type Item = { category: SettingCategory; level: 1 | 2; /** It has pages under it: it opens and closes. */ heading: boolean };

type Props = {
  state: Snapshot;
  t: Translate;
  label: string;
  /** The page that is shown (none while a search is). */
  selected?: string;
  /** The headings that are closed. */
  closed: string[];
  onSelect: (id: string) => void;
  onToggle: (id: string) => void;
};

/**
 * The categories of the settings as a tree, the way a file explorer or the settings of a code editor show theirs: a heading opens and
 * closes (with its arrow) and holds its pages, which are indented under it along a line. It is one tab stop; the arrows move through it
 * (up and down between lines, right opens a heading, left closes it or goes back up to it), Enter chooses a page.
 */
export function SettingsTree({ state, t, label, selected, closed, onSelect, onToggle }: Props) {
  const rows = useRef(new Map<string, HTMLElement>());
  const [focused, setFocused] = useState<string | null>(null);

  // the lines that are on screen, in order: a heading, then its pages when it is open
  const items: Item[] = topCategories(state, t).flatMap((top) => {
    const pages = subCategories(state, top.id, t);
    const head: Item = { category: top, level: 1, heading: pages.length > 0 };
    return head.heading && !closed.includes(top.id) ? [head, ...pages.map((p): Item => ({ category: p, level: 2, heading: false }))] : [head];
  });
  const ids = items.map((i) => i.category.id);
  // the line that can be reached with the Tab key: the one that has the focus, else the page that is shown, else the first
  const stop = focused && ids.includes(focused) ? focused : selected && ids.includes(selected) ? selected : ids[0];

  const focusOn = (id: string | undefined) => {
    if (!id) return;
    setFocused(id);
    rows.current.get(id)?.focus();
  };
  const press = (e: KeyboardEvent, at: number) => {
    const item = items[at];
    const id = item.category.id;
    const open = !closed.includes(id);
    switch (e.key) {
      case 'ArrowDown':
        focusOn(ids[Math.min(at + 1, ids.length - 1)]);
        break;
      case 'ArrowUp':
        focusOn(ids[Math.max(at - 1, 0)]);
        break;
      case 'Home':
        focusOn(ids[0]);
        break;
      case 'End':
        focusOn(ids[ids.length - 1]);
        break;
      case 'ArrowRight':
        if (item.heading && !open) onToggle(id);
        else if (item.heading) focusOn(ids[at + 1]);
        break;
      case 'ArrowLeft':
        if (item.heading && open) onToggle(id);
        else if (item.level === 2) focusOn(item.category.parent);
        break;
      case 'Enter':
      case ' ':
        if (item.heading) onToggle(id);
        else onSelect(id);
        break;
      default:
        return;
    }
    e.preventDefault();
  };

  return (
    <div role="tree" aria-label={label} className="stree">
      {items.map((item, at) => {
        const c = item.category;
        const open = !closed.includes(c.id);
        const on = !item.heading && c.id === selected;
        return (
          <div
            key={c.id}
            ref={(el) => {
              if (el) rows.current.set(c.id, el);
              else rows.current.delete(c.id);
            }}
            role="treeitem"
            aria-level={item.level}
            aria-expanded={item.heading ? open : undefined}
            aria-selected={item.heading ? undefined : on}
            tabIndex={c.id === stop ? 0 : -1}
            className={`stree-row level-${item.level}${item.heading ? ' heading' : ''}${on ? ' on' : ''}`}
            onFocus={() => setFocused(c.id)}
            onClick={() => (item.heading ? onToggle(c.id) : onSelect(c.id))}
            onKeyDown={(e) => press(e, at)}
          >
            {item.heading && <Icon name="chevron" size={12} />}
            {item.heading && c.icon && <Icon name={c.icon} size={15} />}
            <span className="stree-label">{textOf(t, c.plain, c.title)}</span>
          </div>
        );
      })}
    </div>
  );
}
