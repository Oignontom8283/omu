import type { api } from '../api.ts';
import type { SoundImports } from '../imports.tsx';
import { useT } from '../i18n/index.tsx';
import { Button } from '../kit/controls.tsx';
import { useSoundLibrary } from '../soundLibrary.ts';
import type { SoundLibrary as Library } from '../types.ts';
import { ImportCard } from './PadCard.tsx';
import { SoundLibrary } from './SoundLibrary.tsx';

/**
 * Every sound that was imported, as a table, with the memory the sounds may use, and a button to import more. It is shown in the window
 * of the soundboards page and in the settings: the sounds belong to the program, not to one soundboard.
 */
export function SoundsManager({ daemon, library, setLibrary, imports, notify }: { daemon: ReturnType<typeof api>; library: Library; setLibrary: (l: Library) => void; imports: SoundImports; notify: (text: string) => void }) {
  const t = useT();
  const lib = useSoundLibrary({ daemon, setLibrary, notify });
  const mine = imports.entries.filter((e) => e.owner === 'library');
  return (
    <div className="sounds-manager">
      <div className="sounds-import">
        <Button variant="primary" icon="plus" onClick={() => imports.choose('library', () => lib.reload())}>
          {t('soundboards.addSounds')}
        </Button>
      </div>
      {mine.length > 0 && (
        <div className="sounds-importing">
          {mine.map((i) => (
            <ImportCard key={i.job} job={imports.progress[i.job]} label={i.label} error={i.error} onDismiss={() => imports.dismiss(i.job)} />
          ))}
        </div>
      )}
      <SoundLibrary
        library={library}
        status={lib.status}
        settings={lib.settings}
        cacheText={lib.cacheText}
        compressText={lib.compressText}
        compressing={lib.compressing}
        onCacheText={lib.setCacheText}
        onCompressText={lib.setCompressText}
        onSaveSettings={() => void lib.saveCache()}
        onRemove={(id, e) => {
          const s = library.sounds.find((x) => x.id === id);
          if (s) void lib.removeSound(s, e);
        }}
        onReveal={(id) => void daemon.revealSound(id).catch((e) => notify(e instanceof Error ? e.message : String(e)))}
        onCompress={(id) => {
          const s = library.sounds.find((x) => x.id === id);
          if (s) void lib.compressSound(s);
        }}
      />
    </div>
  );
}
