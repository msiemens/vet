// Editing the keymap: which bindings the viewer changed, who a new binding would clash with, and
// how the changes are kept. Bindings are specs as `matches` reads them.

import type { CommandListing } from "./commands.ts";
import { canonicalSpec } from "./keymap.ts";

/** The bindings that differ from the defaults, by command id; `null` is a command left without a key. */
export type Overrides = Record<string, string | null>;

const sameKey = (a: string | undefined, b: string | undefined) =>
  a !== undefined && b !== undefined && canonicalSpec(a) === canonicalSpec(b);

/** The other commands that a key already runs. */
export function conflictsOf(listing: CommandListing[], id: string, spec: string): CommandListing[] {
  return listing.filter((c) => c.id !== id && sameKey(c.binding?.spec, spec));
}

/** `overrides` without the command's: it has its default again. */
export function withoutOverride(overrides: Overrides, id: string): Overrides {
  return Object.fromEntries(Object.entries(overrides).filter(([other]) => other !== id));
}

/** `overrides` with the command bound to `spec`, or unbound for `null`; a binding equal to the default is no override. */
export function withBinding(overrides: Overrides, command: CommandListing, spec: string | null): Overrides {
  const rest = withoutOverride(overrides, command.id);
  if (spec === null) return command.defaultBinding === undefined ? rest : { ...rest, [command.id]: null };
  return sameKey(spec, command.defaultBinding?.spec) ? rest : { ...rest, [command.id]: spec };
}

/** Binds `spec` to the command and takes it from the commands that had it. */
export function withReplacedBinding(overrides: Overrides, listing: CommandListing[], id: string, spec: string) {
  let next = overrides;
  for (const other of conflictsOf(listing, id, spec)) next = withBinding(next, other, null);
  const command = listing.find((c) => c.id === id);
  return command ? withBinding(next, command, spec) : next;
}

/** The overrides as they are kept. */
export const serializeOverrides = (overrides: Overrides): string | undefined =>
  Object.keys(overrides).length ? JSON.stringify(overrides) : undefined;

/** The overrides kept in `text`; whatever is not a binding is dropped, so a damaged value costs only itself. */
export function parseOverrides(text: string | undefined): Overrides {
  if (text === undefined) return {};
  let value: unknown;
  try {
    value = JSON.parse(text);
  } catch {
    return {};
  }
  if (typeof value !== "object" || value === null || Array.isArray(value)) return {};
  const overrides: Overrides = {};
  for (const [id, spec] of Object.entries(value as Record<string, unknown>)) {
    if (spec === null || (typeof spec === "string" && spec !== "")) overrides[id] = spec;
  }
  return overrides;
}
