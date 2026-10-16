// What the window remembers of how the viewer left it (SPEC §9): theme, key bindings, open tabs,
// panel sizes, folded sections. The backend keeps the values; this holds the copy the components
// read, reactively, and hands the viewer's changes on to the backend through `sink`.

import { SvelteMap, SvelteSet } from "svelte/reactivity";
import { decode, encode } from "./preferenceText";

/** `app` values belong to the whole app, `project` values to the project on view. */
export type Scope = "app" | "project";

/** Where the viewer's changes go; `undefined` forgets the key. */
type Sink = (scope: Scope, key: string, value: string | undefined) => void;

class Preferences {
  private app = new SvelteMap<string, string>();
  private project = new SvelteMap<string, string>();
  private watchers = new Map<string, SvelteSet<() => void>>();
  sink: Sink | undefined;

  read(scope: Scope, key: string): string | undefined {
    return this.values(scope).get(key);
  }

  /** The viewer's change: kept here at once and handed to the backend. */
  write(scope: Scope, key: string, value: string | undefined) {
    this.keep(scope, key, value);
    this.sink?.(scope, key, value);
  }

  /** What the backend says is the value now, as announced by an event. */
  keep(scope: Scope, key: string, value: string | undefined) {
    const values = this.values(scope);
    if (values.get(key) === value) return;
    if (value === undefined) values.delete(key);
    else values.set(key, value);
    this.notify(scope, [key]);
  }

  /** All the backend keeps in `scope`, as loaded. */
  replace(scope: Scope, entries: Iterable<readonly [string, string]>) {
    const values = this.values(scope);
    const next = new Map(entries);
    const changed = [...new Set([...values.keys(), ...next.keys()])].filter((key) => values.get(key) !== next.get(key));
    values.clear();
    for (const [key, value] of next) values.set(key, value);
    this.notify(scope, changed);
  }

  /** Calls `listener` whenever the value under `key` changes, by the viewer or by the backend. */
  watch(scope: Scope, key: string, listener: () => void): () => void {
    const id = `${scope}:${key}`;
    const set = this.watchers.get(id) ?? new SvelteSet();
    set.add(listener);
    this.watchers.set(id, set);
    return () => set.delete(listener);
  }

  private values(scope: Scope) {
    return scope === "app" ? this.app : this.project;
  }

  private notify(scope: Scope, keys: string[]) {
    for (const key of keys) for (const listener of this.watchers.get(`${scope}:${key}`) ?? []) listener();
  }
}

export const preferences = new Preferences();

/**
 * One preference. `parse` turns the JSON kept under the key into a value, or into `undefined` for
 * one that is not a value of this preference; the key then has the `fallback`, which is not kept
 * when it is set, so that it follows a later change of the default.
 */
export class Preference<T> {
  constructor(
    private readonly scope: Scope,
    private readonly key: string,
    private readonly parse: (json: unknown) => T | undefined,
    private readonly fallback: T,
  ) {}

  get value(): T {
    return decode(preferences.read(this.scope, this.key), this.parse, this.fallback);
  }

  set value(value: T) {
    preferences.write(this.scope, this.key, encode(value, this.fallback));
  }

  watch(listener: () => void): () => void {
    return preferences.watch(this.scope, this.key, listener);
  }
}

/** An on/off preference. */
export const flagPreference = (scope: Scope, key: string, fallback: boolean) =>
  new Preference(scope, key, (json) => (typeof json === "boolean" ? json : undefined), fallback);

/** A preference that is one of `choices`. */
export const choicePreference = <T extends string>(scope: Scope, key: string, choices: readonly T[], fallback: T) =>
  new Preference(scope, key, (json) => choices.find((choice) => choice === json), fallback);

/** A preference that is a number. */
export const numberPreference = (scope: Scope, key: string) =>
  new Preference<number | undefined>(
    scope,
    key,
    (json) => (typeof json === "number" && Number.isFinite(json) ? json : undefined),
    undefined,
  );
