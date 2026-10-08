// What the dev tools measure in the review screen's editors: live latencies for the HUD in the
// dev bar, and the input-correctness check.

import type { Probe } from "../features/review";
import { nextPaint } from "../lib/nextPaint";
import { matches } from "../lib/keymap";

/** Live latencies shown in the dev bar. */
export const hud = $state({ keystroke: 0, hover: 0, collisions: 0 });

/** Bound only to count collisions in the manual input check: AltGr types these keys on a German layout. */
const BAIT = ["Ctrl-Alt-q", "Ctrl-Alt-e", "Ctrl-Alt-7", "Ctrl-Alt-8", "Ctrl-Alt-9", "Ctrl-Alt-0", "Ctrl-Alt-m"];

/** Keys that are only a modifier going down: the key that follows is the one worth timing. */
const MODIFIERS = new Set(["Shift", "Control", "Alt", "AltGraph", "Meta"]);

export const probe: Probe = {
  hooks: (hooks) => ({
    ...hooks,
    hover: async (...args) => {
      const t0 = performance.now();
      const hover = await hooks.hover(...args);
      hud.hover = performance.now() - t0;
      return hover;
    },
    keydown: (event) => {
      if (MODIFIERS.has(event.key)) return;
      void nextPaint().then(() => (hud.keystroke = performance.now() - event.timeStamp));
    },
  }),
  key: (event) => {
    if (!BAIT.some((spec) => matches(spec, event))) return false;
    hud.collisions++;
    return true;
  },
};
