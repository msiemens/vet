// 24x24 icons unless VIEW_BOX says otherwise. SOLID icons are filled shapes, the rest are strokes.
export const ICONS = {
  menu: '<path d="M4 7h16M4 12h16M4 17h16"/>',
  // Lucide's inbox (ISC).
  queue:
    '<path d="M22 12h-6l-2 3h-4l-2-3H2"/><path d="M5.45 5.11 2 12v6a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2v-6l-3.45-6.89A2 2 0 0 0 16.76 4H7.24a2 2 0 0 0-1.79 1.11z"/>',
  // Lucide's file-diff (ISC).
  review:
    '<path d="M6 22a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h8a2.4 2.4 0 0 1 1.704.706l3.588 3.588A2.4 2.4 0 0 1 20 8v12a2 2 0 0 1-2 2z"/><path d="M9 10h6M12 13V7M9 17h6"/>',
  files: '<path d="M3 7a2 2 0 012-2h4l2 2h8a2 2 0 012 2v8a2 2 0 01-2 2H5a2 2 0 01-2-2z"/>',
  // Lucide's folder-tree (ISC).
  tree: '<path d="M20 10a1 1 0 0 0 1-1V6a1 1 0 0 0-1-1h-2.5a1 1 0 0 1-.8-.4l-.9-1.2A1 1 0 0 0 15 3h-2a1 1 0 0 0-1 1v5a1 1 0 0 0 1 1Z"/><path d="M20 21a1 1 0 0 0 1-1v-3a1 1 0 0 0-1-1h-2.9a1 1 0 0 1-.88-.55l-.42-.85a1 1 0 0 0-.92-.6H13a1 1 0 0 0-1 1v5a1 1 0 0 0 1 1Z"/><path d="M3 5a2 2 0 0 0 2 2h3"/><path d="M3 3v13a2 2 0 0 0 2 2h3"/>',
  // Lucide's list-tree (ISC).
  structure:
    '<path d="M21 12h-8"/><path d="M21 6H8"/><path d="M21 18h-8"/><path d="M3 6v4c0 1.1.9 2 2 2h3"/><path d="M3 10v6c0 1.1.9 2 2 2h3"/>',
  // Lucide's git-commit-vertical (ISC).
  gitCommit: '<path d="M12 3v6"/><circle cx="12" cy="12" r="3"/><path d="M12 15v6"/>',
  // Lucide's locate-fixed (ISC).
  locate:
    '<line x1="2" x2="5" y1="12" y2="12"/><line x1="19" x2="22" y1="12" y2="12"/><line x1="12" x2="12" y1="2" y2="5"/><line x1="12" x2="12" y1="19" y2="22"/><circle cx="12" cy="12" r="7"/><circle cx="12" cy="12" r="3"/>',
  timeline: '<circle cx="12" cy="12" r="8"/><path d="M12 8v4l3 2"/>',
  comments: '<path d="M4 5h16v11H9l-5 4z"/>',
  branch:
    '<circle cx="6" cy="6" r="2"/><circle cx="6" cy="18" r="2"/><circle cx="18" cy="8" r="2"/><path d="M6 8v8M18 10c0 4-6 3-11 7"/>',
  commit: '<circle cx="12" cy="12" r="3.5"/><path d="M3 12h5.5M15.5 12H21"/>',
  merge:
    '<circle cx="6" cy="5" r="2"/><circle cx="6" cy="19" r="2"/><circle cx="18" cy="12" r="2"/><path d="M6 7v10M16 12h-5a5 5 0 01-5-5"/>',
  arrow: '<path d="M5 12h14M13 6l6 6-6 6"/>',
  up: '<path d="M12 19V5M6 11l6-6 6 6"/>',
  down: '<path d="M12 5v14M6 13l6 6 6-6"/>',
  filter: '<path d="M4 5h16l-6 7v6l-4 2v-8z"/>',
  search: '<circle cx="11" cy="11" r="6.5"/><path d="M20 20l-4.2-4.2"/>',
  find: '<circle cx="10.5" cy="10.5" r="6.5"/><path d="M20 20l-5-5"/><path d="M8 10.5h5"/>',
  usages: '<circle cx="6" cy="12" r="2.5"/><path d="M8.5 12H14"/><path d="m14 8 4 4-4 4"/><path d="M21 5v14"/>',
  // Lucide's pin (ISC).
  pin: '<path d="M12 17v5"/><path d="M9 10.76a2 2 0 0 1-1.11 1.79l-1.78.9A2 2 0 0 0 5 15.24V16a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1v-.76a2 2 0 0 0-1.11-1.79l-1.78-.9A2 2 0 0 1 15 10.76V7a1 1 0 0 1 1-1 2 2 0 0 0 0-4H8a2 2 0 0 0 0 4 1 1 0 0 1 1 1z"/>',
  // Lucide's rotate-cw (ISC).
  refresh: '<path d="M21 12a9 9 0 1 1-9-9c2.52 0 4.93 1 6.74 2.74L21 8"/><path d="M21 3v5h-5"/>',
  stop: '<rect x="6" y="6" width="12" height="12" rx="1.8"/>',
  // Lucide's panel-right (ISC).
  preview: '<rect width="18" height="16" x="3" y="4" rx="2"/><path d="M14 4v16"/>',
  check: '<path d="M5 12.5l4.5 4.5L19 7.5"/>',
  // Lucide's zap (ISC).
  zap: '<path d="M4 14a1 1 0 0 1-.78-1.63l9.9-10.2a.5.5 0 0 1 .86.46l-1.92 6.02A1 1 0 0 0 13 10h7a1 1 0 0 1 .78 1.63l-9.9 10.2a.5.5 0 0 1-.86-.46l1.92-6.02A1 1 0 0 0 11 14z"/>',
  // Lucide's flag (ISC).
  flag: '<path d="M4 15s1-1 4-1 5 2 8 2 4-1 4-1V3s-1 1-4 1-5-2-8-2-4 1-4 1z"/><line x1="4" x2="4" y1="22" y2="15"/>',
  reconfirm: '<path d="M20 11a8 8 0 10-2.3 5.7M20 5v6h-6"/>',
  more: '<circle cx="5" cy="12" r="1.6"/><circle cx="12" cy="12" r="1.6"/><circle cx="19" cy="12" r="1.6"/>',
  expandAll: '<path d="M7 7l5 5 5-5M7 13l5 5 5-5"/>',
  collapseAll: '<path d="M7 11l5-5 5 5M7 17l5-5 5 5"/>',
  open: '<path d="M6 8l6 8 6-8z"/>',
  closed: '<path d="M9 6l8 6-8 6z"/>',
  file: '<path d="M7 3h7l5 5v13H7z"/><path d="M14 3v5h5"/>',
  image:
    '<rect x="4" y="5" width="16" height="14" rx="2"/><circle cx="9" cy="10" r="1.5"/><path d="M5 17l5-4 3 3 3-2 3 3"/>',
  lock: '<rect x="6" y="11" width="12" height="9" rx="2"/><path d="M9 11V8a3 3 0 016 0v3"/>',
  // Lucide's external-link (ISC).
  external:
    '<path d="M15 3h6v6"/><path d="M10 14 21 3"/><path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6"/>',
  module: '<path d="M12 3l8 4.5v9L12 21l-8-4.5v-9z"/><path d="M12 12l8-4.5M12 12L4 7.5M12 12v9"/>',
  close: '<path d="M6 6l12 12M18 6L6 18"/>',
  // The IntelliJ Platform's status icons (expui/status/{info,warning,error}.svg, Apache 2.0, JetBrains s.r.o.),
  // each shape and its glyph as one path with the glyph cut out; see THIRD_PARTY_NOTICES.md.
  info: '<path fill-rule="evenodd" d="M1 8a7 7 0 1 0 14 0A7 7 0 0 0 1 8ZM7 11C7 11.5523 7.44772 12 8 12C8.55228 12 9 11.5523 9 11L9 8.5C9 7.94771 8.55228 7.5 8 7.5C7.44771 7.5 7 7.94772 7 8.5L7 11ZM8 4C7.44771 4 7 4.44772 7 5C7 5.55228 7.44771 6 8 6C8.55228 6 9 5.55228 9 5C9 4.44772 8.55228 4 8 4Z"/>',
  warning:
    '<path fill-rule="evenodd" d="M1.27603 10.8634L6.3028 1.98903C7.04977 0.670323 8.94893 0.670326 9.69589 1.98903L14.7227 10.8634C15.516 12.2639 14.5047 14 12.8956 14H3.10308C1.494 14 0.482737 12.2639 1.27603 10.8634ZM9 5C9 4.44772 8.55228 4 8 4C7.44772 4 7 4.44772 7 5V7.5C7 8.05229 7.44772 8.5 8 8.5C8.55229 8.5 9 8.05228 9 7.5L9 5ZM8 12C8.55228 12 9 11.5523 9 11C9 10.4477 8.55228 10 8 10C7.44772 10 7 10.4477 7 11C7 11.5523 7.44772 12 8 12Z"/>',
  error:
    '<path fill-rule="evenodd" d="M1 8a7 7 0 1 0 14 0A7 7 0 0 0 1 8ZM9 5C9 4.44772 8.55228 4 8 4C7.44772 4 7 4.44772 7 5V7.5C7 8.05229 7.44772 8.5 8 8.5C8.55229 8.5 9 8.05228 9 7.5L9 5ZM8 12C8.55228 12 9 11.5523 9 11C9 10.4477 8.55228 10 8 10C7.44772 10 7 10.4477 7 11C7 11.5523 7.44772 12 8 12Z"/>',
  // The IntelliJ Platform's outline Problems icon (expui/toolwindows/problems.svg, Apache 2.0, JetBrains s.r.o.).
  problems:
    '<circle cx="8" cy="8" r="6.5" stroke-width="1"/><path d="M8 4.6V8.4" stroke-width="1.2" stroke-linecap="round"/><circle cx="8.00078" cy="10.7" r="0.5" fill="currentColor" stroke-width="0.4"/>',
  minus: '<path d="M6 12h12"/>',
  // The marks of a view that shows no file.
  emptyWatch:
    '<path d="M2.5 12S6 5.5 12 5.5 21.5 12 21.5 12 18 18.5 12 18.5 2.5 12 2.5 12z"/><circle cx="12" cy="12" r="2.8"/>',
  emptyMerge:
    '<circle cx="6.5" cy="5.5" r="2.2"/><circle cx="6.5" cy="18.5" r="2.2"/><circle cx="17.5" cy="12" r="2.2"/><path d="M6.5 7.7v8.6M8.4 6.6c5 1.2 7 2.6 7.2 3.4"/>',
  emptyFilter: '<path d="M3.5 5h17l-6.5 7.8V19l-4 1.5v-7.7z" stroke-linejoin="round"/>',
  emptyCommit: '<circle cx="12" cy="12" r="3.8"/><path d="M2.5 12h5.7M15.8 12h5.7"/>',
  emptyRange: '<circle cx="12" cy="12" r="8.5"/><path d="M12 7v5l3.2 2" stroke-linecap="round"/>',
  emptyCheck:
    '<circle cx="12" cy="12" r="8.5"/><path d="M8 12.4l2.7 2.7L16.2 9.6" stroke-linecap="round" stroke-linejoin="round"/>',
  emptyMinus: '<circle cx="12" cy="12" r="8.5"/><path d="M8 12h8" stroke-linecap="round"/>',
  eye: '<path d="M2 12s3.6-7 10-7 10 7 10 7-3.6 7-10 7S2 12 2 12z"/><circle cx="12" cy="12" r="3"/>',
  user: '<circle cx="12" cy="8" r="4"/><path d="M4.5 20a7.5 7.5 0 0115 0"/>',
  settings:
    '<path d="M12.22 2h-.44a2 2 0 00-2 2v.18a2 2 0 01-1 1.73l-.43.25a2 2 0 01-2 0l-.15-.08a2 2 0 00-2.73.73l-.22.38a2 2 0 00.73 2.73l.15.1a2 2 0 011 1.72v.51a2 2 0 01-1 1.74l-.15.09a2 2 0 00-.73 2.73l.22.38a2 2 0 002.73.73l.15-.08a2 2 0 012 0l.43.25a2 2 0 011 1.73V20a2 2 0 002 2h.44a2 2 0 002-2v-.18a2 2 0 011-1.73l.43-.25a2 2 0 012 0l.15.08a2 2 0 002.73-.73l.22-.39a2 2 0 00-.73-2.73l-.15-.08a2 2 0 01-1-1.74v-.5a2 2 0 011-1.74l.15-.09a2 2 0 00.73-2.73l-.22-.38a2 2 0 00-2.73-.73l-.15.08a2 2 0 01-2 0l-.43-.25a2 2 0 01-1-1.73V4a2 2 0 00-2-2z"/><circle cx="12" cy="12" r="3"/>',
  key: '<circle cx="8" cy="15" r="3.5"/><path d="M11 12.5L20 4M16 8l2.5 2.5"/>',
  edit: '<path d="M4 20h4L19 9l-4-4L4 16z"/><path d="M13.5 6.5l4 4"/>',
  bell: '<path d="M6 17V11a6 6 0 0112 0v6l1.5 2h-15z"/><path d="M10 21h4"/>',
  // Lucide's send (ISC).
  outgoing:
    '<path d="M14.536 21.686a.5.5 0 0 0 .937-.024l6.5-19a.496.496 0 0 0-.635-.635l-19 6.5a.5.5 0 0 0-.024.937l7.93 3.18a2 2 0 0 1 1.112 1.11z"/><path d="m21.854 2.147-10.94 10.939"/>',
  // Lucide's workflow (ISC).
  pipelines:
    '<rect width="8" height="8" x="3" y="3" rx="2"/><path d="M7 11v4a2 2 0 0 0 2 2h4"/><rect width="8" height="8" x="13" y="13" rx="2"/>',
  stop: '<rect x="6.5" y="6.5" width="11" height="11" rx="1.5"/>',
  // Platform marks from simple-icons (CC0).
  github:
    '<path d="M12 .297c-6.63 0-12 5.373-12 12 0 5.303 3.438 9.8 8.205 11.385.6.113.82-.258.82-.577 0-.285-.01-1.04-.015-2.04-3.338.724-4.042-1.61-4.042-1.61C4.422 18.07 3.633 17.7 3.633 17.7c-1.087-.744.084-.729.084-.729 1.205.084 1.838 1.236 1.838 1.236 1.07 1.835 2.809 1.305 3.495.998.108-.776.417-1.305.76-1.605-2.665-.3-5.466-1.332-5.466-5.93 0-1.31.465-2.38 1.235-3.22-.135-.303-.54-1.523.105-3.176 0 0 1.005-.322 3.3 1.23.96-.267 1.98-.399 3-.405 1.02.006 2.04.138 3 .405 2.28-1.552 3.285-1.23 3.285-1.23.645 1.653.24 2.873.12 3.176.765.84 1.23 1.91 1.23 3.22 0 4.61-2.805 5.625-5.475 5.92.42.36.81 1.096.81 2.22 0 1.606-.015 2.896-.015 3.286 0 .315.21.69.825.57C20.565 22.092 24 17.592 24 12.297c0-6.627-5.373-12-12-12"/>',
  azureDevOps:
    '<path d="M0 8.877L2.247 5.91l8.405-3.416V.022l7.37 5.393L2.966 8.338v8.225L0 15.707zm24-4.45v14.651l-5.753 4.9-9.303-3.057v3.056l-5.978-7.416 15.057 1.798V5.415z"/>',
} as const;
export const SOLID: ReadonlySet<string> = new Set([
  "open",
  "closed",
  "more",
  "stop",
  "github",
  "azureDevOps",
  "info",
  "warning",
  "error",
]);

/** The icons that are not drawn on the 24x24 grid, with the area of their own grid they fill (the 16x16 grid of the IntelliJ Platform's icons, less its empty margin). */
const VIEW_BOX: Partial<Record<string, string>> = {
  info: "1 1 14 14",
  warning: "1 1 14 14",
  error: "1 1 14 14",
  problems: "1 1 14 14",
};

export type IconName = keyof typeof ICONS;

/** The attributes of an icon's `<svg>` besides its size and stroke width. */
export function iconFrame(name: IconName): { viewBox: string; fill: string; stroke: string } {
  const solid = SOLID.has(name);
  return {
    viewBox: VIEW_BOX[name] ?? "0 0 24 24",
    fill: solid ? "currentColor" : "none",
    stroke: solid ? "none" : "currentColor",
  };
}

/** Markup for code that builds DOM outside Svelte (CodeMirror widgets). */
export function iconSvg(name: IconName, size = 13, stroke = 2): string {
  const { viewBox, fill, stroke: strokeColor } = iconFrame(name);
  return `<svg width="${size}" height="${size}" viewBox="${viewBox}" fill="${fill}" stroke="${strokeColor}" stroke-width="${stroke}" aria-hidden="true">${ICONS[name]}</svg>`;
}
