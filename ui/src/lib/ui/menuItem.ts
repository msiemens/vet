/** One entry of a menu; the same definition drives the in-window `Menu` and the host's native menu bar. */
export interface MenuItem {
  id: string;
  label: string;
  /** Secondary text, e.g. where a run target came from. */
  detail?: string;
  /** Display text of the key binding, e.g. `Ctrl+Alt+S`. */
  shortcut?: string;
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
