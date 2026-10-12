// Menus as data: a description of which commands a menu offers, in what order and structure.
// `resolveMenu` fills in what the registry knows (label, shortcut, enabled) and returns the
// `MenuItem`s that the in-window `Menu` draws and the host draws natively.

import type { CommandRegistry } from "./commands.ts";
import { type MenuItem, type PredefinedItem, separator } from "./ui/menuItem.ts";

export type MenuSpec = MenuSpecEntry[];

export type MenuSpecEntry =
  | "-"
  | {
      command: string;
      /** The item's id is `command:arg`; the command's `run` and `enabled` get the argument. */
      arg?: string;
      /** Replaces the command's label. */
      label?: string;
      detail?: string;
      checked?: boolean;
      /** The check marks one choice out of several. */
      radio?: boolean;
      /** Disables the item whatever the command says, for this reason. */
      disabled?: string;
    }
  | {
      /** An item the host builds from the OS's definition. */
      predefined: PredefinedItem;
    }
  | {
      /** A submenu; it is disabled when nothing in it is enabled (including when it is empty). */
      id: string;
      label: string;
      items: MenuSpec;
    };

/**
 * The items of `spec` for `context`. A command that is not registered is left out, and so are
 * the separators that would then lead, trail or double up.
 */
export function resolveMenu<C extends object>(
  registry: CommandRegistry<C>,
  spec: MenuSpec,
  context: C,
  /** The host draws the menu bar: an item with a native accelerator carries it instead of the window's binding as text. */
  native = false,
): MenuItem[] {
  const items: MenuItem[] = [];
  for (const entry of spec) {
    if (entry === "-") {
      if (items.length && !items[items.length - 1].separator) items.push(separator);
    } else if ("predefined" in entry) {
      items.push({ id: `predefined.${entry.predefined}`, label: "", predefined: entry.predefined });
    } else if ("items" in entry) {
      const children = resolveMenu(registry, entry.items, context, native);
      const enabled = children.some((c) => !c.separator && c.enabled !== false);
      items.push({ id: entry.id, label: entry.label, ...(enabled ? {} : { enabled: false }), children });
    } else {
      const command = registry.find(entry.command);
      if (!command) continue;
      const id = entry.arg === undefined ? command.id : `${command.id}:${entry.arg}`;
      const enabled = entry.disabled === undefined && registry.isEnabled(id, context);
      const reason = enabled ? undefined : (entry.disabled ?? registry.disabledReason(id, context));
      const accelerator = native ? command.accelerator : undefined;
      const binding = accelerator === undefined ? registry.binding(command.id) : undefined;
      items.push({
        id,
        label: entry.label ?? command.label,
        ...(entry.detail === undefined ? {} : { detail: entry.detail }),
        ...(binding === undefined ? {} : { shortcut: binding.label, chord: binding.chord }),
        ...(accelerator === undefined ? {} : { accelerator }),
        ...(enabled ? {} : { enabled: false }),
        ...(reason === undefined ? {} : { reason }),
        ...(entry.checked === undefined ? {} : { checked: entry.checked }),
        ...(entry.radio ? { radio: true } : {}),
      });
    }
  }
  while (items.at(-1)?.separator) items.pop();
  return items;
}
