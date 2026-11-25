import type { CSSProperties, MouseEvent } from 'react';
import { useT } from '../i18n/index.tsx';
import { Button, Slider, TextInput } from '../kit/controls.tsx';
import { formatBytes, formatDb, formatDuration, importFraction } from '../soundboard.ts';
import type { ImportJob, Pad, SoundInfo } from '../types.ts';
import { Icon } from './Icon.tsx';
import { ShortcutField } from './ShortcutField.tsx';

const colorOf = (pad: Pad): CSSProperties | undefined => (pad.color ? ({ '--pad': pad.color } as CSSProperties) : undefined);

/**
 * One sound of a soundboard, from top to bottom: its name (a field that does not look like one until it is touched), the file with its
 * length and size under it, the volume, then the shortcut, the play button and the one that stops the sound. Everything is always there
 * (nothing moves when a sound starts), and a thin bar along the bottom shows how far the sound has got.
 */
export function PadCard({
  pad,
  sound,
  playing,
  progress,
  canPlay,
  problems,
  onChange,
  onPlay,
  onStop,
  onSettings,
  onRemove,
  onContextMenu,
}: {
  pad: Pad;
  /** The file it plays; none when it was lost. */
  sound: SoundInfo | undefined;
  playing: boolean;
  /** How far the sound has got, from 0 to 1; none while it does not play. */
  progress?: number;
  /** The soundboard is in the running graph. */
  canPlay: boolean;
  problems: string[];
  onChange: (patch: Partial<Pad>) => void;
  onPlay: () => void;
  onStop: () => void;
  onSettings: (e: MouseEvent) => void;
  onRemove: (e: MouseEvent) => void;
  onContextMenu: (e: MouseEvent) => void;
}) {
  const t = useT();
  const gain = pad.gain_db ?? 0;
  const blocked = !sound || !canPlay;
  return (
    <article className={`pad-card${playing ? ' playing' : ''}${sound ? '' : ' lost'}`} style={colorOf(pad)} onContextMenu={onContextMenu}>
      <div className="pad-card-top">
        <TextInput code={false} className="pad-name" title={t('soundboards.padName')} value={pad.name} onChange={(name) => onChange({ name })} />
        <Button variant="flat" size="small" icon="gear" title={t('soundboards.padSettings')} onClick={onSettings} />
        <Button variant="flat" size="small" icon="trash" title={t('soundboards.removePad')} onClick={onRemove} />
      </div>
      <p className="pad-file" title={sound?.name}>
        {sound ? sound.name : t('soundboards.lost')}
      </p>
      <p className="pad-meta">{sound ? `${formatDuration(sound.frames)} · ${formatBytes(sound.bytes)}` : '\u00a0'}</p>
      <div className="pad-volume" title={t('soundboards.volumeHint')}>
        <Icon name="speaker" size={14} />
        <Slider label={t('soundboards.volume')} min={-30} max={12} step={0.5} value={gain} onChange={(gain_db) => onChange({ gain_db })} />
        <span className="pad-db">{formatDb(gain)}</span>
      </div>
      {problems.map((p) => (
        <small key={p} className="warn">
          {p}
        </small>
      ))}
      <div className="pad-actions">
        <ShortcutField value={pad.key ?? ''} placeholder={t('soundboards.shortcutShort')} onChange={(keys) => onChange({ key: keys || null })} />
        <button
          type="button"
          className="pad-play nodrag"
          disabled={blocked}
          title={!sound ? t('soundboards.lost') : !canPlay ? t('soundboards.cannotPlay') : t('soundboards.playPad')}
          onClick={onPlay}
        >
          <Icon name="play" size={14} />
          {t('soundboards.play')}
        </button>
        <button type="button" className="pad-stop nodrag" disabled={!playing} title={t('soundboards.stopPad')} aria-label={t('soundboards.stopPad')} onClick={onStop}>
          <Icon name="stop" size={14} />
        </button>
      </div>
      <div className="pad-progress" role="progressbar" aria-label={t('soundboards.progress')} aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round((progress ?? 0) * 100)}>
        <span style={{ width: `${Math.round((progress ?? 0) * 1000) / 10}%` }} />
      </div>
    </article>
  );
}

/** The card of a sound that is being imported: only the progress bar, until the sound is a card. A failure says why. */
export function ImportCard({ job, label, error, onDismiss }: { job: ImportJob | undefined; label: string; error?: string; onDismiss: () => void }) {
  const t = useT();
  const fraction = job ? importFraction(job.done, job.total) : null;
  return (
    <article className={`pad-card importing${error ? ' failed' : ''}`} aria-busy={!error} aria-label={t('soundboards.importingAria', { name: label })} title={label}>
      {error ? (
        <div className="pad-card-top">
          <p className="pad-file warn">{error}</p>
          <Button variant="flat" size="small" icon="close" title={t('app.close')} onClick={onDismiss} />
        </div>
      ) : (
        <div className="progress thick" role="progressbar" aria-label={label} aria-valuemin={0} aria-valuemax={100} aria-valuenow={fraction === null ? undefined : Math.round(fraction * 100)}>
          <span className={fraction === null ? 'indeterminate' : ''} style={fraction === null ? undefined : { width: `${Math.round(fraction * 100)}%` }} />
        </div>
      )}
    </article>
  );
}
