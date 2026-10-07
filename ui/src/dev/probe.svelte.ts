// What the dev tools measure in the review screen's editors: live latencies for the HUD in the
// dev bar, the tooltip latencies the benchmark collects, and the input-correctness check.

import type { Probe, ViewProbe } from "../features/review";
import { nextPaint } from "../lib/nextPaint";
import { matches } from "../lib/keymap";

/** Live latencies shown in the dev bar. */
export const hud = $state({ keystroke: 0, hover: 0, collisions: 0 });

/** Painted hover tooltips since the benchmark cleared this, in ms from the query. */
export const tooltipLatencies: number[] = [];

/** Bound only to count collisions in the manual input check: AltGr types these keys on a German layout. */
const BAIT = ["Ctrl-Alt-q", "Ctrl-Alt-e", "Ctrl-Alt-7", "Ctrl-Alt-8", "Ctrl-Alt-9", "Ctrl-Alt-0", "Ctrl-Alt-m"];

/** The latest seen toggle, so the benchmark can wait for its repaint. */
export const toggling = { pending: Promise.resolve() };

/** The probe of each tab's current view, by tab. */
export const views: Partial<Record<string, ViewProbe>> = {};

export const probe: Probe = {
  listeners: (listeners) => {
    const pending = (request: Promise<void>) => (toggling.pending = request);
    return {
      ...listeners,
      toggleHunk: (revision, path, hunk) => pending(listeners.toggleHunk(revision, path, hunk)),
      toggleItem: (revision, path, kind) => pending(listeners.toggleItem(revision, path, kind)),
      toggleFileSeen: (revision, path) => pending(listeners.toggleFileSeen(revision, path)),
    };
  },
  view: (tab, view) => {
    views[tab] = view;
  },
  hooks: (hooks) => ({
    ...hooks,
    hover: async (...args) => {
      const t0 = performance.now();
      const hover = await hooks.hover(...args);
      hud.hover = performance.now() - t0;
      return hover;
    },
    hoverShown: (ms) => void tooltipLatencies.push(ms),
    keydown: (event) => {
      if (event.key.length !== 1 && event.key !== "Backspace" && event.key !== "Enter") return;
      void nextPaint().then(() => (hud.keystroke = performance.now() - event.timeStamp));
    },
  }),
  key: (event) => {
    if (!BAIT.some((spec) => matches(spec, event))) return false;
    hud.collisions++;
    return true;
  },
};
