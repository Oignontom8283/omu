import { useRef } from 'react';
import { tokenizeTemplate } from '../template.ts';

/**
 * A line of text where the values of a template are coloured as in a code editor: the dollar signs, the names of the values
 * (red and underlined when no value has that name), the defaults, the escaped characters. It is a real text field written
 * in transparent ink over the coloured copy of its text.
 */
export function HighlightedInput({
  value,
  known,
  disabled,
  title,
  onChange,
}: {
  value: string;
  known: (name: string) => boolean;
  disabled?: boolean;
  title?: string;
  onChange: (text: string) => void;
}) {
  const back = useRef<HTMLDivElement>(null);
  // the copy follows the field when what is typed is longer than the field
  const follow = (field: HTMLInputElement) => {
    if (back.current) back.current.scrollLeft = field.scrollLeft;
  };
  return (
    <span className="hl-wrap" title={title}>
      <div ref={back} className="hl-back" aria-hidden="true">
        {tokenizeTemplate(value, known).map((token, i) => (
          <span key={i} className={`tok-${token.kind}`}>
            {token.text}
          </span>
        ))}
      </div>
      <input
        className="nodrag hl-input"
        value={value}
        disabled={disabled}
        spellCheck={false}
        onChange={(e) => {
          onChange(e.target.value);
          follow(e.target);
        }}
        onScroll={(e) => follow(e.currentTarget)}
        onKeyUp={(e) => follow(e.currentTarget)}
        onSelect={(e) => follow(e.currentTarget)}
      />
    </span>
  );
}
