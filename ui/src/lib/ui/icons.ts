// 24x24 icons. `fill` icons are solid shapes, the rest are strokes.
export const ICONS = {
  menu: '<path d="M4 7h16M4 12h16M4 17h16"/>',
  queue: '<path d="M4 6h16M4 12h16M4 18h10"/>',
  files: '<path d="M3 7a2 2 0 012-2h4l2 2h8a2 2 0 012 2v8a2 2 0 01-2 2H5a2 2 0 01-2-2z"/>',
  timeline: '<circle cx="12" cy="12" r="8"/><path d="M12 8v4l3 2"/>',
  comments: '<path d="M4 5h16v11H9l-5 4z"/>',
  branch:
    '<circle cx="6" cy="6" r="2"/><circle cx="6" cy="18" r="2"/><circle cx="18" cy="8" r="2"/><path d="M6 8v8M18 10c0 4-6 3-11 7"/>',
  arrow: '<path d="M5 12h14M13 6l6 6-6 6"/>',
  up: '<path d="M12 19V5M6 11l6-6 6 6"/>',
  down: '<path d="M12 5v14M6 13l6 6 6-6"/>',
  filter: '<path d="M4 5h16l-6 7v6l-4 2v-8z"/>',
  search: '<circle cx="11" cy="11" r="6.5"/><path d="M20 20l-4.2-4.2"/>',
  caret: '<path d="M6 9l6 6 6-6"/>',
  check: '<path d="M5 12.5l4.5 4.5L19 7.5"/>',
  reconfirm: '<path d="M20 11a8 8 0 10-2.3 5.7M20 5v6h-6"/>',
  derived: '<circle cx="12" cy="12" r="8" stroke-dasharray="2.5 2.5"/>',
  build: '<path d="M14.7 6.3a4 4 0 00-5.4 5.4L3 18l3 3 6.3-6.3a4 4 0 005.4-5.4l-2.6 2.6-2.8-.6-.6-2.8z"/>',
  more: '<circle cx="5" cy="12" r="1.6"/><circle cx="12" cy="12" r="1.6"/><circle cx="19" cy="12" r="1.6"/>',
  play: '<path d="M7 4l13 8-13 8z"/>',
  stop: '<rect x="6" y="6" width="12" height="12" rx="1.5"/>',
  open: '<path d="M6 8l6 8 6-8z"/>',
  closed: '<path d="M9 6l8 6-8 6z"/>',
  file: '<path d="M7 3h7l5 5v13H7z"/><path d="M14 3v5h5"/>',
  image:
    '<rect x="4" y="5" width="16" height="14" rx="2"/><circle cx="9" cy="10" r="1.5"/><path d="M5 17l5-4 3 3 3-2 3 3"/>',
  lock: '<rect x="6" y="11" width="12" height="9" rx="2"/><path d="M9 11V8a3 3 0 016 0v3"/>',
  link: '<path d="M9 15l6-6M7 11l-2 2a3.5 3.5 0 005 5l2-2M17 13l2-2a3.5 3.5 0 00-5-5l-2 2"/>',
  module: '<path d="M12 3l8 4.5v9L12 21l-8-4.5v-9z"/><path d="M12 12l8-4.5M12 12L4 7.5M12 12v9"/>',
  close: '<path d="M6 6l12 12M18 6L6 18"/>',
  key: '<circle cx="8" cy="15" r="3.5"/><path d="M11 12.5L20 4M16 8l2.5 2.5"/>',
} as const;
export const SOLID: ReadonlySet<string> = new Set(["play", "stop", "open", "closed", "more"]);

export type IconName = keyof typeof ICONS;

/** Markup for code that builds DOM outside Svelte (CodeMirror widgets). */
export function iconSvg(name: IconName, size = 13, stroke = 2): string {
  const solid = SOLID.has(name);
  return `<svg width="${size}" height="${size}" viewBox="0 0 24 24" fill="${solid ? "currentColor" : "none"}" stroke="${solid ? "none" : "currentColor"}" stroke-width="${stroke}" aria-hidden="true">${ICONS[name]}</svg>`;
}
