// How a symbol is named in a row of a list: its own name, and the innermost container around it.

import type { SymbolId } from "../generated/SymbolId";

/** The symbol's own name. */
export const nameOf = (id: SymbolId): string => id.name[id.name.length - 1] ?? id.path;

/** What separates a container from its member in the language of the file at `path`. */
export const separatorOf = (path: string): string => (/\.(rs|c|h|cc|cpp|cxx|hpp)$/i.test(path) ? "::" : ".");

/** The names of `id` up to its `length`th, joined as its language writes them (`A::B` in Rust, `A.B` in TypeScript). */
export const qualified = (id: SymbolId, length: number = id.name.length): string =>
  id.name.slice(0, length).join(separatorOf(id.path));

/** The last part of a path, which may be absolute. */
const fileName = (path: string): string => path.split(/[\\/]/).pop() ?? path;

/**
 * A name from the outline of the file at `path` as a row shows it. The C# server lists a method with
 * its parameters and return type (`Merge(Dictionary<…>, Task<Line>) : List<Line>`), which are left out.
 */
function plainName(path: string, name: string): string {
  if (!/\.csx?$/i.test(path)) return name;
  const open = name.indexOf("(");
  return open > 0 ? name.slice(0, open) : name;
}

/**
 * What a row of a symbol list says about a symbol: its name, and where it is, which is its innermost
 * container, else the name of its file. A place without a symbol (`id` absent) is its file.
 */
export function rowLabel(id: SymbolId | null, path: string): { name: string; context: string } {
  if (!id) return { name: fileName(path), context: "" };
  const container = id.name.length > 1 ? id.name[id.name.length - 2] : undefined;
  return {
    name: plainName(id.path, nameOf(id)),
    context: container === undefined ? fileName(id.path) : plainName(id.path, container),
  };
}
