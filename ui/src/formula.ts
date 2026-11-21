// What a mathematical expression, or a test on texts, looks like: for the colours and the completion of its editor. The meaning is the
// daemon's (it reads them with libraries, and refuses what it cannot read: see `src/types/expression.rs` and `text_expression.rs`); this
// only cuts the text into the pieces that are shown in different colours, and lists what can be written. What each function does is
// said in the interface's own words (`formula.fn.<name>`, `test.fn.<name>`).

export type FormulaToken = {
  text: string;
  kind: 'space' | 'number' | 'string' | 'variable' | 'function' | 'keyword' | 'operator' | 'paren' | 'unknown';
};

export type FormulaFunction = {
  name: string;
  /** How it is written, as the completion and the hover show it. */
  signature: string;
  /** What is inserted when it is chosen (`${x}` marks a place to fill in: see the completion's snippets). */
  snippet: string;
};

/** The functions of an expression (the same as the daemon's `FUNCTIONS`). */
export const FORMULA_FUNCTIONS: FormulaFunction[] = [
  ['abs', 'abs(x)'],
  ['ceil', 'ceil(x)'],
  ['floor', 'floor(x)'],
  ['int', 'int(x)'],
  ['round', 'round(x)'],
  ['sign', 'sign(x)'],
  ['min', 'min(x, y)'],
  ['max', 'max(x, y)'],
  ['log', 'log(x)'],
  ['e', 'e()'],
  ['pi', 'pi()'],
  ['sin', 'sin(x)'],
  ['cos', 'cos(x)'],
  ['tan', 'tan(x)'],
  ['asin', 'asin(x)'],
  ['acos', 'acos(x)'],
  ['atan', 'atan(x)'],
].map(([name, signature]) => ({ name, signature, snippet: signature.endsWith('()') ? signature : `${name}(\${x})` }));

/** The functions of a text that a test can call after a dot (`a.contains(b)`): what the `cel` library brings to texts. */
export const TEXT_METHODS: FormulaFunction[] = [
  ['contains', 'a.contains(b)', 'contains(${b})'],
  ['startsWith', 'a.startsWith(b)', 'startsWith(${b})'],
  ['endsWith', 'a.endsWith(b)', 'endsWith(${b})'],
  ['matches', 'a.matches("regex")', 'matches("${regex}")'],
  ['lowerAscii', 'a.lowerAscii()', 'lowerAscii()'],
  ['upperAscii', 'a.upperAscii()', 'upperAscii()'],
  ['trim', 'a.trim()', 'trim()'],
  ['indexOf', 'a.indexOf(b)', 'indexOf(${b})'],
  ['substring', 'a.substring(start, end)', 'substring(${start}, ${end})'],
  ['replace', 'a.replace(b, c)', 'replace(${b}, ${c})'],
  ['split', 'a.split(b)', 'split(${b})'],
].map(([name, signature, snippet]) => ({ name, signature, snippet }));

/** The one function of a test that is not written after a dot. */
export const TEXT_SIZE: FormulaFunction = { name: 'size', signature: 'size(a)', snippet: 'size(${a})' };

const FUNCTION_NAMES = new Set(FORMULA_FUNCTIONS.map((f) => f.name));
const METHOD_NAMES = new Set(TEXT_METHODS.map((f) => f.name));
const KEYWORDS = new Set(['and', 'or']);
const TEST_KEYWORDS = new Set(['and', 'or', 'true', 'false']);
const PIECE = /(\s+)|(\d+\.?\d*(?:[eE][+-]?\d+)?|\.\d+(?:[eE][+-]?\d+)?)|([A-Za-z_]\w*)|(==|!=|<=|>=|&&|\|\||[-+*/%^<>=!,])|([()])|(.)/gy;
const TEST_PIECE = /(\s+)|("(?:[^"\\]|\\.)*"?|'(?:[^'\\]|\\.)*'?)|(\d+\.?\d*(?:[eE][+-]?\d+)?)|([A-Za-z_]\w*)|(==|!=|<=|>=|&&|\|\||[-+*/%<>=!?:,.])|([()[\]{}])|(.)/gy;

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

/**
 * The same for a test on texts: the texts between quotes are `string`, a word after a dot is a function of a text (`unknown` when it is
 * not one), `size` is the one function that stands alone.
 */
export function tokenizeTest(source: string, inputs: readonly string[]): FormulaToken[] {
  const known = new Set(inputs);
  const tokens: FormulaToken[] = [];
  let afterDot = false;
  TEST_PIECE.lastIndex = 0;
  for (let m = TEST_PIECE.exec(source); m; m = TEST_PIECE.exec(source)) {
    const [text, space, string, number, word, operator, paren] = m;
    if (space) {
      tokens.push({ text, kind: 'space' });
      continue;
    }
    let kind: FormulaToken['kind'] = 'unknown';
    if (string) kind = 'string';
    else if (number) kind = 'number';
    else if (word) {
      const call = /^\s*\(/.test(source.slice(TEST_PIECE.lastIndex));
      if (afterDot) kind = call && METHOD_NAMES.has(word) ? 'function' : 'unknown';
      else if (TEST_KEYWORDS.has(word)) kind = 'keyword';
      else if (word === TEXT_SIZE.name && call) kind = 'function';
      else if (known.has(word)) kind = 'variable';
    } else if (operator) kind = 'operator';
    else if (paren) kind = 'paren';
    afterDot = operator === '.';
    tokens.push({ text, kind });
  }
  return tokens;
}
