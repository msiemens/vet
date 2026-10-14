// The window's own preferences: how it looks and how the review views lay lines out.

import { choicePreference } from "../lib/preferences.svelte";
import { SOFT_WRAPS } from "../lib/softWrap";
import { THEMES } from "../lib/theme/theme";

export const themePreference = choicePreference(
  "app",
  "theme",
  THEMES.map((t) => t.id),
  "system",
);

export const softWrapPreference = choicePreference(
  "app",
  "softWrap",
  SOFT_WRAPS.map((w) => w.id),
  "prose",
);
