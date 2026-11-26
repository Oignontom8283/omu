import { useCallback, useEffect, useState } from 'react';
import type { api } from './api.ts';
import { useT } from './i18n/index.tsx';
import { ask } from './kit/dialogs.tsx';
import type { SoundInfo, SoundLibrary, SoundStatus } from './types.ts';

type Daemon = ReturnType<typeof api>;

const message = (e: unknown): string => (e instanceof Error ? e.message : String(e));

/**
 * What the list of imported sounds does, wherever it is shown (the soundboards page, the settings): how the memory of the sounds is used,
 * compressing a sound, removing one (after a question). Keeps the list the page holds up to date after each change.
 */
export function useSoundLibrary({ daemon, setLibrary, notify }: { daemon: Daemon; setLibrary: (l: SoundLibrary) => void; notify: (text: string) => void }) {
  const t = useT();
  const [status, setStatus] = useState<SoundStatus | null>(null);
  const [compressing, setCompressing] = useState<string | null>(null);

  useEffect(() => {
    const read = () => daemon.soundStatus().then(setStatus).catch(() => undefined);
    void read();
    const timer = setInterval(read, 1500);
    return () => clearInterval(timer);
  }, [daemon]);
  const reload = useCallback(async () => {
    try {
      setLibrary(await daemon.soundboards());
    } catch (e) {
      notify(message(e));
    }
  }, [daemon, setLibrary, notify]);

  /** Removes sounds after one question (Shift skips it). */
  const removeSounds = useCallback(
    async (list: SoundInfo[], e?: { shiftKey?: boolean }) => {
      if (list.length === 0) return;
      const title = list.length === 1 ? t('dialog.removeSound.title', { name: list[0].name }) : t('dialog.removeSounds.title', { count: list.length });
      if (!(await ask({ title, message: t('dialog.removeSound.message') }, e))) return;
      for (const s of list) {
        try {
          await daemon.removeSound(s.id);
        } catch (err) {
          notify(`${s.name}: ${message(err)}`);
        }
      }
      await reload();
    },
    [daemon, notify, reload, t],
  );

  const compressSound = useCallback(
    async (s: SoundInfo) => {
      setCompressing(s.id);
      try {
        await daemon.compressSound(s.id);
      } catch (e) {
        notify(`${s.name}: ${message(e)}`);
      } finally {
        setCompressing(null);
      }
      await reload();
    },
    [daemon, notify, reload],
  );

  return { status, compressing, reload, removeSounds, compressSound };
}
