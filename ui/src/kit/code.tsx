import { useState } from 'react';
import { createPortal } from 'react-dom';
import { CodeEditor, type CodeLanguage, type CodeName } from '../components/CodeEditor.tsx';
import { FloatingWindow } from '../components/FloatingWindow.tsx';
import { useT } from '../i18n/index.tsx';
import { Button } from './controls.tsx';

// The field of a text: a line of code editor (coloured, completed, brackets closed) with a button at its side that opens the same
// text in a window of its own, as a file of a code editor: several lines, line numbers, the same colours and completion.

export type CodeInputProps = {
  value: string;
  onChange: (text: string) => void;
  /** What the text is: it decides the colours and the completion. */
  language?: CodeLanguage;
  /** The names the text can use, with what they hold now (the inputs of the block). */
  names?: CodeName[];
  /** What the text is for: the title of its window. */
  label: string;
  disabled?: boolean;
  title?: string;
  placeholder?: string;
};

// the window that was touched last is the nearest one
let front = 100;

/** The window that edits a text as a file: moved by its title, resized by its corner, closed with its cross or Escape. */
function CodeWindow({ label, onClose, ...editor }: Omit<CodeInputProps, 'title'> & { onClose: () => void }) {
  const t = useT();
  const [at, setAt] = useState(() => ({ x: Math.max(8, window.innerWidth / 2 - 280), y: Math.max(8, window.innerHeight / 2 - 190) }));
  const [order, setOrder] = useState(() => ++front);
  const [caret, setCaret] = useState({ line: 1, column: 1 });
  const language = editor.language ?? 'plain';
  const inputs = (editor.names ?? []).map((n) => n.name);
  return (
    <FloatingWindow className="code-window" title={label} x={at.x} y={at.y} order={order} onMove={(x, y) => setAt({ x, y })} onFocus={() => setOrder(++front)} onClose={onClose}>
      <div className="code-window-editor">
        <CodeEditor {...editor} multiline autoFocus title={label} onCaret={(line, column) => setCaret({ line, column })} onEscape={onClose} />
      </div>
      <div className="code-window-status">
        <span>{t('code.position', { line: caret.line, column: caret.column })}</span>
        {inputs.length > 0 && <span className="code-window-inputs">{t('code.inputs', { names: inputs.join(', ') })}</span>}
        <span className="spacer" />
        <span>{t(`code.language.${language}`)}</span>
      </div>
    </FloatingWindow>
  );
}

/** A field for a text that is written like code, with a button to edit it in a window. `nodrag` so that a block is not moved by it. */
export function CodeInput({ value, onChange, language = 'plain', names, label, disabled, title, placeholder }: CodeInputProps) {
  const t = useT();
  const [open, setOpen] = useState(false);
  return (
    <span className="code-input nodrag">
      <CodeEditor value={value} onChange={onChange} language={language} names={names} disabled={disabled} title={title ?? label} placeholder={placeholder} />
      <Button variant="flat" size="small" icon="expand" className="code-expand" title={t('code.expand')} pressed={open} onClick={() => setOpen((v) => !v)} />
      {open && createPortal(<CodeWindow value={value} onChange={onChange} language={language} names={names} label={label} disabled={disabled} placeholder={placeholder} onClose={() => setOpen(false)} />, document.body)}
    </span>
  );
}
