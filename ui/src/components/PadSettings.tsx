import type { ReactNode } from 'react';
import { ON_THE_DAEMON_COMPUTER } from '../api.ts';
import { useT } from '../i18n/index.tsx';
import { Button, ColorField, NumberField, Slider, Switch, TextInput } from '../kit/controls.tsx';
import { formatBytes, formatDb, formatDuration, partChange } from '../soundboard.ts';
import type { Pad, SoundInfo } from '../types.ts';
import { ShortcutField } from './ShortcutField.tsx';

/** A row of settings: its name on the left, its control on the right (the same columns in every window). */
function Row({ name, hint, children }: { name: string; hint?: string; children: ReactNode }) {
  return (
    <div className="srow" title={hint}>
      <span>{name}</span>
      <div>{children}</div>
    </div>
  );
}

/** The settings of a sound that its card has no room for: where it starts and ends, its fade, whether it loops, its colour... */
export function PadSettings({
  pad,
  sound,
  problems,
  compressing,
  onChange,
  onCompress,
  onReveal,
}: {
  pad: Pad;
  sound: SoundInfo | undefined;
  problems: string[];
  compressing: boolean;
  onChange: (patch: Partial<Pad>) => void;
  onCompress: () => void;
  onReveal: () => void;
}) {
  const t = useT();
  const gain = pad.gain_db ?? 0;
  return (
    <div className="settings-grid">
      <Row name={t('soundboards.padName')}>
        <TextInput code={false} value={pad.name} onChange={(name) => onChange({ name })} />
      </Row>
      {sound && (
        <Row name={t('soundboards.file')}>
          <div className="settings-file">
            <span title={sound.name}>{sound.name}</span>
            <small className="muted">
              {formatDuration(sound.frames)} · {formatBytes(sound.bytes)}
              {sound.compressed ? ` · ${t('soundboards.compressed')}` : ''}
            </small>
            {ON_THE_DAEMON_COMPUTER && (
              <Button size="small" icon="folder" title={t('soundboards.showFile')} onClick={onReveal}>
                {t('soundboards.showFileShort')}
              </Button>
            )}
            {!sound.compressed && (
              <Button size="small" icon="stack" disabled={compressing} title={t('soundboards.compressNow')} onClick={onCompress}>
                {compressing ? t('soundboards.compressing') : t('soundboards.compress')}
              </Button>
            )}
          </div>
        </Row>
      )}
      <Row name={t('soundboards.volume')} hint={t('soundboards.volumeHint')}>
        <div className="srow-inline">
          <Slider label={t('soundboards.volume')} min={-30} max={12} step={0.5} value={gain} onChange={(gain_db) => onChange({ gain_db })} />
          <span className="pad-db">{formatDb(gain)}</span>
        </div>
      </Row>
      <Row name={t('soundboards.padKey')}>
        <ShortcutField value={pad.key ?? ''} onChange={(keys) => onChange({ key: keys || null })} />
        {problems.map((p) => (
          <small key={p} className="warn">
            {p}
          </small>
        ))}
        {pad.key && (
          <Switch checked={pad.hold ?? false} title={t('soundboards.holdHint')} onChange={(hold) => onChange({ hold })}>
            {t('soundboards.hold')}
          </Switch>
        )}
      </Row>
      <Row name={t('soundboards.start')} hint={t('soundboards.partHint')}>
        <NumberField min={0} step={0.1} unit="s" value={(pad.start_ms ?? 0) / 1000} onChange={(s) => onChange(partChange(pad, 'start_ms', String(s)))} />
      </Row>
      <Row name={t('soundboards.end')} hint={t('soundboards.partHint')}>
        <NumberField min={0} step={0.1} unit="s" value={(pad.end_ms ?? 0) / 1000} onChange={(s) => onChange(partChange(pad, 'end_ms', String(s)))} />
      </Row>
      <Row name={t('soundboards.fade')} hint={t('soundboards.fadeHint')}>
        <div className="srow-inline">
          <Slider label={t('soundboards.fade')} min={0} max={5000} step={50} value={pad.fade_ms ?? 0} onChange={(fade_ms) => onChange({ fade_ms })} />
          <span className="pad-db">{pad.fade_ms ? `${pad.fade_ms} ms` : t('soundboards.fadeShort')}</span>
        </div>
      </Row>
      <Row name={t('soundboards.options')}>
        <div className="checks">
          <Switch checked={pad.looped ?? false} title={t('soundboards.loopHint')} onChange={(looped) => onChange({ looped })}>
            {t('soundboards.loop')}
          </Switch>
          <Switch checked={pad.exclusive ?? false} title={t('soundboards.exclusiveHint')} onChange={(exclusive) => onChange({ exclusive })}>
            {t('soundboards.exclusive')}
          </Switch>
        </div>
      </Row>
      <Row name={t('soundboards.color')}>
        <div className="srow-inline">
          <ColorField label={t('soundboards.color')} value={pad.color ?? '#0f9d8a'} onChange={(color) => onChange({ color })} />
          {pad.color && <Button variant="flat" size="small" icon="close" title={t('soundboards.colorReset')} onClick={() => onChange({ color: null })} />}
        </div>
      </Row>
    </div>
  );
}
