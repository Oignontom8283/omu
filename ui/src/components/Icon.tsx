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
  minus: 'M3 8h10',
  expand: 'M9.5 2.5h4v4M13.5 2.5 8.5 7.5M6.5 3.5h-3.5v9.5h9.5v-3.5',
  help: 'M8 2.5a5.5 5.5 0 1 0 0 11 5.5 5.5 0 0 0 0-11zM6.3 6.4a1.8 1.8 0 1 1 2.5 1.6c-.5.3-.8.7-.8 1.3M8 11.3v.2',
  trash: 'M2.8 4.5h10.4M6.2 4.5V3h3.6v1.5M4.2 4.5l.6 8.5h6.4l.6-8.5M6.8 7v3.8M9.2 7v3.8',
  folder: 'M2 4.2h4l1.2 1.5H14v7H2z',
  broom: 'M9.5 2.5 7 7M5.2 7h5.6l1.2 6H4zM6.6 10l.4 3M9.4 10l-.4 3',
  stack: 'M8 2.5 14 5.5 8 8.5 2 5.5zM2 8.5l6 3 6-3M2 11l6 3 6-3',
  copy: 'M5.5 5.5h7.5v8H5.5zM3 10.5V2.5h7.5',
  save: 'M3 3h8.5L13 4.5V13H3zM5.5 3v3.2h4.5V3M5.5 13V9.5h5V13',
  record: 'M8 4a4 4 0 1 0 0 8 4 4 0 0 0 0-8z',
  power: 'M8 2.5v5M5 4.6a5 5 0 1 0 6 0',
  refresh: 'M13 8a5 5 0 1 1-1.5-3.6M13 2.5v3h-3',
  close: 'M3.5 3.5l9 9M12.5 3.5l-9 9',
  search: 'M7 2.5a4.5 4.5 0 1 0 0 9 4.5 4.5 0 0 0 0-9zM10.5 10.5 14 14',
  warning: 'M8 2 14.5 13.5h-13zM8 6.5v3.2M8 11.6v.4',
  error: 'M8 2.5a5.5 5.5 0 1 0 0 11 5.5 5.5 0 0 0 0-11zM5.8 5.8l4.4 4.4M10.2 5.8l-4.4 4.4',
  info: 'M8 2.5a5.5 5.5 0 1 0 0 11 5.5 5.5 0 0 0 0-11zM8 7.2v3.6M8 5.2v.2',
  panel: 'M2.5 3h11v10h-11zM2.5 6.5h11',
  list: 'M5 4h8M5 8h8M5 12h8M2.5 4h.5M2.5 8h.5M2.5 12h.5',
  speaker: 'M2.5 6.2h2.2L8 3.5v9L4.7 9.8H2.5zM10.5 6a3 3 0 0 1 0 4M12.2 4.2a5.5 5.5 0 0 1 0 7.6',
  shield: 'M8 2 13 4v4c0 3-2.2 4.8-5 6-2.8-1.2-5-3-5-6V4zM5.8 8l1.7 1.7L10.4 6.6',
  keyboard: 'M1.8 4.5h12.4v7H1.8zM4 7h.4M6.2 7h.4M8.4 7h.4M10.6 7h.4M4.6 9.5h6.8',
} as const;

export type IconName = keyof typeof PATHS;

export function Icon({ name, size = 16 }: { name: IconName; size?: number }) {
  return (
    <svg className="icon" viewBox="0 0 16 16" width={size} height={size} fill="none" stroke="currentColor" strokeWidth="1.2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <path d={PATHS[name]} />
    </svg>
  );
}
