import type { SymbolId } from "../generated/SymbolId";

/** A stable key for maps and each blocks: the file and container chain identify a symbol (SPEC §6.3). */
export const keyOf = (id: SymbolId): string => JSON.stringify([id.path, ...id.name]);
