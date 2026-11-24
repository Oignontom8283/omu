import { useCallback, useEffect, useRef, useState, type ReactNode } from 'react';
import { IN_DESKTOP_WINDOW, RpcError, pickSoundFiles, uploadSound, type api } from './api.ts';
import { MODULE_NEEDED } from './modules.ts';
import { newId } from './graph.ts';
import { importFraction } from './soundboard.ts';
import type { ImportJob, SoundInfo } from './types.ts';

type Daemon = ReturnType<typeof api>;

/** A sound on its way in. `owner` is who asked (a soundboard, a block, the list of sounds): it shows the progress where it was asked. */
export type Importing = { job: string; owner: string; label: string; error?: string };

/** One file to import: its name, and how it is sent to the daemon (`job` is the number the progress is followed by). */
export type ImportJobSpec = { label: string; run: (job: string) => Promise<SoundInfo> };

/** What follows the imports of the whole program: any part of the interface may start one and show how far it is. */
export type SoundImports = {
  entries: Importing[];
  progress: Record<string, ImportJob>;
  /** Imports the files one after the other; `onDone` is called with each sound once it is stored. */
  start: (owner: string, jobs: ImportJobSpec[], onDone: (sound: SoundInfo) => void | Promise<void>) => Promise<void>;
  fromFiles: (files: File[]) => ImportJobSpec[];
  fromPaths: (paths: string[]) => ImportJobSpec[];
  /** Opens the file chooser of the system and imports what is chosen. */
  choose: (owner: string, onDone: (sound: SoundInfo) => void | Promise<void>, multiple?: boolean) => void;
  dismiss: (job: string) => void;
  /** The import that is running, or has failed, for this owner (the first one). */
  of: (owner: string) => { importing: Importing; fraction: number | null } | undefined;
  /** The chooser the browser needs (a hidden field): put it once in the page. */
  chooser: ReactNode;
};

const message = (e: unknown): string => (e instanceof Error ? e.message : String(e));

const fractionOf = (job: ImportJob | undefined): number | null => (job ? importFraction(job.done, job.total) : null);

/**
 * The imports of the sounds, shared by the whole program: the soundboards, the file blocks and the list of sounds all start theirs
 * here, so one place asks the daemon how far they are, and one place answers a file that needs an optional module.
 */
export function useSoundImports({ daemon, token, notify, onModuleNeeded }: { daemon: Daemon; token: string; notify: (text: string) => void; onModuleNeeded: (module: string) => void }): SoundImports {
  const [entries, setEntries] = useState<Importing[]>([]);
  const [progress, setProgress] = useState<Record<string, ImportJob>>({});
  const input = useRef<HTMLInputElement>(null);
  const asked = useRef<{ owner: string; onDone: (sound: SoundInfo) => void | Promise<void> } | null>(null);
  const moduleNeeded = useRef(onModuleNeeded);
  moduleNeeded.current = onModuleNeeded;

  // how far the imports are, asked of the daemon while there are some
  const running = entries.some((e) => !e.error);
  useEffect(() => {
    if (!running) {
      setProgress({});
      return;
    }
    const read = () =>
      daemon
        .imports()
        .then((jobs) => setProgress(Object.fromEntries(jobs.map((j) => [j.job, j]))))
        .catch(() => undefined);
    void read();
    const timer = setInterval(read, 300);
    return () => clearInterval(timer);
  }, [daemon, running]);

  const start = useCallback<SoundImports['start']>(async (owner, jobs, onDone) => {
    if (jobs.length === 0) return;
    const queued = jobs.map((j) => ({ ...j, id: newId() }));
    setEntries((l) => [...l, ...queued.map((j): Importing => ({ job: j.id, owner, label: j.label }))]);
    for (const job of queued) {
      try {
        const sound = await job.run(job.id);
        await onDone(sound);
        setEntries((l) => l.filter((e) => e.job !== job.id));
      } catch (e) {
        if (e instanceof RpcError && e.code === MODULE_NEEDED) {
          moduleNeeded.current(typeof e.data === 'string' ? e.data : 'ffmpeg');
          setEntries((l) => l.filter((x) => x.job !== job.id));
        } else {
          setEntries((l) => l.map((x) => (x.job === job.id ? { ...x, error: message(e) } : x)));
        }
      }
    }
  }, []);

  const fromFiles = useCallback((files: File[]): ImportJobSpec[] => files.map((f) => ({ label: f.name, run: (job) => uploadSound(token, f, job) })), [token]);
  const fromPaths = useCallback(
    (paths: string[]): ImportJobSpec[] =>
      paths
        .map((p) => p.trim())
        .filter(Boolean)
        .map((p) => ({ label: p.split(/[\\/]/).pop() ?? p, run: (job) => daemon.importSound(p, job) })),
    [daemon],
  );

  const choose = useCallback<SoundImports['choose']>(
    (owner, onDone, multiple = true) => {
      if (IN_DESKTOP_WINDOW) {
        // the system's dialog: it gives paths (the daemon reads the files where they are)
        void pickSoundFiles()
          .then((paths) => start(owner, fromPaths(multiple ? paths : paths.slice(0, 1)), onDone))
          .catch((e) => notify(message(e)));
        return;
      }
      asked.current = { owner, onDone };
      if (input.current) {
        input.current.multiple = multiple;
        input.current.click();
      }
    },
    [start, fromPaths, notify],
  );

  const chooser = (
    <input
      ref={input}
      type="file"
      hidden
      multiple
      accept=".wav,.mp3,.flac,.ogg,.oga,.aac,.m4a,.mp4,.m4v,.mov,.mkv,.webm,audio/*,video/*"
      onChange={(e) => {
        const files = Array.from(e.target.files ?? []);
        e.target.value = '';
        const target = asked.current;
        asked.current = null;
        if (target) void start(target.owner, fromFiles(files), target.onDone);
      }}
    />
  );

  const dismiss = useCallback((job: string) => setEntries((l) => l.filter((e) => e.job !== job)), []);
  const of = useCallback<SoundImports['of']>(
    (owner) => {
      const importing = entries.find((e) => e.owner === owner);
      return importing ? { importing, fraction: fractionOf(progress[importing.job]) } : undefined;
    },
    [entries, progress],
  );

  return { entries, progress, start, fromFiles, fromPaths, choose, dismiss, of, chooser };
}
