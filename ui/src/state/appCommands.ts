// The commands of the window as a whole: the ones the main menu offers. The owner of the state
// they change (`App`) supplies the actions.

import type { ChangesetKey } from "../generated/ChangesetKey";
import type { Command } from "../lib/commands.ts";
import { THEMES, type Theme } from "../lib/theme/theme.ts";
import { parseKeyId } from "./changesetKey.ts";
import type { CommandContext } from "./commands.ts";

export interface AppActions {
  pickProject(): void;
  openProject(path: string): void;
  /** Closes the open project and shows the welcome screen. */
  closeProject(): void;
  finishReview(): void;
  /** Shows what changed since the version last reviewed. */
  changesSinceReview(): void;
  /** Shows the whole changeset again, after a range of versions. */
  wholeChangeset(): void;
  /** Fetches the open project's remotes and lists its changesets again. */
  refresh(): void;
  /** Marks every hunk seen against an old base seen again. */
  reconfirmAll(): void;
  /** Puts a changeset on view. */
  openChangeset(key: ChangesetKey): void;
  /** Shows the queue of changesets, or the review of the one on view. */
  showScreen(screen: "queue" | "review"): void;
  /** Opens the changeset switcher. */
  showChangesets(): void;
  /** Asks for a pull request by its number. */
  openPullRequest(): void;
  /** Shows the pull request at `url`, or the one on view, on its platform's web page. */
  openInBrowser(url?: string): void;
  showDialog(dialog: "settings" | "about"): void;
  /** Opens Settings on the Keymap page. */
  showKeymap(): void;
  /** Opens Settings on the Accounts page. */
  showAccounts(): void;
  /** Opens Settings on the Trust page, the first of the project's pages. */
  showProjectSettings(): void;
  quit(): void;
  /** Wraps long lines, or stops wrapping them. */
  toggleSoftWrap(): void;
  /** Shows or hides the tool window `id`. */
  toggleTool(id: string): void;
  /** Shows the problem `delta` after the one shown, new ones first. */
  /** Shows the problem after (before) the caret in the file on view. */
  stepProblem(delta: 1 | -1): void;
  setTheme(theme: Theme): void;
  /** Opens the comment form on the changeset as a whole. */
  commentOnChangeset(): void;
  /** Opens the list of the comments and the vote that wait to be sent, with the button that sends them. */
  postComments(): void;
  /** Opens the picker of the viewer's vote at the toolbar's outgoing button. */
  pickVote(): void;
  /** Stages the viewer's vote on the pull request. */
  stageVote(verdict: string): void;
  /** Clears the backend's semantic caches. */
  clearSemanticCaches(): void;
  /** Opens the folder of the log files, or says where it is. */
  showLog(): void;
  /** Ends the backend and starts another. */
  restartBackend(): void;
  /** Restarts the open project's servers of `language`, and says if they do not start. */
  restartLanguageServer(language: string): void;
}

/**
 * The tool windows of the rails, with the key that shows or hides each. The keys count the left
 * rail's tools, then the bottom's, then the right's, each from the top down or left to right.
 */
export const TOOL_WINDOWS: readonly {
  side: "left" | "right" | "bottom";
  id: string;
  label: string;
  binding: string;
}[] = [
  { side: "left", id: "changes", label: "Changes", binding: "Alt-1" },
  { side: "left", id: "all", label: "All Files", binding: "Alt-2" },
  { side: "left", id: "commits", label: "Commits", binding: "Alt-3" },
  { side: "left", id: "comments", label: "Comments", binding: "Alt-4" },
  { side: "bottom", id: "timeline", label: "Timeline", binding: "Alt-5" },
  { side: "bottom", id: "problems", label: "Problems", binding: "Alt-6" },
  { side: "bottom", id: "find", label: "Find", binding: "Alt-7" },
  { side: "bottom", id: "pipelines", label: "Pipelines", binding: "Alt-8" },
  { side: "right", id: "symbol", label: "Symbol", binding: "Alt-9" },
  { side: "right", id: "structure", label: "Structure", binding: "Alt-0" },
];

/** The id of the command that shows or hides the tool window `id`. */
export const toolCommand = (id: string) => `view.tool.${id}`;

const NO_PROJECT = "Open a project first";
const NO_CHANGESET = "No changeset is on view";
const NO_REVIEW = "No review is open";
const NO_PULL_REQUEST = "No pull request is on view";
const NO_PROBLEMS = "The file on view has no problems";

export function appCommands(a: AppActions): Command<CommandContext>[] {
  const repoOpen = (c: CommandContext) => c.repoOpen;
  const needsProject = () => NO_PROJECT;
  const changesetOpen = (c: CommandContext) => !!c.changesetOpen;
  return [
    { id: "file.open", label: "Open Project…", binding: "Mod-o", accelerator: "Cmd+O", run: () => a.pickProject() },
    {
      id: "file.openRecent",
      label: "Open Recent",
      argument: true,
      run: (_, path) => {
        if (path === undefined) return false;
        a.openProject(path);
      },
    },
    {
      id: "file.close",
      label: "Close Project",
      enabled: repoOpen,
      disabledReason: needsProject,
      run: () => a.closeProject(),
    },
    {
      id: "view.queue",
      label: "Queue",
      binding: "Mod-1",
      accelerator: "Cmd+1",
      enabled: repoOpen,
      disabledReason: needsProject,
      run: () => a.showScreen("queue"),
    },
    {
      id: "view.review",
      label: "Review",
      binding: "Mod-2",
      accelerator: "Cmd+2",
      enabled: repoOpen,
      disabledReason: needsProject,
      run: () => a.showScreen("review"),
    },
    {
      id: "changeset.switch",
      label: "Switch Changeset…",
      binding: "Mod-Shift-o",
      accelerator: "Cmd+Shift+O",
      enabled: repoOpen,
      disabledReason: needsProject,
      run: () => a.showChangesets(),
    },
    {
      id: "changeset.openPullRequest",
      label: "Open Pull Request by Number…",
      enabled: repoOpen,
      disabledReason: needsProject,
      run: () => a.openPullRequest(),
    },
    {
      id: "changeset.openInBrowser",
      label: "Open in Browser",
      // On a queue row's menu it is that row's pull request, else the one on view.
      enabled: (c) => (c.row ? c.row.url !== undefined : !!c.pullRequest),
      disabledReason: (c) => (c.row ? "Only a pull request has a page on its platform" : NO_PULL_REQUEST),
      run: (c) => a.openInBrowser(c.row ? c.row.url : undefined),
    },
    {
      id: "changeset.open",
      label: "Open Changeset",
      argument: true,
      enabled: repoOpen,
      run: (_, id) => {
        const key = parseKeyId(id);
        if (!key) return false;
        a.openChangeset(key);
      },
    },
    {
      id: "changeset.whole",
      label: "Whole Changeset",
      enabled: changesetOpen,
      disabledReason: () => NO_CHANGESET,
      run: () => a.wholeChangeset(),
    },
    {
      id: "review.sinceLast",
      label: "Changes Since My Last Review",
      enabled: (c) => c.reviewOpen,
      disabledReason: () => NO_REVIEW,
      run: () => a.changesSinceReview(),
    },
    {
      id: "changeset.toggleSinceLast",
      label: "Toggle Changes Since My Last Review",
      binding: "Ctrl-Alt-l",
      enabled: (c) => (c.rangeShown ? !!c.changesetOpen : c.reviewOpen),
      disabledReason: () => NO_REVIEW,
      run: (c) => (c.rangeShown ? a.wholeChangeset() : a.changesSinceReview()),
    },
    {
      id: "changeset.refresh",
      label: "Fetch and Refresh",
      binding: "Mod-t",
      accelerator: "Cmd+T",
      enabled: repoOpen,
      disabledReason: needsProject,
      run: () => a.refresh(),
    },
    {
      id: "review.reconfirmAll",
      label: "Re-confirm All",
      enabled: (c) => !!c.reconfirmable,
      disabledReason: () => "No hunk was seen against an old base",
      run: () => a.reconfirmAll(),
    },
    {
      id: "review.commentOnChangeset",
      label: "Comment on Changeset",
      enabled: changesetOpen,
      disabledReason: () => NO_CHANGESET,
      run: () => a.commentOnChangeset(),
    },
    {
      id: "review.post",
      label: "Post Comments…",
      binding: "Mod-Shift-k",
      enabled: (c) => !!c.pullRequest && !!c.outgoing && !c.rangeShown,
      disabledReason: (c) =>
        !c.pullRequest
          ? NO_PULL_REQUEST
          : c.rangeShown
            ? "Show the whole changeset to post comments"
            : "No comment or vote waits to be sent",
      run: () => a.postComments(),
    },
    {
      id: "review.pickVote",
      label: "Choose Vote",
      binding: "Ctrl-Alt-v",
      enabled: (c) => !!c.pullRequest && !!c.voteSeparate && c.voteBlock === undefined,
      disabledReason: (c) =>
        !c.pullRequest
          ? NO_PULL_REQUEST
          : !c.voteSeparate
            ? "This platform sets the vote with the review"
            : c.voteBlock,
      run: () => a.pickVote(),
    },
    {
      id: "review.vote",
      label: "Vote",
      argument: true,
      enabled: (c) => !!c.pullRequest && c.voteBlock === undefined,
      disabledReason: (c) => (c.pullRequest ? c.voteBlock : NO_PULL_REQUEST),
      run: (_, verdict) => {
        if (!verdict) return false;
        a.stageVote(verdict);
      },
    },
    {
      id: "review.finish",
      label: "Finish Review…",
      enabled: (c) => c.reviewOpen,
      disabledReason: () => NO_REVIEW,
      run: () => a.finishReview(),
    },
    {
      id: "app.settings",
      label: "Settings…",
      binding: "Ctrl-Alt-s",
      accelerator: "Cmd+,",
      inFields: true,
      run: () => a.showDialog("settings"),
    },
    {
      id: "app.projectSettings",
      label: "Project Settings…",
      enabled: repoOpen,
      disabledReason: needsProject,
      run: () => a.showProjectSettings(),
    },
    { id: "app.accounts", label: "Accounts…", run: () => a.showAccounts() },
    { id: "app.about", label: "About Vet", run: () => a.showDialog("about") },
    {
      id: "app.quit",
      label: "Exit",
      accelerator: "Cmd+Q",
      enabled: (c) => c.canQuit,
      disabledReason: () => "Only the desktop app can be exited",
      run: () => a.quit(),
    },
    { id: "view.softWrap", label: "Soft Wrap", run: () => a.toggleSoftWrap() },
    ...TOOL_WINDOWS.map(({ id, label, binding }): Command<CommandContext> => ({
      id: toolCommand(id),
      label,
      binding,
      enabled:
        id === "commits"
          ? (c) => c.repoOpen && !!c.manyCommits
          : id === "pipelines"
            ? (c) => c.repoOpen && !!c.pullRequest
            : repoOpen,
      disabledReason: (c) =>
        !c.repoOpen
          ? NO_PROJECT
          : id === "pipelines"
            ? NO_PULL_REQUEST
            : "Only a changeset of several commits can be reviewed by commit",
      run: () => a.toggleTool(id),
    })),
    {
      id: "nav.nextProblem",
      label: "Next Problem",
      binding: "F2",
      enabled: (c) => !!c.problems,
      disabledReason: () => NO_PROBLEMS,
      run: () => a.stepProblem(1),
    },
    {
      id: "nav.previousProblem",
      label: "Previous Problem",
      binding: "Shift-F2",
      enabled: (c) => !!c.problems,
      disabledReason: () => NO_PROBLEMS,
      run: () => a.stepProblem(-1),
    },
    {
      id: "view.theme",
      label: "Theme",
      argument: true,
      run: (_, id) => {
        const theme = THEMES.find((t) => t.id === id);
        if (!theme) return false;
        a.setTheme(theme.id);
      },
    },
    { id: "semantic.clearCaches", label: "Clear Semantic Caches", run: () => a.clearSemanticCaches() },
    { id: "help.keymap", label: "Keyboard Shortcuts", run: () => a.showKeymap() },
    { id: "help.showLog", label: "Show Log Folder", run: () => a.showLog() },
    { id: "app.restartBackend", label: "Restart Backend", run: () => a.restartBackend() },
    {
      id: "app.restartLanguageServer",
      label: "Restart Language Server",
      enabled: (c) => c.serverLanguage !== undefined,
      disabledReason: () => "The active file has no language server",
      run: (c) => {
        if (c.serverLanguage === undefined) return false;
        a.restartLanguageServer(c.serverLanguage);
      },
    },
  ];
}
