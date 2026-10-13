import type { FileStatus } from "../generated/FileStatus";
import type { SymbolChange } from "../generated/SymbolChange";

/** The letter that stands for how a file or a symbol changed. */
export type StatusLetter = "M" | "A" | "D" | "R";

const OF_STATUS = { modified: "M", added: "A", deleted: "D", renamed: "R" } as const satisfies Record<
  FileStatus,
  StatusLetter
>;

/** A removed symbol is marked as a deleted file is. */
const OF_CHANGE = { modified: "M", added: "A", removed: "D" } as const satisfies Record<SymbolChange, StatusLetter>;

export const letterOfStatus = (status: FileStatus): StatusLetter => OF_STATUS[status];

export const letterOfChange = (change: SymbolChange): StatusLetter => OF_CHANGE[change];
