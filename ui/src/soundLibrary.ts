import { useCallback, useEffect, useState } from 'react';
import type { api } from './api.ts';
import { useT } from './i18n/index.tsx';
import { ask } from './kit/dialogs.tsx';
import { cleanCacheMb, cleanCompressMb, MIN_CACHE_MB } from './soundboard.ts';
import type { Settings, SoundInfo, SoundLibrary, SoundStatus } from './types.ts';

type Daemon = ReturnType<typeof api>;

const message = (e: unknown): string => (e instanceof Error ? e.message : String(e));

/**
 * What the list of imported sounds does, wherever it is shown (the soundboards page, the settings): the memory the sounds use and its
 * limits, compressing a sound, removing one (after a question). Keeps the list the page holds up to date after each change.
 */
export function useSoundLibrary({ daemon, setLibrary, notify }: { daemon: Daemon; setLibrary: (l: SoundLibrary) => void; notify: (text: string) => void }) {
  const t = useT();
  const [status, setStatus] = useState<SoundStatus | null>(null);
  const [settings, setSettings] = useState<Settings | null>(null);
  const [cacheText, setCacheText] = useState('');
  const [compressText, setCompressText] = useState('');
  const [compressing, setCompressing] = useState<string | null>(null);

  useEffect(() => {
    const read = () => daemon.soundStatus().then(setStatus).catch(() => undefined);
    void read();
    const timer = setInterval(read, 1500);
    return () => clearInterval(timer);
  }, [daemon]);
  useEffect(() => {
    daemon
      .settings()
      .then((s) => {
        setSettings(s);
        setCacheText(String(s.sound_cache_mb));
        setCompressText(String(s.compress_above_mb));
      })
      .catch(() => undefined);
  }, [daemon]);

  const reload = useCallback(async () => {
    try {
      setLibrary(await daemon.soundboards());
    } catch (e) {
      notify(message(e));
    }
  }, [daemon, setLibrary, notify]);

  const removeSound = useCallback(
    async (s: SoundInfo, e?: { shiftKey?: boolean }) => {
      if (!(await ask({ title: t('dialog.removeSound.title', { name: s.name }), message: t('dialog.removeSound.message') }, e))) return;
      try {
        await daemon.removeSound(s.id);
      } catch (err) {
        notify(message(err));
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

  const saveCache = useCallback(async () => {
    const mb = cleanCacheMb(cacheText);
    if (mb === null) {
      notify(t('soundboards.cacheInvalid', { min: MIN_CACHE_MB }));
      return;
    }
    const packMb = cleanCompressMb(compressText);
    if (packMb === null) {
      notify(t('soundboards.compressInvalid'));
      return;
    }
    try {
      // never defaults over what the daemon holds: when the settings were not read earlier, they are read now
      const now = settings ?? (await daemon.settings());
      const next: Settings = { ...now, sound_cache_mb: mb, compress_above_mb: packMb };
      await daemon.setSettings(next);
      setSettings(next);
      setCacheText(String(mb));
      setCompressText(String(packMb));
    } catch (e) {
      notify(message(e));
    }
  }, [daemon, notify, t, settings, cacheText, compressText]);

  return { status, settings, cacheText, compressText, compressing, setCacheText, setCompressText, reload, removeSound, compressSound, saveCache };
}
