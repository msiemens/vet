// Context menus: one request, shown by the host when there is one and drawn by the page otherwise.

import { type MenuItem, separator } from "./ui/menuItem.ts";

export interface ContextMenuRequest {
  /** Viewport position of the pointer. */
  x: number;
  y: number;
  items: MenuItem[];
  /** Called with the id of the chosen leaf item. */
  onselect(id: string): void;
}

/**
 * The items as the host draws them in a context menu: the shortcut stays display only (the host
 * shows it the way the OS does; the window's key handler runs it), a context menu registers no
 * accelerators, and stray separators and empty submenus are dropped.
 */
export function nativeItems(items: MenuItem[]): MenuItem[] {
  const out: MenuItem[] = [];
  for (const item of items) {
    if (item.separator) {
      if (out.length && !out[out.length - 1].separator) out.push(separator);
      continue;
    }
    const rest: MenuItem = { ...item };
    delete rest.accelerator;
    if (item.children === undefined) {
      out.push(rest);
      continue;
    }
    const children = nativeItems(item.children);
    if (children.length) out.push({ ...rest, children });
  }
  while (out.at(-1)?.separator) out.pop();
  return out;
}

/** Pops up a menu of the host; resolves with the chosen id, or `null` if none was. */
export type NativePopup = (items: MenuItem[], x: number, y: number) => Promise<string | null>;

/**
 * Shows the menu through `native` if there is one (and it works), else hands it to `draw`. A choice made in the
 * host's menu runs after `restoreFocus`, so it acts where the focus was; the page's own menu does
 * that itself.
 */
export async function openContextMenu(
  request: ContextMenuRequest,
  native: NativePopup | undefined,
  draw: (request: ContextMenuRequest) => void,
  restoreFocus: () => void,
): Promise<void> {
  if (!native) {
    draw(request);
    return;
  }
  const items = nativeItems(request.items);
  if (!items.length) return;
  let id: string | null;
  try {
    id = await native(items, request.x, request.y);
  } catch (e) {
    console.error(e);
    draw(request);
    return;
  }
  restoreFocus();
  if (id !== null) request.onselect(id);
}
