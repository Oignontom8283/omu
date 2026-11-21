import { autocompletion, closeBrackets, closeBracketsKeymap, snippetCompletion, type Completion, type CompletionContext, type CompletionResult } from '@codemirror/autocomplete';
import { defaultKeymap, history, historyKeymap } from '@codemirror/commands';
import { bracketMatching } from '@codemirror/language';
import { Annotation, Compartment, EditorState, Prec, RangeSetBuilder, StateField, type Extension, type Text } from '@codemirror/state';
import { Decoration, EditorView, WidgetType, ViewPlugin, highlightActiveLine, highlightActiveLineGutter, hoverTooltip, keymap, lineNumbers, placeholder as placeholderExtension, tooltips, type DecorationSet } from '@codemirror/view';
import { useEffect, useRef } from 'react';
import { FORMULA_FUNCTIONS, tokenizeFormula } from '../formula.ts';
import { useT } from '../i18n/index.tsx';
import { tokenizeTemplate } from '../template.ts';

// A text written the way a code editor writes it: coloured by what it says, completed while it is typed, brackets closed and
// matched, undo and redo, the value of a name shown when the pointer rests on it. The editor itself is CodeMirror; what is ours
// is what each kind of text means (`template.ts`, `formula.ts`) and what the block knows about its names.
//
// It is drawn in one line, as a field of a block (a line break, when the text has one, is shown as a mark), or as a whole editor
// with line numbers (the window that every text field opens: see `kit/code.tsx`).

/** What the text is: a plain text, a template of text (`$arg1 is ${arg2:none}`), or a mathematical expression (`a + b`). */
export type CodeLanguage = 'plain' | 'template' | 'expression';

/** A name the text can use (an input of the block: `arg1`, `a`), with what it holds now, written out. */
export type CodeName = { name: string; shown: string };

export type CodeEditorProps = {
  value: string;
  onChange: (text: string) => void;
  language?: CodeLanguage;
  names?: CodeName[];
  disabled?: boolean;
  title?: string;
  placeholder?: string;
  /** The whole editor (line numbers, several lines) rather than the single line of a field. */
  multiline?: boolean;
  autoFocus?: boolean;
  /** Called with the line and the column of the caret (counted from 1). */
  onCaret?: (line: number, column: number) => void;
  /** Escape was pressed and nothing else used it (no list of completions was open). */
  onEscape?: () => void;
};

/** A change that comes from the value the field is given, not from typing: it is not told back to the block. */
const External = Annotation.define<boolean>();

type Piece = { from: number; to: number; className: string; tip?: () => string };

class LineBreak extends WidgetType {
  constructor(private readonly label: string) {
    super();
  }
  eq(other: LineBreak) {
    return other.label === this.label;
  }
  toDOM() {
    const mark = document.createElement('span');
    mark.className = 'cm-break';
    mark.textContent = '↵';
    mark.title = this.label;
    return mark;
  }
}

/** A line break of a text in a one-line field is a mark, so that the text stays on one line. */
function lineBreaks(label: () => string) {
  const mark = (doc: Text) => {
    const builder = new RangeSetBuilder<Decoration>();
    for (let n = 1; n < doc.lines; n++) {
      const end = doc.line(n).to;
      builder.add(end, end + 1, Decoration.replace({ widget: new LineBreak(label()) }));
    }
    return builder.finish();
  };
  return StateField.define<DecorationSet>({
    create: (state) => mark(state.doc),
    update: (value, tr) => (tr.docChanged ? mark(tr.newDoc) : value),
    provide: (field) => [EditorView.decorations.from(field), EditorView.atomicRanges.of((view) => view.state.field(field))],
  });
}

/** The pieces of the text that have a colour, and what to say about them when the pointer rests on one. */
function piecesOf(language: CodeLanguage, doc: string, names: readonly CodeName[], say: (key: string, params?: Record<string, string>) => string): Piece[] {
  const shownOf = (word: string) => names.find((n) => n.name === word || n.name === `arg${word}`);
  const pieces: Piece[] = [];
  let at = 0;
  if (language === 'template') {
    const known = new Set(names.map((n) => n.name));
    for (const token of tokenizeTemplate(doc, (name) => known.has(name) || known.has(`arg${name}`))) {
      const end = at + token.text.length;
      const found = token.kind === 'variable' ? shownOf(token.text) : undefined;
      const tip = found ? () => `${found.name} = ${found.shown}` : token.kind === 'error' && /^\w+$/.test(token.text) ? () => say('code.unknownText', { name: token.text }) : undefined;
      pieces.push({ from: at, to: end, className: `tok-${token.kind}`, tip });
      at = end;
    }
  } else if (language === 'expression') {
    for (const token of tokenizeFormula(doc, names.map((n) => n.name))) {
      const end = at + token.text.length;
      if (token.kind !== 'space') {
        const found = token.kind === 'variable' ? shownOf(token.text) : undefined;
        const fn = token.kind === 'function' ? FORMULA_FUNCTIONS.find((f) => f.name === token.text) : undefined;
        const tip = found ? () => `${found.name} = ${found.shown}` : fn ? () => `${fn.signature}: ${fn.info}` : token.kind === 'unknown' && /^\w+$/.test(token.text) ? () => say('code.unknownName', { name: token.text }) : undefined;
        pieces.push({ from: at, to: end, className: `tok-${token.kind === 'unknown' ? 'error' : token.kind}`, tip });
      }
      at = end;
    }
  }
  return pieces;
}

/** Colours the text and says what a name is, from the same cutting as the daemon's reading. */
function colours(language: CodeLanguage, read: () => { names: readonly CodeName[]; say: (key: string, params?: Record<string, string>) => string }) {
  const build = (doc: string): DecorationSet => {
    const { names, say } = read();
    const builder = new RangeSetBuilder<Decoration>();
    for (const piece of piecesOf(language, doc, names, say)) builder.add(piece.from, piece.to, Decoration.mark({ class: piece.className }));
    return builder.finish();
  };
  return ViewPlugin.fromClass(
    class {
      decorations: DecorationSet;
      constructor(view: EditorView) {
        this.decorations = build(view.state.doc.toString());
      }
      update(u: { docChanged: boolean; view: EditorView; transactions: readonly { reconfigured: boolean }[] }) {
        if (u.docChanged || u.transactions.some((t) => t.reconfigured)) this.decorations = build(u.view.state.doc.toString());
      }
    },
    { decorations: (v) => v.decorations },
  );
}

/** The list of what can be written at the caret: the names of the block, and for an expression its functions. */
function completer(language: CodeLanguage, read: () => { names: readonly CodeName[] }) {
  return (ctx: CompletionContext): CompletionResult | null => {
    const { names } = read();
    if (language === 'template') {
      const before = ctx.matchBefore(/\$\{?\w*$/);
      if (!before || (before.from === before.to && !ctx.explicit)) return null;
      const braced = before.text.startsWith('${');
      return {
        from: before.from,
        options: names.map((v) => ({ label: braced ? `\${${v.name}}` : `$${v.name}`, detail: v.shown, type: 'variable' })),
        validFor: /^\$\{?\w*\}?$/,
      };
    }
    const word = ctx.matchBefore(/[A-Za-z_]\w*$/);
    if (!word && !ctx.explicit) return null;
    const options: Completion[] = [
      ...names.map((v): Completion => ({ label: v.name, detail: v.shown, type: 'variable', boost: 2 })),
      ...FORMULA_FUNCTIONS.map((f) => snippetCompletion(f.name === 'e' || f.name === 'pi' ? `${f.name}()` : `${f.name}(\${x})`, { label: f.name, detail: f.signature, info: f.info, type: 'function' })),
      { label: 'and', type: 'keyword', detail: '&&' },
      { label: 'or', type: 'keyword', detail: '||' },
    ];
    return { from: word ? word.from : ctx.pos, options, validFor: /^\w*$/ };
  };
}

const theme = EditorView.theme({
  '&': { height: '100%', background: 'transparent', color: 'var(--text)', fontSize: '12px' },
  '&.cm-focused': { outline: 'none' },
  '.cm-scroller': { fontFamily: 'var(--font-code)', lineHeight: '18px' },
  '.cm-content': { padding: '0 6px', caretColor: 'var(--text)' },
  '.cm-line': { padding: 0 },
  '.cm-gutters': { background: 'transparent', color: 'var(--muted)', border: 'none', borderRight: '1px solid var(--line)' },
  '.cm-activeLine': { background: 'color-mix(in srgb, var(--text) 6%, transparent)' },
  '.cm-activeLineGutter': { background: 'color-mix(in srgb, var(--text) 8%, transparent)', color: 'var(--text)' },
  '.cm-placeholder': { color: 'var(--muted)' },
  '&.cm-focused .cm-matchingBracket': { background: 'color-mix(in srgb, var(--primary) 30%, transparent)', outline: '1px solid var(--primary)' },
  '&.cm-focused .cm-nonmatchingBracket': { background: 'color-mix(in srgb, var(--bad) 30%, transparent)' },
});

export function CodeEditor({ value, onChange, language = 'plain', names = [], disabled, title, placeholder, multiline, autoFocus, onCaret, onEscape }: CodeEditorProps) {
  const t = useT();
  const host = useRef<HTMLDivElement>(null);
  const view = useRef<EditorView | null>(null);
  // what the editor needs to know, and that changes: read at the moment it is used
  const latest = useRef({ names, say: t as (key: string, params?: Record<string, string>) => string, onChange, onCaret, onEscape });
  latest.current = { names, say: t as (key: string, params?: Record<string, string>) => string, onChange, onCaret, onEscape };
  const colouring = useRef(new Compartment());
  // a new list with the same names does not change anything
  const namesKey = names.map((n) => `${n.name}=${n.shown}`).join('\n');

  useEffect(() => {
    if (!host.current) return;
    const read = () => latest.current;
    const code = language !== 'plain';
    const hover = hoverTooltip((v, pos) => {
      const piece = piecesOf(language, v.state.doc.toString(), read().names, read().say).find((p) => p.tip && pos >= p.from && pos <= p.to);
      if (!piece?.tip) return null;
      const text = piece.tip();
      return {
        pos: piece.from,
        end: piece.to,
        above: true,
        create: () => {
          const dom = document.createElement('div');
          dom.className = 'cm-hover';
          dom.textContent = text;
          return { dom };
        },
      };
    });
    const extensions: Extension[] = [
      theme,
      // the lists and hovers are drawn over everything, not cut by the block or the window that holds the editor
      tooltips({ parent: document.body }),
      history(),
      keymap.of([...(code ? closeBracketsKeymap : []), ...defaultKeymap, ...historyKeymap]),
      EditorView.updateListener.of((u) => {
        if (u.docChanged && !u.transactions.some((tr) => tr.annotation(External))) read().onChange(u.state.doc.toString());
        if (u.selectionSet || u.docChanged) {
          const head = u.state.selection.main.head;
          const line = u.state.doc.lineAt(head);
          read().onCaret?.(line.number, head - line.from + 1);
        }
      }),
      EditorView.editable.of(!disabled),
      EditorState.readOnly.of(Boolean(disabled)),
      EditorView.contentAttributes.of({ 'aria-label': title ?? '', spellcheck: 'false', autocorrect: 'off', autocapitalize: 'off' }),
      Prec.low(keymap.of([{ key: 'Escape', run: () => (read().onEscape ? (read().onEscape?.(), true) : false) }])),
    ];
    if (placeholder) extensions.push(placeholderExtension(placeholder));
    if (code) {
      extensions.push(
        // only the brackets that mean something in this text are closed (an apostrophe or a quote in a text is only text)
        EditorState.languageData.of(() => [{ closeBrackets: { brackets: language === 'expression' ? ['('] : ['{', '('] } }]),
        colouring.current.of(colours(language, read)),
        hover,
        bracketMatching(),
        closeBrackets(), autocompletion({ override: [completer(language, read)], activateOnTyping: true, icons: false }));
    }
    if (multiline) extensions.push(lineNumbers(), highlightActiveLine(), highlightActiveLineGutter(), EditorView.lineWrapping);
    else
      extensions.push(
        lineBreaks(() => read().say('code.lineBreak')),
        // one line: Enter and a pasted line break are not typed in (a text that has several lines is changed in the window)
        EditorState.transactionFilter.of((tr) => (tr.docChanged && !tr.annotation(External) && tr.newDoc.lines > tr.startState.doc.lines ? [] : tr)),
      );
    const state = EditorState.create({ doc: value, extensions });
    const editor = new EditorView({ state, parent: host.current });
    view.current = editor;
    if (autoFocus) editor.focus();
    return () => editor.destroy();
    // the editor is made once for a language and a mode; what changes after is told to it below
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [language, multiline, disabled]);

  // the text changed from outside (undo, another editor of the same text): the editor shows it
  useEffect(() => {
    const v = view.current;
    if (v && v.state.doc.toString() !== value) v.dispatch({ changes: { from: 0, to: v.state.doc.length, insert: value }, annotations: External.of(true) });
  }, [value]);
  // the names of the block changed: the colours are worked out again (a name that did not exist may now)
  useEffect(() => {
    if (language === 'plain') return;
    view.current?.dispatch({ effects: colouring.current.reconfigure(colours(language, () => latest.current)) });
  }, [namesKey, language]);

  return <div ref={host} className={`code-editor nodrag${multiline ? ' multiline' : ''}${disabled ? ' disabled' : ''}`} title={title} />;
}
