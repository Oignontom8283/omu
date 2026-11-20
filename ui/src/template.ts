// What a text template looks like, for the colours of its editor. The meaning of a template is the daemon's (it uses a
// shell-like substitution library, and refuses a template it cannot read); this only cuts the text into the pieces that are
// shown in different colours, following the same grammar: `$name`, `${name}`, `${name:default}`, and a backslash before one
// of `\ $ { } :` to write it as it is.

export type TemplateToken = {
  text: string;
  kind: 'text' | 'escape' | 'dollar' | 'variable' | 'default' | 'error';
};

const isName = (c: string | undefined): boolean => c !== undefined && /[A-Za-z0-9_]/.test(c);

/**
 * Cuts `source` into colourable pieces. `known` says whether a name stands for a value that exists: one that does not is an
 * `error` (unless the template gives it a default). Anything the daemon would refuse is an `error` too.
 */
export function tokenizeTemplate(source: string, known: (name: string) => boolean): TemplateToken[] {
  const tokens: TemplateToken[] = [];
  const push = (text: string, kind: TemplateToken['kind']) => {
    if (text === '') return;
    const last = tokens[tokens.length - 1];
    if (last && last.kind === kind && (kind === 'text' || kind === 'default')) last.text += text;
    else tokens.push({ text, kind });
  };
  let i = 0;
  while (i < source.length) {
    const c = source[i];
    if (c === '\\') {
      const next = source[i + 1];
      if (next !== undefined && '\\${}:'.includes(next)) {
        push(c + next, 'escape');
        i += 2;
      } else {
        push(next === undefined ? c : c + next, 'error');
        i += next === undefined ? 1 : 2;
      }
    } else if (c !== '$') {
      push(c, 'text');
      i += 1;
    } else if (source[i + 1] === '{') {
      let j = i + 2;
      while (isName(source[j])) j += 1;
      const name = source.slice(i + 2, j);
      if (name === '') {
        push(source.slice(i, j), 'error');
        i = j;
      } else if (source[j] === '}') {
        push('${', 'dollar');
        push(name, known(name) ? 'variable' : 'error');
        push('}', 'dollar');
        i = j + 1;
      } else if (source[j] === ':') {
        // a default: everything up to the closing brace that is not escaped
        let k = j + 1;
        while (k < source.length && source[k] !== '}') k += source[k] === '\\' ? 2 : 1;
        if (k >= source.length) {
          push(source.slice(i), 'error');
          i = source.length;
        } else {
          push('${', 'dollar');
          push(name, 'variable');
          push(':', 'dollar');
          push(source.slice(j + 1, k), 'default');
          push('}', 'dollar');
          i = k + 1;
        }
      } else {
        push(source.slice(i), 'error');
        i = source.length;
      }
    } else {
      let j = i + 1;
      while (isName(source[j])) j += 1;
      const name = source.slice(i + 1, j);
      if (name === '') {
        push('$', 'error');
      } else {
        push('$', 'dollar');
        push(name, known(name) ? 'variable' : 'error');
      }
      i = j;
    }
  }
  return tokens;
}
