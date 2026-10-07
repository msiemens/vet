// Makes the command registry's bindings reactive: what reads one (a tooltip, a menu built in an
// effect) is drawn again when a binding changes.

import type { BindingSignal } from "../lib/commands";

let version = $state(0);

export const bindingSignal: BindingSignal = {
  read() {
    return version;
  },
  write() {
    version++;
  },
};
