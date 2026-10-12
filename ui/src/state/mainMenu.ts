// The main menu, defined once as a description of which commands it offers (see `lib/commandMenu`).
// Labels, shortcuts and enabled state come from the command registry. The host draws the result
// as the native menu bar (macOS), the title bar's ☰ button shows it elsewhere. Both report a chosen
// item by its id, which `commands.run` executes.

import type { Verdict } from "../generated/Verdict.ts";
import type { MenuSpec } from "../lib/commandMenu.ts";
import { THEMES, type Theme } from "../lib/theme/theme.ts";
import { type ViewMode, modeCommand } from "../lib/viewMode.ts";
import { toolCommand } from "./appCommands.ts";

const MODE_ITEMS: { mode: ViewMode; label: string }[] = [
  { mode: "unified", label: "Unified" },
  { mode: "split", label: "Side-by-Side" },
  { mode: "file", label: "File" },
];

/** What the menu shows that is not a command's own: the lists it offers and what is checked in them. */
export interface MenuState {
  /** The host draws the macOS menu bar, with its app menu and its own key accelerators. */
  native: boolean;
  /** Backend-side paths with their display names, most recent first. */
  recent: { path: string; name: string }[];
  /** Which screen shows. */
  screen: "queue" | "review";
  /** What the review's views show: the active tab's file text, else the diff in its mode. */
  viewMode: ViewMode;
  /** Long lines wrap. */
  softWrap: boolean;
  theme: Theme;
  /** The tool windows by id, and which are shown. */
  tools: { id: string; shown: boolean }[];
  /** The review view shows a range of versions. */
  range: boolean;
  /** The host can start the backend again. */
  canRestart: boolean;
  /** The platform of the pull request last on view reviews in one batch, so posting submits the review. */
  batchedReview: boolean;
  /** The votes of a platform that keeps the vote apart from the comments, and the one the viewer has: staged, else cast. */
  votes?: { verdicts: Verdict[]; chosen?: string };
}

/** Also offered by the project chip's menu. */
export const openItem: MenuSpec = [{ command: "file.open" }];

export function mainMenuSpec(s: MenuState): MenuSpec {
  const settings = { command: "app.settings" };
  const about = { command: "app.about" };
  const quit = { command: "app.quit", label: s.native ? "Quit Vet" : undefined };
  // The screens that show hunks and tabs; the queue covers them.
  const inReview = s.screen === "review" ? undefined : "Open the Review screen first";

  const file: MenuSpec = [
    ...openItem,
    {
      id: "recent",
      label: "Open Recent",
      items: s.recent.map(({ path, name }) => ({ command: "file.openRecent", arg: path, label: name, detail: path })),
    },
    "-",
    { command: "tab.close", disabled: inReview },
    { command: "tab.closeOthers", disabled: inReview },
    ...(s.native ? [] : (["-", settings, "-", quit] as MenuSpec)),
  ];
  const view: MenuSpec = [
    { command: "view.queue", checked: s.screen === "queue", radio: true },
    { command: "view.review", checked: s.screen === "review", radio: true },
    "-",
    {
      id: "tools",
      label: "Tool Windows",
      items: [
        ...["changes", "all", "commits", "comments"].map((id) => toolItem(s, id)),
        "-",
        toolItem(s, "symbol"),
        "-",
        toolItem(s, "timeline"),
      ],
    },
    "-",
    { command: "tab.previous", disabled: inReview },
    { command: "tab.next", disabled: inReview },
    "-",
    ...MODE_ITEMS.map(({ mode, label }) => ({
      command: modeCommand(mode),
      label,
      checked: s.viewMode === mode,
      radio: true as const,
      disabled: mode === "file" ? inReview : undefined,
    })),
    "-",
    { command: "view.softWrap", checked: s.softWrap },
    "-",
    {
      id: "theme",
      label: "Theme",
      items: THEMES.map(({ id, label }) => ({
        command: "view.theme",
        arg: id,
        label,
        checked: s.theme === id,
        radio: true,
      })),
    },
  ];
  const changeset: MenuSpec = [
    { command: "changeset.switch" },
    { command: "changeset.openPullRequest" },
    "-",
    { command: "changeset.whole", checked: !s.range, radio: true },
    { command: "review.sinceLast", checked: s.range, radio: true },
    "-",
    { command: "changeset.refresh" },
  ];
  const review: MenuSpec = [
    { command: "review.toggleSeen", disabled: inReview },
    { command: "review.reconfirm", disabled: inReview },
    { command: "review.reconfirmAll" },
    "-",
    { command: "review.comment", disabled: inReview },
    { command: "review.commentOnChangeset" },
    "-",
    { command: "review.post", label: s.batchedReview ? "Submit Review…" : undefined },
    ...(s.votes
      ? [
          {
            id: "vote",
            label: "Vote",
            items: s.votes.verdicts.map(({ id, label }) => ({
              command: "review.vote",
              arg: id,
              label,
              checked: s.votes?.chosen === id,
              radio: true,
            })),
          },
        ]
      : []),
    "-",
    { command: "review.finish" },
  ];
  const navigate: MenuSpec = [
    { command: "review.nextHunk", disabled: inReview },
    { command: "review.previousHunk", disabled: inReview },
    { command: "review.nextCommit", disabled: inReview },
    { command: "review.previousCommit", disabled: inReview },
    "-",
    { command: "review.jumpToSource", disabled: inReview },
    { command: "review.goToDefinition", disabled: inReview },
    { command: "review.changeCard", disabled: inReview },
    "-",
    { command: "view.revealFile", disabled: inReview },
  ];
  const help: MenuSpec = [
    { command: "help.keymap" },
    "-",
    { command: "help.showLog" },
    ...(s.canRestart ? [{ command: "app.restartBackend" }] : []),
    { command: "semantic.clearCaches" },
    ...(s.native ? [] : (["-", about] as MenuSpec)),
  ];

  const bar: MenuSpec = [
    ...(s.native
      ? [
          {
            id: "app",
            label: "Vet",
            items: [
              about,
              "-",
              settings,
              "-",
              { predefined: "services" },
              "-",
              { predefined: "hide" },
              { predefined: "hideOthers" },
              { predefined: "showAll" },
              "-",
              quit,
            ] as MenuSpec,
          },
        ]
      : []),
    { id: "file", label: "File", items: file },
    ...(s.native
      ? [
          {
            id: "edit",
            label: "Edit",
            items: [
              { predefined: "undo" },
              { predefined: "redo" },
              "-",
              { predefined: "cut" },
              { predefined: "copy" },
              { predefined: "paste" },
              { predefined: "selectAll" },
            ] as MenuSpec,
          },
        ]
      : []),
    { id: "view", label: "View", items: view },
    { id: "navigate", label: "Navigate", items: navigate },
    { id: "changeset", label: "Changeset", items: changeset },
    { id: "review", label: "Review", items: review },
    ...(s.native
      ? [
          {
            id: "window",
            label: "Window",
            items: [
              { predefined: "minimize" },
              { predefined: "zoom" },
              "-",
              { predefined: "fullscreen" },
              "-",
              { predefined: "bringAllToFront" },
            ] as MenuSpec,
          },
        ]
      : []),
    { id: "help", label: "Help", items: help },
  ];
  return bar;
}

/** A tool window's item: checked while it is shown on the Review screen. */
function toolItem(s: MenuState, id: string) {
  return { command: toolCommand(id), checked: s.screen === "review" && !!s.tools.find((t) => t.id === id)?.shown };
}
