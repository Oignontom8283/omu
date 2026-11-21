import { useCallback, useEffect, useRef, useState } from 'react';
import type { api } from '../api.ts';
import { analyse, type Problem, type Suggestion } from '../calibration.ts';
import { useEditor } from '../context.ts';
import { useT } from '../i18n/index.tsx';
import { Button } from '../kit/controls.tsx';
import type { NodeSpec } from '../types.ts';

const QUIET_SECONDS = 4;
const SPEECH_SECONDS = 6;
const CYCLES_PER_SECOND = 100;

type Phase = 'intro' | 'quiet' | 'speech' | 'result';
type Saved = { date: string; suggestion: Suggestion };

const storageKey = (node: string) => `omu-calibration:${node}`;

function loadSaved(node: string): Saved | null {
  try {
    const raw = localStorage.getItem(storageKey(node));
    return raw ? (JSON.parse(raw) as Saved) : null;
  } catch {
    return null;
  }
}

/**
 * The calibration of a voice, for a noise reduction block: the person keeps quiet for a few seconds, then speaks as they
 * usually do, while the block listens to its input. From what it heard, settings are suggested (and the one the block
 * has is applied on request). The last result is remembered in this browser, per block.
 */
export function Calibration({ spec, daemon, running }: { spec: NodeSpec; daemon: ReturnType<typeof api>; running: boolean }) {
  const t = useT();
  const { update } = useEditor();
  const [phase, setPhase] = useState<Phase>('intro');
  const [left, setLeft] = useState(0);
  const [problem, setProblem] = useState<Problem | 'unavailable' | null>(null);
  const [result, setResult] = useState<Suggestion | null>(null);
  const [saved, setSaved] = useState<Saved | null>(() => loadSaved(spec.id));
  const quiet = useRef<number[]>([]);
  const timer = useRef<number | undefined>(undefined);
  useEffect(() => () => window.clearInterval(timer.current), []);

  /** Counts down `seconds`, then hands over the levels the block heard in that time. */
  const listen = useCallback(
    (seconds: number, done: (levels: number[]) => void) => {
      let remaining = seconds;
      setLeft(remaining);
      window.clearInterval(timer.current);
      timer.current = window.setInterval(() => {
        remaining -= 1;
        setLeft(remaining);
        if (remaining > 0) return;
        window.clearInterval(timer.current);
        daemon
          .levels(spec.id)
          .then((l) => done(l.db.slice(-seconds * CYCLES_PER_SECOND)))
          .catch(() => {
            setProblem('unavailable');
            setPhase('intro');
          });
      }, 1000);
    },
    [daemon, spec.id],
  );

  const start = () => {
    setProblem(null);
    setPhase('quiet');
    listen(QUIET_SECONDS, (levels) => {
      quiet.current = levels;
      setPhase('speech');
      listen(SPEECH_SECONDS, (spoken) => {
        const r = analyse(quiet.current, spoken);
        if ('problem' in r) {
          setProblem(r.problem);
          setPhase('intro');
          return;
        }
        setResult(r);
        const keep: Saved = { date: new Date().toISOString(), suggestion: r };
        setSaved(keep);
        try {
          localStorage.setItem(storageKey(spec.id), JSON.stringify(keep));
        } catch {
          /* storage may be blocked: the result is still shown */
        }
        setPhase('result');
      });
    });
  };

  if (phase === 'quiet' || phase === 'speech') {
    return (
      <div className="calibration">
        <p className="calibration-step">{t(phase === 'quiet' ? 'calibration.beQuiet' : 'calibration.speak')}</p>
        <p className="calibration-count" aria-live="polite">
          {left}
        </p>
        <div className="meter calibration-bar">
          <div className="meter-fill" style={{ width: `${(100 * (phase === 'quiet' ? QUIET_SECONDS - left : SPEECH_SECONDS - left)) / (phase === 'quiet' ? QUIET_SECONDS : SPEECH_SECONDS)}%` }} />
        </div>
      </div>
    );
  }

  if (phase === 'result' && result) {
    const deep = spec.type === 'deep_filter';
    return (
      <div className="calibration">
        <table className="calibration-table">
          <tbody>
            <tr>
              <th>{t('calibration.noise')}</th>
              <td>{result.noiseFloor} dB</td>
            </tr>
            <tr>
              <th>{t('calibration.voice')}</th>
              <td>{result.voice} dB</td>
            </tr>
            <tr>
              <th>{t('calibration.margin')}</th>
              <td>
                {result.margin} dB · <strong className={`quality ${result.quality}`}>{t(`calibration.quality.${result.quality}`)}</strong>
              </td>
            </tr>
          </tbody>
        </table>
        {result.quality === 'poor' && <p className="calibration-advice">{t('calibration.poor')}</p>}
        <p>{t('calibration.suggestGain', { db: `${result.gainDb > 0 ? '+' : ''}${result.gainDb}` })}</p>
        <p>{t('calibration.suggestThreshold', { db: result.thresholdDb })}</p>
        {deep ? (
          <>
            <p>{t(result.limitDb >= 100 ? 'calibration.suggestNoLimit' : 'calibration.suggestLimit', { db: result.limitDb })}</p>
            <Button variant="primary" icon="check" onClick={() => update(spec.id, { type: 'deep_filter', limit_db: result.limitDb })}>
              {t('calibration.apply')}
            </Button>{' '}
          </>
        ) : (
          <p className="muted">{t('calibration.noSetting')}</p>
        )}
        <Button icon="refresh" onClick={() => setPhase('intro')}>
          {t('calibration.again')}
        </Button>
      </div>
    );
  }

  return (
    <div className="calibration">
      <p>{t('calibration.intro')}</p>
      {!running && <p className="calibration-advice">{t('calibration.notRunning')}</p>}
      {problem && <p className="calibration-advice">{t(`calibration.problem.${problem}`)}</p>}
      {saved && (
        <p className="muted">
          {t('calibration.last', { date: new Date(saved.date).toLocaleString(), margin: saved.suggestion.margin })}
        </p>
      )}
      <Button variant="primary" icon="record" onClick={start} disabled={!running}>
        {t('calibration.start')}
      </Button>
    </div>
  );
}
