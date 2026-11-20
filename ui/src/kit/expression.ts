// A number typed as an expression: `2*3`, `-6 + 1.5`, `(10 - 2) / 4`. A small parser of its own for arithmetic only: nothing typed in a
// field is ever run as code (no `eval`). Returns null when the text is not an expression that gives a number.

type Cursor = { text: string; at: number };

const skip = (c: Cursor) => {
  while (c.text[c.at] === ' ') c.at += 1;
};

function number(c: Cursor): number | null {
  skip(c);
  const match = /^(\d+\.?\d*|\.\d+)(e[+-]?\d+)?/i.exec(c.text.slice(c.at));
  if (!match) return null;
  c.at += match[0].length;
  return Number(match[0]);
}

function unary(c: Cursor): number | null {
  skip(c);
  const sign = c.text[c.at];
  if (sign === '-' || sign === '+') {
    c.at += 1;
    const v = unary(c);
    return v === null ? null : sign === '-' ? -v : v;
  }
  if (sign === '(') {
    c.at += 1;
    const v = sum(c);
    skip(c);
    if (v === null || c.text[c.at] !== ')') return null;
    c.at += 1;
    return v;
  }
  return number(c);
}

function product(c: Cursor): number | null {
  let v = unary(c);
  for (;;) {
    skip(c);
    const op = c.text[c.at];
    if (v === null || (op !== '*' && op !== '/' && op !== '×' && op !== '÷')) return v;
    c.at += 1;
    const right = unary(c);
    if (right === null) return null;
    v = op === '*' || op === '×' ? v * right : v / right;
  }
}

function sum(c: Cursor): number | null {
  let v = product(c);
  for (;;) {
    skip(c);
    const op = c.text[c.at];
    if (v === null || (op !== '+' && op !== '-')) return v;
    c.at += 1;
    const right = product(c);
    if (right === null) return null;
    v = op === '+' ? v + right : v - right;
  }
}

/** The number an expression gives (a comma counts as a decimal point), or null. */
export function evaluate(text: string): number | null {
  const c: Cursor = { text: text.replace(/,/g, '.').trim(), at: 0 };
  if (c.text === '') return null;
  const v = sum(c);
  skip(c);
  return v !== null && c.at === c.text.length && Number.isFinite(v) ? v : null;
}

/** A number written for a field: no useless decimals. */
export function written(value: number, whole: boolean): string {
  if (whole || Number.isInteger(value)) return String(Math.round(value));
  return String(Number(value.toFixed(4)));
}
