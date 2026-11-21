// What a mathematical expression looks like, for the colours and the completion of its editor. The meaning of an expression is the
// daemon's (it reads it with a library, and refuses one it cannot read: see `src/types/expression.rs`); this only cuts the text into
// the pieces that are shown in different colours, and lists what can be written.

export type FormulaToken = {
  text: string;
  kind: 'space' | 'number' | 'variable' | 'function' | 'keyword' | 'operator' | 'paren' | 'unknown';
};

/** The functions an expression can call (the same as the daemon's `FUNCTIONS`): how they are written, and what they give. */
export const FORMULA_FUNCTIONS: { name: string; signature: string; info: string }[] = [
  { name: 'abs', signature: 'abs(x)', info: 'the number without its sign' },
  { name: 'ceil', signature: 'ceil(x)', info: 'rounded up' },
  { name: 'floor', signature: 'floor(x)', info: 'rounded down' },
  { name: 'int', signature: 'int(x)', info: 'without its decimal part' },
  { name: 'round', signature: 'round(x)', info: 'rounded to the nearest whole number' },
  { name: 'sign', signature: 'sign(x)', info: '-1, 0 or 1' },
  { name: 'min', signature: 'min(x, y)', info: 'the smaller (more numbers can be given)' },
  { name: 'max', signature: 'max(x, y)', info: 'the greater (more numbers can be given)' },
  { name: 'log', signature: 'log(x)', info: 'logarithm in base 10; log(base, x) for another base' },
  { name: 'e', signature: 'e()', info: 'the number e' },
  { name: 'pi', signature: 'pi()', info: 'the number pi' },
  { name: 'sin', signature: 'sin(x)', info: 'sine (in radians)' },
  { name: 'cos', signature: 'cos(x)', info: 'cosine (in radians)' },
  { name: 'tan', signature: 'tan(x)', info: 'tangent (in radians)' },
  { name: 'asin', signature: 'asin(x)', info: 'arc sine' },
  { name: 'acos', signature: 'acos(x)', info: 'arc cosine' },
  { name: 'atan', signature: 'atan(x)', info: 'arc tangent' },
];

const FUNCTION_NAMES = new Set(FORMULA_FUNCTIONS.map((f) => f.name));
const KEYWORDS = new Set(['and', 'or']);
const PIECE = /(\s+)|(\d+\.?\d*(?:[eE][+-]?\d+)?|\.\d+(?:[eE][+-]?\d+)?)|([A-Za-z_]\w*)|(==|!=|<=|>=|&&|\|\||[-+*/%^<>=!,])|([()])|(.)/gy;

/**
 * Cuts `source` into colourable pieces. `inputs` are the names of the block's inputs (`a`, `b`...): a word that is neither one of them,
 * nor a function followed by its parenthesis, nor `and` / `or`, is `unknown`. The pieces put end to end give `source` back.
 */
export function tokenizeFormula(source: string, inputs: readonly string[]): FormulaToken[] {
  const known = new Set(inputs);
  const tokens: FormulaToken[] = [];
  PIECE.lastIndex = 0;
  for (let m = PIECE.exec(source); m; m = PIECE.exec(source)) {
    const [text, space, number, word, operator, paren] = m;
    if (space) tokens.push({ text, kind: 'space' });
    else if (number) tokens.push({ text, kind: 'number' });
    else if (word) {
      const call = /^\s*\(/.test(source.slice(PIECE.lastIndex));
      const kind = KEYWORDS.has(word) ? 'keyword' : call && FUNCTION_NAMES.has(word) ? 'function' : known.has(word) ? 'variable' : 'unknown';
      tokens.push({ text, kind });
    } else if (operator) tokens.push({ text, kind: 'operator' });
    else if (paren) tokens.push({ text, kind: 'paren' });
    else tokens.push({ text, kind: 'unknown' });
  }
  return tokens;
}
