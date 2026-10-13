// Two taps of Shift anywhere in the window run `fire`, while `enabled` and no modal dialog is open.

import { ShiftTap } from "../../lib/ui/shiftTap.ts";

/** Starts watching the window's keys; the returned function stops it. */
export function watchDoubleShift(enabled: () => boolean, fire: () => void): () => void {
  const tap = new ShiftTap();
  // Capturing, so that no handler that takes a key away from the window hides it from the count.
  const down = (e: KeyboardEvent) => tap.keydown(e);
  const up = (e: KeyboardEvent) => {
    // A modal dialog takes the keys of the window: what it asks for is not Search Everywhere's.
    if (tap.keyup(e, e.timeStamp) && enabled() && document.querySelector(":modal") === null) fire();
  };
  const cancel = () => tap.cancel();
  // Shift with the wheel scrolls sideways; Shift with the pointer extends a selection.
  const listening = [
    ["keydown", down, true],
    ["keyup", up, true],
    ["pointerdown", cancel, true],
    ["wheel", cancel, { capture: true, passive: true }],
    ["blur", cancel, false],
  ] as const;
  for (const [type, listener, options] of listening) window.addEventListener(type, listener as EventListener, options);
  return () => {
    for (const [type, listener, options] of listening)
      window.removeEventListener(type, listener as EventListener, options);
  };
}
