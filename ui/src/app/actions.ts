// What the window's commands do and the places Search Everywhere and Find in Files open: opening a
// project, the log folder, the caches, the platform's comments, and showing a place. `App` composes
// them into its commands and its screens.

import { tick } from "svelte";
import type { ChoiceHost } from "../features/palette";
import { ON_SCREEN, fileTab, reviewActions } from "../features/review";
import type { FindSet } from "../state/findSets.svelte";
import type { Place } from "../state/foundPlace";
import type { Thread } from "../generated/Thread";
import { browse } from "../state/browse";
import { backendSettings } from "../state/backendSettings.svelte";
import { changeset } from "../state/changeset.svelte";
import { keyId } from "../state/changesetKey";
import { commands } from "../state/commands";
import { canShowFolder, showFolder } from "../state/host";
import { notices } from "../state/notices.svelte";
import { queue } from "../state/queue.svelte";
import { serverList } from "../state/serverList.svelte";
import { session } from "../state/session.svelte";
import { symbols } from "../state/symbols.svelte";
import { defaultRange } from "../state/timelineRange";
import { threads } from "../state/threads.svelte";
import { versions } from "../state/versions.svelte";
import { shell } from "../features/chrome";
import { commentsUi } from "../features/comments";
import { appState } from "./appState.svelte";

/** Opens the folder of the log files in the file manager; a plain browser says where it is. */
export async function showLog() {
  try {
    const path = await session.connected().logDirectory();
    if (canShowFolder) await showFolder(path);
    else notices.show(`The log is in ${path}`);
  } catch (e) {
    notices.failed("Cannot show the log", e);
  }
}

export async function clearSemanticCaches() {
  const problem = await backendSettings.clearCaches();
  notices.show(problem === undefined ? "Semantic caches cleared" : `Cannot clear the semantic caches: ${problem}`);
}

export async function restartLanguageServer(language: string) {
  const problem = await serverList.restart(language);
  if (problem !== undefined) notices.show(`Cannot restart the ${language} language server: ${problem}`);
}

/** Asks for a pull request by its number, in the repository the project has on GitHub. */
export async function openPullRequest() {
  if (!queue.sources.length) await queue.loadPullRequests();
  if (queue.sources.length) appState.dialog = "pullRequest";
  else notices.show("This project has no GitHub remote to open a pull request from");
}

/** Every way of opening a project (recent list, menu, folder dialog, typed path) ends here. */
export function openProject(path: string) {
  void session.openRepo(path);
}

export async function pickProject() {
  const path = await session.pickRepo();
  if (path) openProject(path);
}

/** The review view compares the last reviewed version with the latest; with nothing after it, the changeset shows. */
export async function showChangesSinceReview() {
  const since = defaultRange(versions.items);
  if (!since) notices.show("Nothing has changed since your last review");
  await changeset.setRange(since);
}

/** Shows a web address in the browser, if there is one. */
export function showInBrowser(url: string | undefined) {
  if (url !== undefined) browse(url);
}

/** Goes to the first flagged hunk; when the range or commit on view has none, to the first of the whole changeset. */
export async function goToFirstFlagged() {
  if (reviewActions.firstFlagged()) return;
  await changeset.showWhole();
  await tick();
  reviewActions.firstFlagged();
}

/** Opens the Comments tool window at the form for a comment on the changeset as a whole. */
export function commentOnChangeset() {
  shell.show("review");
  shell.showLeft("comments");
  commentsUi.showConversation();
  commentsUi.composeChangesetComment();
}

/** Shows a thread where it is: at its code, or in the Comments tool window's Conversation tab. */
export function openThread(thread: Thread) {
  shell.show("review");
  if (thread.kind !== "changeset") return void appState.controller?.goToThread(thread);
  shell.showLeft("comments");
  commentsUi.showConversation();
}

/** Opens a place of a result set where it was found. */
export function openPlace(set: FindSet, place: Place, ownTab: boolean) {
  shell.show("review");
  const { spec } = set;
  if (spec.type === "usages") {
    const { file, side, line, col, width } = place;
    void appState.controller?.openUsage({ path: file.path, side, line, col, width, external: file.external }, ownTab);
    return;
  }
  void appState.controller?.openFound({
    path: place.file.path,
    hunks: place.file.hunks,
    kind: place.kind,
    line: place.line,
    col: place.col,
    width: place.width,
    scope: spec.request.scope,
    commit: spec.scope.commit?.id,
    ownTab,
    options: spec.options,
  });
}

/** Does what was chosen in Search Everywhere, which has closed and given the focus back. */
export const choiceHost: ChoiceHost = {
  showReview: () => shell.show("review"),
  commitShown: () => changeset.shownCommit !== undefined,
  showWhole: async () => {
    await changeset.showWhole();
    await tick();
  },
  openOnScreen: (path, line) => void appState.controller?.openSource(ON_SCREEN, path, line),
  openUnpinned: (path, line) => void appState.controller?.openSource(fileTab(path), path, line),
  revealFile: (path) => void appState.controller?.revealInChanges(path),
  revealSymbol: (symbol, change) => {
    symbols.choose(symbol, change, true);
    void appState.controller?.revealSymbol({ symbol, change });
  },
  openChangeset: (id) => {
    if (!queue.onView || keyId(queue.onView) !== id) commands.run(`changeset.open:${id}`);
  },
  openThread: (id) => {
    const thread = threads.items.find((t) => t.id === id);
    if (thread) openThread(thread);
  },
  run: (command) => commands.run(command),
};
