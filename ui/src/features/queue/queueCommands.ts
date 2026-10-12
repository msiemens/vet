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
      enabled: (c) => c.row !== undefined,
      run: (c) => {
        if (!c.row) return false;
        a.open(c.row.key);
      },
    },
    {
      id: "queue.copyLink",
      label: "Copy Link",
      enabled: (c) => c.row?.url !== undefined,
      disabledReason: () => "Only a pull request has a link",
      run: (c) => {
        if (c.row?.url === undefined) return false;
        a.copyText(c.row.url);
      },
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
