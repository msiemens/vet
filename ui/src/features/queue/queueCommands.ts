// The commands on the queue's rows and the description of a row's context menu.

import type { ChangesetKey } from "../../generated/ChangesetKey";
import type { MenuSpec } from "../../lib/commandMenu.ts";
import type { Command } from "../../lib/commands.ts";
import type { CommandContext } from "../../state/commands.ts";

export interface QueueActions {
  /** Puts the changeset on view and shows its review. */
  open(key: ChangesetKey): void;
  copyText(text: string): void;
}

export function queueCommands(a: QueueActions): Command<CommandContext>[] {
  return [
    {
      id: "queue.open",
      label: "Open",
      subject: true,
      enabled: (c) => c.row !== undefined,
      run: (c) => {
        if (!c.row) return false;
        a.open(c.row.key);
      },
    },
    {
      id: "queue.copyLink",
      label: "Copy Link",
      subject: true,
      enabled: (c) => c.row?.url !== undefined,
      disabledReason: () => "Only a pull request has a link",
      run: (c) => {
        if (c.row?.url === undefined) return false;
        a.copyText(c.row.url);
      },
    },
  ];
}

/** The commands on the tab row of the queue screen, which exist while it is mounted. */
export function queueTabCommands(a: { stepTab(delta: -1 | 1): void }): Command<CommandContext>[] {
  const shown = (c: CommandContext) => !!c.queueShown && !c.paletteOpen;
  return [
    {
      id: "queue.previousTab",
      label: "Previous Queue Tab",
      binding: "Ctrl-Shift-Tab",
      inFields: true,
      enabled: shown,
      run: () => a.stepTab(-1),
    },
    {
      id: "queue.nextTab",
      label: "Next Queue Tab",
      binding: "Ctrl-Tab",
      inFields: true,
      enabled: shown,
      run: () => a.stepTab(1),
    },
  ];
}

/** The context menu of a row in the queue. */
export const rowMenu: MenuSpec = [
  { command: "queue.open" },
  { command: "changeset.openInBrowser" },
  "-",
  { command: "queue.copyLink" },
];
