import { Highlight } from '../kit/Panes.tsx';
import { TreeView, type TreeNode } from '../kit/TreeView.tsx';
import { subCategories, textOf, topCategories, type Snapshot, type Translate } from './registry.ts';

type Props = {
  state: Snapshot;
  t: Translate;
  label: string;
  /** The page (or heading) the list is on. */
  selected?: string;
  /** The headings that are open. */
  expanded: string[];
  onSelect: (id: string) => void;
  onToggle: (id: string) => void;
  /** What each page shows on its right: how many settings a search finds in it, or how many were changed. */
  badges: Record<string, { count: number; changed: boolean } | undefined>;
  /** The search, to mark its words in the names; while one is typed, the pages it finds nothing in are faded. */
  query: string;
  focusKey?: number;
};

/**
 * The categories of the settings as the tree of a code editor's settings: a heading opens and closes and holds its pages, indented under
 * it along a guide. A page shows how many of its settings were changed, or how many a search finds in it.
 */
export function SettingsTree({ state, t, label, selected, expanded, onSelect, onToggle, badges, query, focusKey }: Props) {
  const searching = query.trim() !== '';
  const node = (id: string, title: string, children?: TreeNode[], icon?: TreeNode['icon']): TreeNode => {
    const badge = badges[id];
    return {
      id,
      text: title,
      label: <Highlight text={title} query={query} />,
      icon,
      children,
      dim: searching && !badge?.count,
      badge: badge && badge.count > 0 ? <span className={badge.changed ? 'changed' : ''}>{badge.count}</span> : undefined,
    };
  };
  const nodes: TreeNode[] = topCategories(state, t).map((top) => {
    const pages = subCategories(state, top.id, t).map((p) => node(p.id, textOf(t, p.plain, p.title)));
    return node(top.id, textOf(t, top.plain, top.title), pages.length > 0 ? pages : undefined, top.icon);
  });
  return (
    <TreeView
      nodes={nodes}
      label={label}
      selected={selected ? [selected] : []}
      expanded={expanded}
      onToggle={onToggle}
      onSelect={(id) => onSelect(id)}
      onActivate={(id) => onSelect(id)}
      focusKey={focusKey}
    />
  );
}
