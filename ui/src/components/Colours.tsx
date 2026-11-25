import { useEffect, useState } from 'react';
import { useT } from '../i18n/index.tsx';
import { Button, ColorField, TextInput } from '../kit/controls.tsx';
import { applyThemeVars, COLOUR_TOKENS, loadThemeVars, useTheme } from '../theme.tsx';

/** What the colour chooser takes: `#rrggbb`. A colour written another way (or not known yet) is shown as black until it is changed. */
function asHex(color: string): string {
  const c = color.trim().toLowerCase();
  if (/^#[0-9a-f]{6}$/.test(c)) return c;
  const short = /^#([0-9a-f])([0-9a-f])([0-9a-f])$/.exec(c);
  return short ? `#${short[1]}${short[1]}${short[2]}${short[2]}${short[3]}${short[3]}` : '#000000';
}

const parse = (text: string): Record<string, string> => {
  try {
    const value: unknown = JSON.parse(text || '{}');
    return typeof value === 'object' && value !== null && !Array.isArray(value) ? (value as Record<string, string>) : {};
  } catch {
    return {};
  }
};

/**
 * The colours of the interface: one swatch for each main colour (the one of the theme until it is changed, with a button to give it back),
 * and the same choices as a text for the person who wants every colour (the families of blocks, the flows…).
 */
export function Colours() {
  const t = useT();
  const { choice } = useTheme();
  const [text, setText] = useState(loadThemeVars);
  const [problem, setProblem] = useState<string | null>(null);
  const [asText, setAsText] = useState(false);
  // the colours of the theme are read from the page once it has changed theme
  const [, reread] = useState(0);
  useEffect(() => {
    applyThemeVars(loadThemeVars());
  }, []);
  useEffect(() => {
    const frame = requestAnimationFrame(() => reread((n) => n + 1));
    return () => cancelAnimationFrame(frame);
  }, [choice]);

  const mine = parse(text);
  const save = (next: Record<string, string>) => {
    const json = JSON.stringify(next);
    setText(json);
    setProblem(applyThemeVars(json));
  };
  const shown = (name: string) => mine[name] ?? getComputedStyle(document.documentElement).getPropertyValue(name);
  const changed = Object.keys(mine).length > 0;

  return (
    <div className="colours">
      <ul className="colour-list">
        {COLOUR_TOKENS.map(({ name, label }) => (
          <li key={name} className={name in mine ? 'changed' : undefined}>
            <ColorField value={asHex(shown(name))} label={t(label)} onChange={(color) => save({ ...mine, [name]: color })} />
            <span>{t(label)}</span>
            <Button
              variant="flat"
              size="small"
              icon="undo"
              title={t('colour.reset', { name: t(label) })}
              disabled={!(name in mine)}
              onClick={() => {
                const { [name]: _gone, ...rest } = mine;
                save(rest);
              }}
            />
          </li>
        ))}
      </ul>
      <div className="row">
        <Button icon="undo" disabled={!changed} onClick={() => save({})}>
          {t('colour.resetAll')}
        </Button>
        <Button variant="flat" pressed={asText} onClick={() => setAsText((v) => !v)}>
          {t('colour.asText')}
        </Button>
      </div>
      {asText && (
        <>
          <small className="muted">{t('colour.asTextHint')}</small>
          <TextInput
            value={text}
            onChange={(value) => {
              setText(value);
              setProblem(applyThemeVars(value));
            }}
          />
        </>
      )}
      {problem && (
        <small className="warn" role="alert">
          {problem}
        </small>
      )}
    </div>
  );
}
