// The commands of the window: what each is called, which key runs it, when it is available and
// what it does. Menus, tooltips and the key handler all read this one table, so a command
// shows the shortcut that runs it and a disabled command is disabled everywhere.
//
// The registry is generic over the context the commands read (`C`), which the application
// defines: `lib/` knows nothing of the domain.

import { type Binding, type KeyLike, type ModKey, bindingOf, isTextInput, matches } from "./keymap.ts";
import { LIST } from "./listKeys.ts";
import { SEARCHING } from "./speedSearch.ts";

export interface Command<C> {
  /** Stable, free of `:`. A menu item or a context menu names the command by it. */
  id: string;
  label: string;
  /** The default key, as `matches` reads it (`Ctrl-Alt-s`); `Mod` is Cmd on macOS and Ctrl elsewhere. */
  binding?: string;
  /** The default key on macOS, where the platform's habit differs from `binding`. */
  macBinding?: string;
  /** Further keys that run it on every platform; a binding the viewer sets replaces them as it does the default. */
  alsoBindings?: string[];
  /** Key the native menu bar shows and handles itself, where the window's own binding is not used. */
  accelerator?: string;
  /** Also runs while a text field has the focus; otherwise its key belongs to the field. */
  inFields?: boolean;
  /** Its key belongs to a list that has the focus (Space selects an item there); the command runs elsewhere. */
  yieldsToLists?: boolean;
  /** Only a menu item carries the argument it needs, so no key runs it. */
  argument?: boolean;
  /** Default: always. `arg` is the argument a menu item carries (`id:arg`). */
  enabled?(context: C, arg?: string): boolean;
  /** Why the command is not enabled in `context`, where a menu can say so. */
  disabledReason?(context: C, arg?: string): string | undefined;
  /** Answers `false` when it found nothing to do: its key then goes on to the page. */
  run(context: C, arg?: string): unknown;
}

/** A keydown as the dispatcher reads it. */
export type KeyEventLike = KeyLike & {
  target?: EventTarget | null;
  preventDefault(): void;
  stopPropagation?(): void;
};

/** A registered command as the Keymap lists it. */
export interface CommandListing {
  id: string;
  label: string;
  /** The key that runs it now. */
  binding?: Binding;
  /** The key it has unless the viewer changed it. */
  defaultBinding?: Binding;
  /** The further keys that run it now; none once the viewer set its binding. */
  alsoBindings: Binding[];
}

/** Lets the registry's bindings be read reactively: `read` runs whenever one is read, `write` when they change. */
export interface BindingSignal {
  read(): void;
  write(): void;
}

/** Whether the key's target is in a text field or a list taking a speed search; the window and the document are not elements. */
const inField = (target: EventTarget | null | undefined) =>
  (target as { closest?: (selector: string) => unknown } | null | undefined)?.closest?.(
    `input, textarea, [${SEARCHING}]`,
  ) != null;

/** Whether the key's target is in a list, which takes some keys for its own items. */
const inList = (target: EventTarget | null | undefined) =>
  (target as { closest?: (selector: string) => unknown } | null | undefined)?.closest?.(`[${LIST}]`) != null;

/** Splits a menu item's id into its command and the argument after the first `:`. */
const splitId = (id: string): [string, string | undefined] => {
  const at = id.indexOf(":");
  return at < 0 ? [id, undefined] : [id.slice(0, at), id.slice(at + 1)];
};

export class CommandRegistry<C extends object> {
  private commands = new Map<string, Command<C>>();
  private sources = new Set<() => Partial<C>>();
  /** Bindings that replace the defaults, by command id; `null` unbinds. */
  private overrides: Record<string, string | null> = {};
  private bound: { binding: Binding; command: Command<C> }[] = [];
  private capturing?: (event: KeyEventLike) => void;

  /** `base` is the context where no source says otherwise. */
  private base: C;
  private mod: ModKey;
  private signal?: BindingSignal;

  constructor(base: C, mod: ModKey, signal?: BindingSignal) {
    this.base = base;
    this.mod = mod;
    this.signal = signal;
  }

  /** Adds the commands; the returned function removes them. */
  register(...added: Command<C>[]): () => void {
    for (const command of added) {
      if (command.id.includes(":")) throw new Error(`command id "${command.id}" contains ":"`);
      if (this.commands.has(command.id)) throw new Error(`command "${command.id}" is registered twice`);
      this.commands.set(command.id, command);
    }
    this.rebind();
    return () => {
      for (const command of added) this.commands.delete(command.id);
      this.rebind();
    };
  }

  /** Adds a source of context facts; the returned function removes it. */
  provide(source: () => Partial<C>): () => void {
    this.sources.add(source);
    return () => void this.sources.delete(source);
  }

  /** The current context: the base, what the sources say, then `extra` (the subject of a context menu). */
  context(extra?: Partial<C>): C {
    const context: C = { ...this.base };
    for (const source of this.sources) Object.assign(context, source());
    return Object.assign(context, extra);
  }

  /** Replaces the default bindings of the given commands; a null spec leaves the command without a key. */
  setBindings(overrides: Record<string, string | null>) {
    this.overrides = overrides;
    this.rebind();
  }

  private rebind() {
    this.bound = [];
    for (const command of this.commands.values()) {
      const overridden = command.id in this.overrides;
      const specs = overridden
        ? [this.overrides[command.id]]
        : [this.defaultSpec(command), ...(command.alsoBindings ?? [])];
      for (const spec of specs) if (spec) this.bound.push({ binding: bindingOf(spec, this.mod), command });
    }
    this.signal?.write();
  }

  private defaultSpec(command: Command<C>): string | undefined {
    return this.mod === "Meta" && command.macBinding !== undefined ? command.macBinding : command.binding;
  }

  find(id: string): Command<C> | undefined {
    return this.commands.get(splitId(id)[0]);
  }

  /** The key that runs the command. */
  binding(id: string): Binding | undefined {
    this.signal?.read();
    return this.bound.find((b) => b.command.id === id)?.binding;
  }

  /** Every command a key can run, with the key it has and the one it has by default. */
  list(): CommandListing[] {
    this.signal?.read();
    return [...this.commands.values()]
      .filter((command) => !command.argument)
      .map((command) => {
        const spec = this.defaultSpec(command);
        return {
          id: command.id,
          label: command.label,
          binding: this.binding(command.id),
          alsoBindings:
            command.id in this.overrides ? [] : (command.alsoBindings ?? []).map((s) => bindingOf(s, this.mod)),
          ...(spec ? { defaultBinding: bindingOf(spec, this.mod) } : {}),
        };
      });
  }

  /**
   * While `handler` is set every key goes to it and runs no command: how a new binding is read by
   * pressing it. The returned function ends that.
   */
  capture(handler: (event: KeyEventLike) => void): () => void {
    this.capturing = handler;
    return () => {
      if (this.capturing === handler) this.capturing = undefined;
    };
  }

  /** `text`, or the command's label, followed by its shortcut in parentheses if it has one. */
  tooltip(id: string, text?: string): string {
    const shortcut = this.binding(id)?.label;
    const label = text ?? this.find(id)?.label ?? "";
    return shortcut ? `${label} (${shortcut})` : label;
  }

  /** The command's label. */
  label(id: string): string {
    return this.find(id)?.label ?? "";
  }

  /** How a key spec (`Mod-Enter`) reads to the viewer on this platform, as a menu shows a binding. */
  keyLabel(spec: string): string {
    return bindingOf(spec, this.mod).label;
  }

  /**
   * The tooltip of a control that does the command's job: why the command cannot run now, if it
   * cannot, else `tooltip`.
   */
  hint(id: string, text?: string, context: C = this.context()): string {
    return this.disabledReason(id, context) ?? this.tooltip(id, text);
  }

  isEnabled(id: string, context: C = this.context()): boolean {
    const [, arg] = splitId(id);
    const command = this.find(id);
    return !!command && (command.enabled?.(context, arg) ?? true);
  }

  /** Why the command is not enabled in `context`; none when it is, or when the command does not say. */
  disabledReason(id: string, context: C = this.context()): string | undefined {
    const [, arg] = splitId(id);
    const command = this.find(id);
    return command && !this.isEnabled(id, context) ? command.disabledReason?.(context, arg) : undefined;
  }

  /** Runs the command named by a menu item's id (`id` or `id:arg`); answers whether it ran. */
  run(id: string, context: C = this.context()): boolean {
    const [, arg] = splitId(id);
    const command = this.find(id);
    if (!command || !this.isEnabled(id, context)) return false;
    return this.execute(command, context, arg);
  }

  private execute(command: Command<C>, context: C, arg?: string): boolean {
    const result = command.run(context, arg);
    if (result instanceof Promise) result.catch((e: unknown) => console.error(e));
    return result !== false;
  }

  /**
   * Runs the command bound to the key, if one is enabled in the current context, and takes the
   * event. Text input and a text field's own keys never reach a command. Only a key that matches a
   * binding costs more than the match.
   */
  dispatch(event: KeyEventLike): boolean {
    if (this.capturing) {
      this.capturing(event);
      event.preventDefault();
      event.stopPropagation?.();
      return true;
    }
    let field: boolean | undefined;
    const typing = () => (field ??= inField(event.target));
    // Option types characters on macOS, but nowhere but in a text field.
    const optionChord = () => event.altKey && !typing();
    if (isTextInput(event, optionChord())) return false;
    let context: C | undefined;
    for (const { binding, command } of this.bound) {
      if (!matches(binding.spec, event, optionChord())) continue;
      if (!command.inFields && typing()) continue;
      if (command.yieldsToLists && inList(event.target)) continue;
      context ??= this.context();
      if (command.enabled && !command.enabled(context)) continue;
      if (!this.execute(command, context)) continue;
      event.preventDefault();
      return true;
    }
    return false;
  }
}
