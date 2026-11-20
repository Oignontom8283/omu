/**
 * The steps of an editing session, to go back and forth in (undo and redo). It holds snapshots: each one replaces the
 * last when it is the same (the `key` says what "the same" is), and anything done after going back ends the way forward.
 */
export class History<T> {
  private past: T[] = [];
  private future: T[] = [];
  private present: T | null = null;
  private readonly key: (snapshot: T) => string;
  private readonly capacity: number;

  constructor(key: (snapshot: T) => string, capacity = 100) {
    this.key = key;
    this.capacity = capacity;
  }

  /** Notes the state the editor is in now. Returns whether it was a new step. */
  record(snapshot: T): boolean {
    if (this.present !== null && this.key(snapshot) === this.key(this.present)) return false;
    if (this.present !== null) {
      this.past.push(this.present);
      if (this.past.length > this.capacity) this.past.shift();
    }
    this.present = snapshot;
    this.future = [];
    return true;
  }

  /** The state before this one, which becomes the present; null when there is none. */
  undo(): T | null {
    const before = this.past.pop();
    if (before === undefined) return null;
    if (this.present !== null) this.future.push(this.present);
    this.present = before;
    return before;
  }

  redo(): T | null {
    const after = this.future.pop();
    if (after === undefined) return null;
    if (this.present !== null) this.past.push(this.present);
    this.present = after;
    return after;
  }

  get canUndo(): boolean {
    return this.past.length > 0;
  }

  get canRedo(): boolean {
    return this.future.length > 0;
  }

  /** Forgets everything: what is shown now is the start. */
  reset(snapshot: T | null = null): void {
    this.past = [];
    this.future = [];
    this.present = snapshot;
  }
}
