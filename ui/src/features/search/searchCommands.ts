// The command that opens Find in Files.

import type { Command } from "../../lib/commands.ts";
import type { CommandContext } from "../../state/commands.ts";
import { openFindInFiles } from "./FindInFiles.svelte.ts";

/** A selection that fits on one line is what the search starts with. */
export const oneLine = (text: string | undefined): string | undefined =>
  text && !/[\r\n]/.test(text) ? text : undefined;

/**
 * `windowQuery` is what the window's find bar looks for. The search starts with the one-line selection of the
 * focused editor, else with that, else with what it looked for last.
 */
export function searchCommands(windowQuery: () => string | undefined): Command<CommandContext>[] {
  return [
    {
      id: "find.inFiles",
      label: "Find in Files…",
      binding: "Mod-Shift-f",
      inFields: true,
      enabled: (c) => !!c.changesetOpen,
      disabledReason: () => "Open a changeset first",
      run: (c) => openFindInFiles({ query: oneLine(c.editor?.selectedText()) ?? oneLine(windowQuery()) }),
    },
  ];
}
