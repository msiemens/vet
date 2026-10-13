/** The items the host builds from the OS's own definitions (macOS), such as Services or Undo. */
export type PredefinedItem =
  | "undo"
  | "redo"
  | "cut"
  | "copy"
  | "paste"
  | "selectAll"
  | "services"
  | "hide"
  | "hideOthers"
  | "showAll"
  | "minimize"
  | "zoom"
  | "fullscreen"
  | "bringAllToFront";

/** One entry of a menu; the same definition drives the in-window `Menu` and the host's native menu bar. */
export interface MenuItem {
  id: string;
  label: string;
  /** Secondary text, e.g. the path behind a recent repository's name. */
  detail?: string;
  /** Display text of the key binding, e.g. `Ctrl+Alt+S`. The menu only shows it; the window's key handler runs it. */
  shortcut?: string;
  /** The binding as a native menu reads an accelerator (`Cmd+B`); a native context menu draws it itself. */
  chord?: string;
  /** A key the native menu bar registers and handles itself, e.g. `Cmd+,`; it needs no `shortcut` text. */
  accelerator?: string;
  /** Default true. */
  enabled?: boolean;
  /** Why it is disabled, for the menu that can say so. */
  reason?: string;
  /** Shows a check mark. */
  checked?: boolean;
  /** The check marks one choice out of several. */
  radio?: boolean;
  /** The host builds this item itself; no other field but `id` is used. */
  predefined?: PredefinedItem;
  /** A divider; no other field is used. */
  separator?: boolean;
  /** A title over the items that follow, in the in-window `Menu` only; no other field but `label` is used. */
  heading?: boolean;
  /** Makes it a submenu. */
  children?: MenuItem[];
}

export const separator: MenuItem = { id: "-", label: "", separator: true };

/** The submenu whose label starts with `key`, as Alt then a letter picks a menu of a menu bar. */
export function mnemonicTarget(items: readonly MenuItem[], key: string): MenuItem | undefined {
  if (key.length !== 1) return undefined;
  return items.find((i) => !!i.children && i.enabled !== false && i.label.toLowerCase().startsWith(key.toLowerCase()));
}
