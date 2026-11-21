import { autocompletion, type CompletionContext, type CompletionResult } from '@codemirror/autocomplete';
import { Compartment, EditorState, RangeSetBuilder } from '@codemirror/state';
import { Decoration, EditorView, ViewPlugin, hoverTooltip, type DecorationSet } from '@codemirror/view';
import { useEffect, useRef } from 'react';
import { tokenizeTemplate } from '../template.ts';

// The field of a text template, written the way a code editor writes a line: the values coloured, a list of the values to complete
// `$` with, the value of a variable shown when the pointer rests on it. The editor itself is CodeMirror; what is ours is the
// grammar (template.ts) and what the block knows about its values.

export type TemplateValue = { name: string; /** What the value is now, written out. */ shown: string };

/** Colours the pieces of the template, from the same cutting as the daemon's reading (`template.ts`). */
function colours(known: () => (name: string) => boolean) {
  const build = (doc: string): DecorationSet => {
    const builder = new RangeSetBuilder<Decoration>();
    let at = 0;
    for (const token of tokenizeTemplate(doc, known())) {
      builder.add(at, at + token.text.length, Decoration.mark({ class: `tok-${token.kind}` }));
      at += token.text.length;
    }
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

const theme = EditorView.theme({
  '&': { height: '100%', background: 'transparent', color: 'var(--text)', fontSize: '12px' },
  '&.cm-focused': { outline: 'none' },
  '.cm-scroller': { fontFamily: 'var(--font-code)', lineHeight: '18px', overflow: 'hidden' },
  '.cm-content': { padding: '0 6px', caretColor: 'var(--text)' },
  '.cm-line': { padding: 0 },
  '.cm-tooltip': { background: 'var(--panel)', color: 'var(--text)', border: '1px solid var(--line)', borderRadius: '4px', fontFamily: 'var(--font-code)', fontSize: '12px' },
  '.cm-tooltip-autocomplete ul li[aria-selected]': { background: 'color-mix(in srgb, var(--primary) 82%, #000)', color: '#fff' },
  '.cm-completionDetail': { marginLeft: '12px', opacity: 0.7, fontStyle: 'normal' },
  '.cm-hover': { padding: '2px 8px' },
});

export function TemplateEditor({ value, values, disabled, title, onChange }: { value: string; values: TemplateValue[]; disabled?: boolean; title?: string; onChange: (text: string) => void }) {
  const host = useRef<HTMLDivElement>(null);
  const view = useRef<EditorView | null>(null);
  // what the editor needs to know that changes: read at the moment it is used
  const latest = useRef({ values, onChange });
  latest.current = { values, onChange };
  const colouring = useRef(new Compartment());

  useEffect(() => {
    if (!host.current) return;
    const known = () => {
      const names = new Set(latest.current.values.map((v) => v.name));
      return (name: string) => names.has(name) || names.has(`arg${name}`);
    };
    const complete = (ctx: CompletionContext): CompletionResult | null => {
      const before = ctx.matchBefore(/\$\{?\w*$/);
      if (!before || (before.from === before.to && !ctx.explicit)) return null;
      const braced = before.text.startsWith('${');
      return {
        from: before.from,
        options: latest.current.values.map((v) => ({ label: braced ? `\${${v.name}}` : `$${v.name}`, detail: v.shown, type: 'variable' })),
        validFor: /^\$\{?\w*\}?$/,
      };
    };
    const hover = hoverTooltip((v, pos) => {
      const doc = v.state.doc.toString();
      let at = 0;
      for (const token of tokenizeTemplate(doc, known())) {
        const end = at + token.text.length;
        if (token.kind === 'variable' && pos >= at && pos <= end) {
          const found = latest.current.values.find((x) => x.name === token.text || x.name === `arg${token.text}`);
          return {
            pos: at,
            end,
            above: true,
            create: () => {
              const dom = document.createElement('div');
              dom.className = 'cm-hover';
              dom.textContent = found ? `${found.name} = ${found.shown}` : token.text;
              return { dom };
            },
          };
        }
        at = end;
      }
      return null;
    });
    const state = EditorState.create({
      doc: value,
      extensions: [
        theme,
        colouring.current.of(colours(known)),
        autocompletion({ override: [complete], activateOnTyping: true, icons: false }),
        hover,
        // one line only
        EditorState.transactionFilter.of((tr) => (tr.newDoc.lines > 1 ? [] : tr)),
        EditorView.updateListener.of((u) => {
          if (u.docChanged) latest.current.onChange(u.state.doc.toString());
        }),
        EditorView.editable.of(!disabled),
      ],
    });
    view.current = new EditorView({ state, parent: host.current });
    return () => view.current?.destroy();
    // the editor is made once; what changes is told to it below
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [disabled]);

  // the text changed from outside (undo, another editor): the editor shows it
  useEffect(() => {
    const v = view.current;
    if (v && v.state.doc.toString() !== value) v.dispatch({ changes: { from: 0, to: v.state.doc.length, insert: value } });
  }, [value]);
  // the values of the block changed: the colours are worked out again (a name that did not exist may now)
  useEffect(() => {
    view.current?.dispatch({
      effects: colouring.current.reconfigure(
        colours(() => {
          const names = new Set(values.map((x) => x.name));
          return (name: string) => names.has(name) || names.has(`arg${name}`);
        }),
      ),
    });
  }, [values]);

  return <div ref={host} className={`template-editor nodrag${disabled ? ' disabled' : ''}`} title={title} />;
}
