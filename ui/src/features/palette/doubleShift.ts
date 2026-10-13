// Two taps of Shift anywhere in the window run `fire`, while `enabled`.

import { ShiftTap } from "../../lib/ui/shiftTap";

/** Starts watching the window's keys; the returned function stops it. */
export function watchDoubleShift(enabled: () => boolean, fire: () => void): () => void {
  const tap = new ShiftTap();
  // Capturing, so that no handler that takes a key away from the window hides it from the count.
  const down = (e: KeyboardEvent) => tap.keydown(e);
  const up = (e: KeyboardEvent) => {
    if (tap.keyup(e, e.timeStamp) && enabled()) fire();
  };
  const cancel = () => tap.cancel();
  window.addEventListener("keydown", down, true);
  window.addEventListener("keyup", up, true);
  window.addEventListener("pointerdown", cancel, true);
  window.addEventListener("blur", cancel);
  return () => {
    window.removeEventListener("keydown", down, true);
    window.removeEventListener("keyup", up, true);
    window.removeEventListener("pointerdown", cancel, true);
    window.removeEventListener("blur", cancel);
  };
}
