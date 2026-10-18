// The pages Settings lists, and the page a command opens it on. A Backends page is not listed:
// SPEC names no setting of its own for it.

export const APPLICATION = [
  { id: "general", label: "General" },
  { id: "appearance", label: "Appearance" },
  { id: "keymap", label: "Keymap" },
  { id: "servers", label: "Language servers" },
  { id: "storage", label: "Storage" },
  { id: "accounts", label: "Accounts" },
] as const;

export const PROJECT = [
  { id: "trust", label: "Trust" },
  { id: "filter", label: "Review filter" },
  { id: "projectStorage", label: "Storage" },
] as const;

/** The page of the settings dialog. */
export type Page = (typeof APPLICATION)[number]["id"] | (typeof PROJECT)[number]["id"];
