/** The little line icons of the interface (16 × 16, drawn with the current colour). */
const PATHS = {
  gear: 'M6.9 1.6h2.2l.35 1.5 1.1.45 1.3-.8 1.55 1.55-.8 1.3.45 1.1 1.5.35v2.2l-1.5.35-.45 1.1.8 1.3-1.55 1.55-1.3-.8-1.1.45-.35 1.5H6.9l-.35-1.5-1.1-.45-1.3.8L2.6 11.4l.8-1.3-.45-1.1-1.5-.35V6.4l1.5-.35.45-1.1-.8-1.3L4.15 2.1l1.3.8 1.1-.45zM8 5.8a2.2 2.2 0 1 0 0 4.4 2.2 2.2 0 0 0 0-4.4z',
  arrow: 'M3 8h9.5M8.5 4.2 12.5 8l-4 3.8',
  check: 'M3 8.5l3.2 3.2L13 4.8',
  undo: 'M5.2 3.2 2.6 5.8l2.6 2.6M2.6 5.8h6.4a3.6 3.6 0 0 1 0 7.2H6',
  grid: 'M2.5 2.5h4v4h-4zM9.5 2.5h4v4h-4zM2.5 9.5h4v4h-4zM9.5 9.5h4v4h-4z',
  braces: 'M6 2.5C4.5 2.5 4.5 3.5 4.5 5v1.2c0 .8-.6 1.8-1.5 1.8.9 0 1.5 1 1.5 1.8V11c0 1.5 0 2.5 1.5 2.5M10 2.5c1.5 0 1.5 1 1.5 2.5v1.2c0 .8.6 1.8 1.5 1.8-.9 0-1.5 1-1.5 1.8V11c0 1.5 0 2.5-1.5 2.5',
  play: 'M4.5 3 12.5 8l-8 5z',
  stop: 'M4 4h8v8H4z',
  command: 'M3.5 4.5 7.5 8l-4 3.5M8.5 12h4',
  chevron: 'M3.5 6 8 10.5 12.5 6',
  plus: 'M8 3v10M3 8h10',
  close: 'M3.5 3.5l9 9M12.5 3.5l-9 9',
  search: 'M7 2.5a4.5 4.5 0 1 0 0 9 4.5 4.5 0 0 0 0-9zM10.5 10.5 14 14',
  warning: 'M8 2 14.5 13.5h-13zM8 6.5v3.2M8 11.6v.4',
  error: 'M8 2.5a5.5 5.5 0 1 0 0 11 5.5 5.5 0 0 0 0-11zM5.8 5.8l4.4 4.4M10.2 5.8l-4.4 4.4',
  info: 'M8 2.5a5.5 5.5 0 1 0 0 11 5.5 5.5 0 0 0 0-11zM8 7.2v3.6M8 5.2v.2',
  panel: 'M2.5 3h11v10h-11zM2.5 6.5h11',
  list: 'M5 4h8M5 8h8M5 12h8M2.5 4h.5M2.5 8h.5M2.5 12h.5',
} as const;

export type IconName = keyof typeof PATHS;

export function Icon({ name, size = 16 }: { name: IconName; size?: number }) {
  return (
    <svg className="icon" viewBox="0 0 16 16" width={size} height={size} fill="none" stroke="currentColor" strokeWidth="1.2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <path d={PATHS[name]} />
    </svg>
  );
}
