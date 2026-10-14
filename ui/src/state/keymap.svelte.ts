// The viewer's key bindings: the changes to the commands' defaults, kept in the browser storage
// like the theme and applied to the command registry, whose menus and tooltips follow at once.

import {
  type Overrides,
  parseOverrides,
  serializeOverrides,
  withBinding,
  withReplacedBinding,
  withoutOverride,
} from "../lib/keymapEdit";
import { readJson, writeStored } from "../lib/storage";
import { commands } from "./commands";

const KEY = "vet.keymap";

class Keymap {
  overrides = $state.raw<Overrides>({});

  /** Applies the kept bindings. */
  load() {
    this.apply(readJson(KEY, parseOverrides) ?? {});
  }

  /** Binds `spec` to the command, taking it from the commands that have it. */
  bind(id: string, spec: string) {
    this.apply(withReplacedBinding(this.overrides, commands.list(), id, spec));
  }

  /** Leaves the command without a key. */
  unbind(id: string) {
    const command = commands.list().find((c) => c.id === id);
    if (command) this.apply(withBinding(this.overrides, command, null));
  }

  /** Gives the command its default key again. */
  reset(id: string) {
    this.apply(withoutOverride(this.overrides, id));
  }

  resetAll() {
    this.apply({});
  }

  private apply(overrides: Overrides) {
    this.overrides = overrides;
    commands.setBindings(overrides);
    writeStored(KEY, serializeOverrides(overrides));
  }
}

export const keymap = new Keymap();
