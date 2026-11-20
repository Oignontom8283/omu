// The versions of the graph that were applied, kept in this browser so that an earlier one can be brought back, and a draft of the
// changes not applied yet, so that a closed window or a crash loses nothing.

import type { GraphSpec, Layout } from './types.ts';

export type Version = { at: string; graph: GraphSpec; layout: Layout };
const VERSIONS = 'omu-versions';
const DRAFT = 'omu-draft';
/** How many versions are kept. */
export const KEPT = 30;

const read = <T>(key: string, fallback: T): T => {
  try {
    const raw = localStorage.getItem(key);
    return raw ? (JSON.parse(raw) as T) : fallback;
  } catch {
    return fallback;
  }
};
const write = (key: string, value: unknown): void => {
  try {
    if (value === null) localStorage.removeItem(key);
    else localStorage.setItem(key, JSON.stringify(value));
  } catch {
    /* storage may be blocked or full: the editor goes on without */
  }
};

export const loadVersions = (): Version[] => read<Version[]>(VERSIONS, []);

/** Puts `version` first, unless it is the same graph as the last one kept; only the newest `KEPT` stay. */
export function withVersion(list: Version[], version: Version): Version[] {
  if (list[0] && JSON.stringify(list[0].graph) === JSON.stringify(version.graph)) return list;
  return [version, ...list].slice(0, KEPT);
}

export function saveVersions(list: Version[]): void {
  write(VERSIONS, list);
}

export const loadDraft = (): Version | null => read<Version | null>(DRAFT, null);
export const saveDraft = (draft: Version | null): void => write(DRAFT, draft);

/** What changed between two graphs, by block: which are new, which are gone, which are not the same any more, how many links differ. */
export type Difference = { added: string[]; removed: string[]; changed: string[]; links: number };

export function differences(before: GraphSpec, after: GraphSpec): Difference {
  const was = new Map(before.nodes.map((n) => [n.id, JSON.stringify(n)]));
  const is = new Map(after.nodes.map((n) => [n.id, JSON.stringify(n)]));
  const link = (e: GraphSpec['edges'][number]) => `${e.from.node}.${e.from.port}>${e.to.node}.${e.to.port}`;
  const wasLinks = new Set(before.edges.map(link));
  const isLinks = new Set(after.edges.map(link));
  return {
    added: [...is.keys()].filter((id) => !was.has(id)),
    removed: [...was.keys()].filter((id) => !is.has(id)),
    changed: [...is.keys()].filter((id) => was.has(id) && was.get(id) !== is.get(id)),
    links: [...isLinks].filter((l) => !wasLinks.has(l)).length + [...wasLinks].filter((l) => !isLinks.has(l)).length,
  };
}

/** Whether `differences` found nothing. */
export const same = (d: Difference): boolean => d.added.length + d.removed.length + d.changed.length + d.links === 0;
