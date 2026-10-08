import type { Thread } from "../generated/Thread";

/** A comment on the pull request in the platform's flat conversation: it has no status and takes no replies. */
export const isFlat = (thread: Thread) => thread.kind === "changeset" && thread.platform?.resolvable === false;
