// How a symbol is named where it is shown: in a row of a list, in a tooltip, in the panel's head.
// What the language servers put in a name, and what makes its identity, stay as they are; this is
// display only.

import type { SymbolId } from "../generated/SymbolId";
import type { SymbolName } from "../generated/SymbolName";

/** What separates a container from its member in the language of the file at `path`. */
export const separatorOf = (path: string): string => (/\.(rs|c|h|cc|cpp|cxx|hpp)$/i.test(path) ? "::" : ".");

/** The last part of a path, which may be absolute. */
export const fileName = (path: string): string => path.split(/[\\/]/).pop() ?? path;

/**
 * A name from the outline of the file at `path` as a row shows it. The C# server lists a method with
 * its parameters and return type (`Merge(Dictionary<…>, Task<Line>) : List<Line>`), a field or
 * property with its type (`Id : int`), which are left out.
 */
export function plainName(path: string, name: string): string {
  if (!/\.csx?$/i.test(path)) return name;
  const open = name.indexOf("(");
  if (open > 0) return name.slice(0, open);
  const typed = name.indexOf(" : ");
  return typed > 0 ? name.slice(0, typed) : name;
}

/** The type a Rust `impl` block is for (`impl Backoff for Retry` is `Retry`); any other name as it is. */
const implTarget = (name: string): string =>
  /^impl\b/.test(name) ? name.replace(/^impl(<[^>]*>)?\s+/, "").replace(/^.* for /, "") : name;

/** The symbol's own name, without what a server adds to it. */
export const nameOf = (id: SymbolId): string => plainName(id.path, id.name[id.name.length - 1] ?? id.path);

/** The names of the containers of `id` as a row shows them, outermost first. */
export const containersOf = (id: SymbolId): string[] => id.name.slice(0, -1).map((n) => plainName(id.path, n));

/**
 * A symbol in a row: its name with the container it is in (`Device.Create`, `Retry::new`), else its
 * name alone. The namespaces and modules around it are left out, and the container is its innermost one.
 */
export function shortLabel({ id, modules }: SymbolName): string {
  const last = id.name.length - 1;
  const name = implTarget(nameOf(id));
  // A container that is a module or namespace is where the symbol is, not what it is in.
  if (last < 1 || last - 1 < modules) return name;
  return `${implTarget(plainName(id.path, id.name[last - 1]))}${separatorOf(id.path)}${name}`;
}

/** What a row says of a place: its symbol, else the name of its file. */
export const placeLabel = (symbol: SymbolName | null, path: string): string =>
  symbol ? shortLabel(symbol) : fileName(path);

/** The whole name of a symbol as its language writes it, namespaces included, for a tooltip. */
export const fullName = (id: SymbolId): string => id.name.map((n) => plainName(id.path, n)).join(separatorOf(id.path));
