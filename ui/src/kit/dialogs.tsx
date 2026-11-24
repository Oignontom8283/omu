import { useState, useSyncExternalStore } from 'react';
import { FloatingWindow } from '../components/FloatingWindow.tsx';
import { useT } from '../i18n/index.tsx';
import { Button } from './controls.tsx';

// Questions the program asks before it does something that cannot be taken back (stopping, removing). A question is a floating window
// like any other, except that it greys the page and keeps it from being used until it is answered. Any part of the interface asks
// with `ask`; the window itself is shown by the one `DialogHost` of the page.

export type Question = {
  title: string;
  message: string;
  /** The words of the two buttons (yes, in red, and no, in green); the usual ones when missing. */
  yes?: string;
  no?: string;
};

type Pending = Question & { id: number; resolve: (yes: boolean) => void };

let pending: Pending[] = [];
let next = 1;
const listeners = new Set<() => void>();
const publish = () => listeners.forEach((l) => l());
const subscribe = (l: () => void) => {
  listeners.add(l);
  return () => listeners.delete(l);
};

/**
 * Asks a question and gives the answer: `true` for yes. With the Shift key held down on the click that asks (pass the event), the
 * answer is yes without asking: whoever knows what they do goes faster.
 */
export function ask(question: Question, event?: { shiftKey?: boolean }): Promise<boolean> {
  if (event?.shiftKey) return Promise.resolve(true);
  return new Promise((resolve) => {
    pending = [...pending, { ...question, id: next++, resolve }];
    publish();
  });
}

const answer = (question: Pending, yes: boolean) => {
  pending = pending.filter((q) => q.id !== question.id);
  publish();
  question.resolve(yes);
};

/** Shows the question that is waiting for an answer (the first asked first). Put it once in the page. */
export function DialogHost() {
  const t = useT();
  const questions = useSyncExternalStore(subscribe, () => pending);
  const [moved, setMoved] = useState<Record<number, { x: number; y: number }>>({});
  const q = questions[0];
  if (!q) return null;
  const at = moved[q.id] ?? { x: Math.max(4, (window.innerWidth - 420) / 2), y: Math.max(4, window.innerHeight / 2 - 140) };
  return (
    <FloatingWindow className="ask-window cat-condition" modal title={q.title} x={at.x} y={at.y} order={0} onMove={(x, y) => setMoved((m) => ({ ...m, [q.id]: { x, y } }))} onFocus={() => undefined} onClose={() => answer(q, false)}>
      <p className="ask-message">{q.message}</p>
      <div className="ask-buttons">
        <Button variant="danger" onClick={() => answer(q, true)}>
          {q.yes ?? t('dialog.yes')}
        </Button>
        <Button variant="success" autoFocus onClick={() => answer(q, false)}>
          {q.no ?? t('dialog.no')}
        </Button>
      </div>
    </FloatingWindow>
  );
}
