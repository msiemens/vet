// The tool windows and the screens of the shell, as data: what each rail holds, and the screen the
// window shows. `App` composes them; the rails and the commands act on them through `shell`.

import { TOOL_WINDOWS } from "../../state/appCommands";

/** Which screen shows: the queue of changesets, or the review of the one on view. */
export type Screen = "queue" | "review";

/** The tool windows of each rail, in the order the rail lists them. */
export const TOOLS = {
  left: TOOL_WINDOWS.filter((t) => t.side === "left"),
  bottom: TOOL_WINDOWS.filter((t) => t.side === "bottom"),
  right: TOOL_WINDOWS.filter((t) => t.side === "right"),
};
