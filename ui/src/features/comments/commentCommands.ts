// The commands on comment threads and the descriptions of their context menus.

import type { Thread } from "../../generated/Thread";
import type { MenuSpec } from "../../lib/commandMenu.ts";
import type { Command } from "../../lib/commands.ts";
import type { CommandContext } from "../../state/commands.ts";
import { candidateLabel, candidates } from "./groups.ts";

export interface ThreadActions {
  /** Re-anchors `thread` at its `candidate`th candidate. */
  attach(thread: number, candidate: number): void;
  /** Deletes the thread with its comments. */
  remove(thread: number): void;
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
    {
      id: "thread.delete",
      label: "Delete Thread",
      subject: true,
      enabled: (c) => c.thread !== undefined,
      run: (c) => {
        if (!c.thread) return false;
        a.remove(c.thread.id);
      },
    },
  ];
}

/** The menu that attaches a detached thread at one of its candidates. */
export const attachMenu = (thread: Thread): MenuSpec =>
  candidates(thread).map((at, i) => ({ command: "thread.attach", arg: String(i), label: candidateLabel(at) }));

/** The menu of a thread's less common actions. */
export const threadMenu: MenuSpec = [{ command: "thread.delete" }];
