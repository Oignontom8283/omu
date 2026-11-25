import type { ReactNode } from 'react';
import { CATEGORIES, KINDS_BY_CATEGORY } from '../graph.ts';
import { parseMarkdown, type Block, type Inline, type LinkScheme } from '../richtext.ts';
import type { Category, NodeType } from '../types.ts';

/** What a click on a link to an object does. A scheme with no handler is shown as plain coloured text. */
export type DocLinks = Partial<Record<LinkScheme, (id: string) => void>> & {
  /** The flow a connection point carries (to colour its links). */
  flowOf?: (port: string) => string;
};

const categoryOfKind = (id: string): Category | undefined => CATEGORIES.find((c) => KINDS_BY_CATEGORY[c].includes(id as NodeType));

function linkClass(scheme: LinkScheme, id: string, links: DocLinks): string {
  if (scheme === 'port') return `ref flow-${links.flowOf?.(id) ?? 'audio'}`;
  if (scheme === 'type') return `ref flow-${id}`;
  if (scheme === 'setting') return 'ref setting';
  return `ref cat-${categoryOfKind(id) ?? 'processing'} kind`;
}

function Inlines({ items, links }: { items: Inline[]; links: DocLinks }) {
  return (
    <>
      {items.map((piece, i) => {
        if (piece.code) return <code key={i}>{piece.text}</code>;
        if (piece.bold) return <strong key={i}>{piece.text}</strong>;
        if (!piece.link) return <span key={i}>{piece.text}</span>;
        const { scheme, id } = piece.link;
        const go = links[scheme];
        const className = linkClass(scheme, id, links);
        return go ? (
          <button key={i} className={className} onClick={() => go(id)}>
            {piece.text}
          </button>
        ) : (
          <span key={i} className={className}>
            {piece.text}
          </span>
        );
      })}
    </>
  );
}

function BlockView({ block, links }: { block: Block; links: DocLinks }): ReactNode {
  switch (block.type) {
    case 'heading': {
      const Tag = `h${block.level}` as 'h1' | 'h2' | 'h3';
      return (
        <Tag>
          <Inlines items={block.inline} links={links} />
        </Tag>
      );
    }
    case 'paragraph':
      return (
        <p>
          <Inlines items={block.inline} links={links} />
        </p>
      );
    case 'list':
      // a list of nothing but links (see also, used by) runs along the line; a list of sentences is a list
      return (
        <ul className={block.items.every((item) => item.length === 1 && item[0].link) ? 'links' : undefined}>
          {block.items.map((item, i) => (
            <li key={i}>
              <Inlines items={item} links={links} />
            </li>
          ))}
        </ul>
      );
    case 'table':
      return (
        <div className="doc-table">
          <table>
            <thead>
              <tr>
                {block.head.map((c, i) => (
                  <th key={i}>
                    <Inlines items={c} links={links} />
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {block.rows.map((row, r) => (
                <tr key={r}>
                  {row.map((c, i) => (
                    <td key={i}>
                      <Inlines items={c} links={links} />
                    </td>
                  ))}
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      );
    case 'quote':
      return (
        <blockquote className="warning">
          <Inlines items={block.inline} links={links} />
        </blockquote>
      );
  }
}

/** A document written in the little markdown of `richtext.ts`, with its links to objects. */
export function Markdown({ source, links }: { source: string; links: DocLinks }) {
  return (
    <div className="doc">
      {parseMarkdown(source).map((block, i) => (
        <BlockView key={i} block={block} links={links} />
      ))}
    </div>
  );
}
