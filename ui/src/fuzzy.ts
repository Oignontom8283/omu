/** Lower case, without accents: "Réglages" and "reglages" are the same text for a search. */
export function fold(text: string): string {
  return text.normalize('NFD').replace(/\p{Diacritic}/gu, '').toLowerCase();
}

export type Match = { score: number; /** the positions (in the original text) of the letters that matched */ indices: number[] };

const isBoundary = (text: string, i: number): boolean => i === 0 || /[\s\-_/.:()]/.test(text[i - 1]);

/**
 * Whether the letters of `query` appear in `text` in that order (not necessarily side by side), and how well: a letter at the
 * start of a word, or right after the previous one, counts more; a text that is the query, or starts with it, beats one that
 * only contains its letters, and a shorter text beats a longer one. Null when they do not appear. A query with no letter
 * matches everything with the same score.
 */
export function fuzzyMatch(query: string, text: string): Match | null {
  const q = fold(query).replace(/\s+/g, '');
  if (q === '') return { score: 0, indices: [] };
  // letter by letter, so that the positions stay those of the original text
  const letters = [...text].map(fold);
  const indices: number[] = [];
  let score = 0;
  let from = 0;
  let previous = -2;
  for (const letter of q) {
    const found = letters.indexOf(letter, from);
    if (found < 0) return null;
    score += 1;
    if (found === previous + 1) score += 3;
    if (isBoundary(text, found)) score += 4;
    if (found === 0) score += 2;
    indices.push(found);
    previous = found;
    from = found + 1;
  }
  const whole = letters.join('');
  if (whole === q) score += 20;
  else if (whole.startsWith(q)) score += 10;
  return { score: score - letters.length * 0.05, indices };
}

/** The items that match, the best first (ties keep the order given). */
export function rank<T>(query: string, items: T[], textOf: (item: T) => string): { item: T; match: Match }[] {
  const found: { item: T; match: Match; at: number }[] = [];
  items.forEach((item, at) => {
    const match = fuzzyMatch(query, textOf(item));
    if (match) found.push({ item, match, at });
  });
  return found.sort((a, b) => b.match.score - a.match.score || a.at - b.at).map(({ item, match }) => ({ item, match }));
}
