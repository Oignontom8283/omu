import type { CSSProperties, MouseEvent } from 'react';
import { useT } from '../i18n/index.tsx';
import { Button, Slider, TextInput } from '../kit/controls.tsx';
import { formatBytes, formatDb, formatDuration, importFraction } from '../soundboard.ts';
import type { ImportJob, Pad, SoundInfo } from '../types.ts';
import { Icon } from './Icon.tsx';
import { ShortcutField } from './ShortcutField.tsx';

const colorOf = (pad: Pad): CSSProperties | undefined => (pad.color ? ({ '--pad': pad.color } as CSSProperties) : undefined);

/**
 * One sound of a soundboard. The top line: play, the name (the file's until it is changed), its settings, its removal. Under it the
 * file, whole (it is what tells two sounds apart), then the volume and the shortcut.
 */
export function PadCard({
  pad,
  sound,
  playing,
  canPlay,
  problems,
  confirming,
  onChange,
  onPlay,
  onStop,
  onSettings,
  onRemove,
}: {
  pad: Pad;
  /** The file it plays; none when it was lost. */
  sound: SoundInfo | undefined;
  playing: boolean;
  /** The soundboard is in the running graph. */
  canPlay: boolean;
  problems: string[];
  /** The remove button has been pressed once: it asks to be pressed again. */
  confirming: boolean;
  onChange: (patch: Partial<Pad>) => void;
  onPlay: () => void;
  onStop: () => void;
  onSettings: (e: MouseEvent) => void;
  onRemove: () => void;
}) {
  const t = useT();
  const gain = pad.gain_db ?? 0;
  const blocked = !sound || (!canPlay && !playing);
  return (
    <article className={`pad-card${playing ? ' playing' : ''}${sound ? '' : ' lost'}`} style={colorOf(pad)}>
      <div className="pad-card-top">
        <button
          type="button"
          className="pad-play nodrag"
          disabled={blocked}
          title={!sound ? t('soundboards.lost') : !canPlay && !playing ? t('soundboards.cannotPlay') : playing ? t('soundboards.stopPad') : t('soundboards.playPad')}
          aria-label={playing ? t('soundboards.stopPad') : t('soundboards.playPad')}
          onClick={playing ? onStop : onPlay}
        >
          <Icon name={playing ? 'stop' : 'play'} size={16} />
        </button>
        <TextInput code={false} className="pad-name" title={t('soundboards.padName')} value={pad.name} onChange={(name) => onChange({ name })} />
        <Button variant="flat" size="small" icon="gear" title={t('soundboards.padSettings')} onClick={onSettings} />
        <Button variant={confirming ? 'danger' : 'flat'} size="small" icon="trash" title={confirming ? t('soundboards.deleteConfirm') : t('soundboards.removePad')} onClick={onRemove} />
      </div>
      <p className="pad-file" title={sound?.name}>
        {sound ? sound.name : t('soundboards.lost')}
        {sound && (
          <span className="pad-meta">
            {formatDuration(sound.frames)} · {formatBytes(sound.bytes)}
          </span>
        )}
      </p>
      <div className="pad-card-bottom">
        <div className="pad-volume" title={t('soundboards.volumeHint')}>
          <Icon name="speaker" size={14} />
          <Slider label={t('soundboards.volume')} min={-30} max={12} step={0.5} value={gain} onChange={(gain_db) => onChange({ gain_db })} />
          <span className="pad-db">{formatDb(gain)}</span>
        </div>
        <ShortcutField value={pad.key ?? ''} placeholder={t('soundboards.shortcutShort')} onChange={(keys) => onChange({ key: keys || null })} />
      </div>
      {problems.map((p) => (
        <small key={p} className="warn">
          {p}
        </small>
      ))}
    </article>
  );
}

/** The card of a sound that is being imported: its file, the step it is at and how far (no picture: there is nothing to see yet). */
export function ImportCard({ job, label, waiting, error, onDismiss }: { job: ImportJob | undefined; label: string; waiting: boolean; error?: string; onDismiss: () => void }) {
  const t = useT();
  const fraction = job ? importFraction(job.done, job.total) : null;
  const step = waiting || !job ? t('soundboards.step.waiting') : t(`soundboards.step.${job.step}`);
  return (
    <article className={`pad-card importing${error ? ' failed' : ''}`} aria-busy={!error}>
      <div className="pad-card-top">
        <strong className="pad-title" title={label}>
          {label}
        </strong>
        {error && <Button variant="flat" size="small" icon="close" title={t('app.close')} onClick={onDismiss} />}
      </div>
      {error ? (
        <p className="pad-file warn">{error}</p>
      ) : (
        <>
          <div className="progress" role="progressbar" aria-valuemin={0} aria-valuemax={100} aria-valuenow={fraction === null ? undefined : Math.round(fraction * 100)}>
            <span className={fraction === null ? 'indeterminate' : ''} style={fraction === null ? undefined : { width: `${Math.round(fraction * 100)}%` }} />
          </div>
          <p className="pad-file">
            {step}
            {fraction !== null && <span className="pad-meta">{Math.round(fraction * 100)} %</span>}
          </p>
        </>
      )}
    </article>
  );
}
