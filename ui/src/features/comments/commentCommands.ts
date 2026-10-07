// The comments panel's commands and the description of its context menu.

import type { Thread } from "../../generated/Thread";
import type { MenuSpec } from "../../lib/commandMenu.ts";
import type { Command } from "../../lib/commands.ts";
import type { CommandContext } from "../../state/commands.ts";
import { candidateLabel, candidates } from "./groups.ts";

export interface ThreadActions {
  /** Re-anchors `thread` at its `candidate`th candidate. */
  attach(thread: number, candidate: number): void;
}

export function threadCommands(a: ThreadActions): Command<CommandContext>[] {
  const candidateOf = (c: CommandContext, arg?: string) => {
    const at = arg === undefined ? NaN : Number(arg);
    return c.thread && Number.isInteger(at) && at >= 0 && at < candidates(c.thread).length ? at : undefined;
  };
  return [
    {
      id: "thread.attach",
      label: "Attach",
      argument: true,
      enabled: (c, arg) => candidateOf(c, arg) !== undefined,
      run: (c, arg) => {
        const at = candidateOf(c, arg);
        if (!c.thread || at === undefined) return false;
        a.attach(c.thread.id, at);
      },
    },
  ];
}

/** The menu that attaches a detached thread at one of its candidates. */
export const attachMenu = (thread: Thread): MenuSpec =>
  candidates(thread).map((at, i) => ({ command: "thread.attach", arg: String(i), label: candidateLabel(at) }));
