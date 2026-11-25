import { useT } from '../i18n/index.tsx';
import type { LinkScheme } from '../richtext.ts';
import { FloatingWindow } from './FloatingWindow.tsx';
import { Markdown, type DocLinks } from './Markdown.tsx';
import { useWindowStack } from './windows.ts';

/** What an information window shows: a document in markdown. */
export type Info = { source: string; /** The family colour of the window (`cat-…`), if it has one. */ className?: string };

/**
 * The information windows of a page (the "i" next to a setting, a module…): floating windows like the documentation of the blocks, several
 * open at once, the last one touched in front. `resolve` says what a key is about (null when it does not exist any more: its window closes by
 * itself); `routes` turns a link written in a document (`[Language](setting:language)`) into the key of the window that link opens.
 */
export function useInfoWindows(resolve: (key: string) => Info | null, routes: Partial<Record<LinkScheme, (id: string) => string>>) {
  const stack = useWindowStack<string>();
  const t = useT();
  const { open } = stack;
  const windows = stack.list.map((w, order) => {
    const info = resolve(w.key);
    if (!info) return null;
    const links: DocLinks = {};
    for (const [scheme, route] of Object.entries(routes) as [LinkScheme, (id: string) => string][]) links[scheme] = (id) => open(route(id), { x: w.x + 28, y: w.y + 28 });
    return (
      <FloatingWindow
        key={w.key}
        className={info.className}
        title={t('help.title')}
        x={w.x}
        y={w.y}
        order={order}
        onMove={(x, y) => stack.move(w.key, x, y)}
        onFocus={() => stack.front(w.key)}
        onClose={() => stack.close(w.key)}
      >
        <Markdown source={info.source} links={links} />
      </FloatingWindow>
    );
  });
  return { open, windows };
}
