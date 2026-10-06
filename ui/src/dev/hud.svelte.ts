// Live latencies shown in the dev bar.

import type { Hud } from "../features/review";

export const hud = $state({ keystroke: 0, hover: 0, collisions: 0 });

export const hudListeners: Hud = {
  keystroke: (ms) => (hud.keystroke = ms),
  hover: (ms) => (hud.hover = ms),
  collision: (n) => (hud.collisions = n),
};
