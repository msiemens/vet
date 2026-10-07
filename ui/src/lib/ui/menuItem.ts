/** One entry of a menu; the same definition drives the in-window `Menu` and the host's native menu bar. */
export interface MenuItem {
  id: string;
  label: string;
  /** Secondary text, e.g. the path behind a recent repository's name. */
  detail?: string;
  /** Display text of the key binding, e.g. `Ctrl+Alt+S`. The menu only shows it; the window's key handler runs it. */
  shortcut?: string;
  /** A key the native menu bar registers and handles itself, e.g. `Cmd+,`; it needs no `shortcut` text. */
  accelerator?: string;
  /** Default true. */
  enabled?: boolean;
  /** Shows a check mark. */
  checked?: boolean;
  /** A divider; no other field is used. */
  separator?: boolean;
  /** Makes it a submenu. */
  children?: MenuItem[];
}

export const separator: MenuItem = { id: "-", label: "", separator: true };
