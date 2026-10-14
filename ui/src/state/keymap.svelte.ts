// The viewer's key bindings: the changes to the commands' defaults, kept as a preference like the
// theme and applied to the command registry, whose menus and tooltips follow at once.

import { type Overrides, parseOverrides, withBinding, withReplacedBinding, withoutOverride } from "../lib/keymapEdit";
import { Preference } from "../lib/preferences.svelte";
import { commands } from "./commands";

class Keymap {
  private kept = new Preference<Overrides>("app", "keymap", parseOverrides, {});
  overrides = $state.raw<Overrides>({});

  /** Applies the kept bindings, now and whenever they change. */
  load() {
    this.apply();
    this.kept.watch(() => this.apply());
  }

  /** Binds `spec` to the command, taking it from the commands that have it. */
  bind(id: string, spec: string) {
    this.kept.value = withReplacedBinding(this.overrides, commands.list(), id, spec);
  }

  /** Leaves the command without a key. */
  unbind(id: string) {
    const command = commands.list().find((c) => c.id === id);
    if (command) this.kept.value = withBinding(this.overrides, command, null);
  }

  /** Gives the command its default key again. */
  reset(id: string) {
    this.kept.value = withoutOverride(this.overrides, id);
  }

  resetAll() {
    this.kept.value = {};
  }

  private apply() {
    this.overrides = this.kept.value;
    commands.setBindings(this.overrides);
  }
}

export const keymap = new Keymap();
