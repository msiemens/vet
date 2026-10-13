// The commands that open Search Everywhere on one of its tabs, and the one its last row hands over to.

import type { Command } from "../../lib/commands.ts";
import type { CommandContext } from "../../state/commands.ts";
import type { PaletteTab } from "./paletteRows.ts";

/** The commands that open the popup, in the order the Navigate menu lists them. */
export const PALETTE_COMMANDS = ["palette.all", "palette.files", "palette.symbols", "palette.actions"] as const;

export const FIND_IN_FILES = "find.inFiles";

export function paletteCommands(a: { show(tab: PaletteTab): void }): Command<CommandContext>[] {
  const open = (id: (typeof PALETTE_COMMANDS)[number], label: string, tab: PaletteTab, binding?: string) => ({
    id,
    label,
    binding,
    // The popup's own field has the focus when a chord moves it to another tab.
    inFields: true,
    enabled: (c: CommandContext) => c.repoOpen,
    disabledReason: () => "Open a project first",
    run: () => a.show(tab),
  });
  return [
    // Double Shift, which the window watches for, opens All too.
    open("palette.all", "Search Everywhere", "all"),
    open("palette.files", "Go to File…", "file", "Mod-p"),
    open("palette.symbols", "Go to Symbol…", "symbol", "Mod-Alt-o"),
    open("palette.actions", "Find Action…", "action", "Mod-Shift-a"),
    {
      id: FIND_IN_FILES,
      label: "Find in Files…",
      binding: "Mod-Shift-f",
      enabled: () => false,
      disabledReason: () => "Find in Files is not built yet",
      run: () => false,
    },
  ];
}
