/**
 * The little markdown the documentation of the blocks is written in (and translated in): headings (`#`, `##`, `###`),
 * paragraphs, lists (`- `), tables (`| a | b |`), quotes (`> `, shown as warnings) and, inside the text, `code`, **bold**
 * and links to objects: `[sound in](port:in)` (a connection point of the block), `[Gain](kind:gain)` (a kind of block),
 * `[Sound](type:audio)` (a kind of flow), `[Language](setting:language)` (a setting of the program). Anything that is named in the documentation is such a link.
 */

export type LinkScheme = 'port' | 'kind' | 'type' | 'setting';

export type Inline = { text: string; code?: boolean; bold?: boolean; link?: { scheme: LinkScheme; id: string } };

export type Block =
  | { type: 'heading'; level: 1 | 2 | 3; inline: Inline[] }
  | { type: 'paragraph'; inline: Inline[] }
  | { type: 'list'; items: Inline[][] }
  | { type: 'table'; head: Inline[][]; rows: Inline[][][] }
  | { type: 'quote'; inline: Inline[] };

const INLINE = /\[([^\]]+)\]\((port|kind|type|setting):([A-Za-z0-9_.:-]+)\)|`([^`]+)`|\*\*([^*]+)\*\*/g;

/** Cuts a line of text into its pieces: plain text, code, bold, and links. */
export function parseInline(source: string): Inline[] {
  const pieces: Inline[] = [];
  let last = 0;
  for (const m of source.matchAll(INLINE)) {
    const at = m.index ?? 0;
    if (at > last) pieces.push({ text: source.slice(last, at) });
    if (m[2] !== undefined) pieces.push({ text: m[1], link: { scheme: m[2] as LinkScheme, id: m[3] } });
    else if (m[4] !== undefined) pieces.push({ text: m[4], code: true });
    else pieces.push({ text: m[5], bold: true });
    last = at + m[0].length;
  }
  if (last < source.length) pieces.push({ text: source.slice(last) });
  return pieces;
}

const cells = (line: string): string[] =>
  line
    .trim()
    .replace(/^\|/, '')
    .replace(/\|$/, '')
    .split('|')
    .map((c) => c.trim());

const isSeparator = (line: string) => /^\|?\s*:?-{2,}:?\s*(\|\s*:?-{2,}:?\s*)*\|?$/.test(line.trim());

/** Cuts a document into its blocks. */
export function parseMarkdown(source: string): Block[] {
  const lines = source.replace(/\r\n/g, '\n').split('\n');
  const blocks: Block[] = [];
  let i = 0;
  while (i < lines.length) {
    const line = lines[i];
    if (line.trim() === '') {
      i++;
      continue;
    }
    const heading = /^(#{1,3})\s+(.*)$/.exec(line);
    if (heading) {
      blocks.push({ type: 'heading', level: heading[1].length as 1 | 2 | 3, inline: parseInline(heading[2]) });
      i++;
    } else if (line.trimStart().startsWith('|')) {
      const rows: string[][] = [];
      while (i < lines.length && lines[i].trimStart().startsWith('|')) {
        if (!isSeparator(lines[i])) rows.push(cells(lines[i]));
        i++;
      }
      const [head = [], ...body] = rows;
      blocks.push({ type: 'table', head: head.map(parseInline), rows: body.map((r) => r.map(parseInline)) });
    } else if (/^[-*]\s+/.test(line)) {
      const items: Inline[][] = [];
      while (i < lines.length && /^[-*]\s+/.test(lines[i])) {
        items.push(parseInline(lines[i].replace(/^[-*]\s+/, '')));
        i++;
      }
      blocks.push({ type: 'list', items });
    } else if (line.startsWith('>')) {
      const text: string[] = [];
      while (i < lines.length && lines[i].startsWith('>')) {
        text.push(lines[i].replace(/^>\s?/, ''));
        i++;
      }
      blocks.push({ type: 'quote', inline: parseInline(text.join(' ')) });
    } else {
      const text: string[] = [];
      while (i < lines.length && lines[i].trim() !== '' && !/^(#{1,3}\s|[-*]\s|>|\|)/.test(lines[i].trimStart())) {
        text.push(lines[i].trim());
        i++;
      }
      blocks.push({ type: 'paragraph', inline: parseInline(text.join(' ')) });
    }
  }
  return blocks;
}
