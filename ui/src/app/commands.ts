// The window's commands: the registry built from the app, queue, recent, search, review, comments and
// palette command groups, the app-wide command context, and the wiring of the result sets, the keymap
// and the screen the project opens on. `App` registers them once before its first render.

import { focusFind, focusPipelines, focusProblems, shell } from "../features/chrome";
import { commentsUi, threadCommands, voteBlocked } from "../features/comments";
import { palette, paletteCommands } from "../features/palette";
import { copyText } from "../features/platform";
import { queueCommands } from "../features/queue";
import { fileCommands, onReviewScreen, reviewActions, viewCommands } from "../features/review";
import { searchCommands } from "../features/search";
import { recentCommands } from "../features/welcome";
import { appCommands } from "../state/appCommands";
import { keyId } from "../state/changesetKey";
import { commands } from "../state/commands";
import { findSets } from "../state/findSets.svelte";
import { canQuit, quit } from "../state/host";
import { keymap } from "../state/keymap.svelte";
import { opened } from "../state/opened.svelte";
import { platform } from "../state/platform.svelte";
import { problems } from "../state/problems.svelte";
import { queue } from "../state/queue.svelte";
import { session } from "../state/session.svelte";
import { softWrapPreference, themePreference } from "./appPreferences.svelte";
import { appState } from "./appState.svelte";
import {
  clearSemanticCaches,
  commentOnChangeset,
  openProject,
  openPullRequest,
  pickProject,
  restartLanguageServer,
  showChangesSinceReview,
  showInBrowser,
  showLog,
} from "./actions";

/** Registers the window's commands and the wiring they depend on; answers the function that takes them away. */
export function registerCommands(): () => void {
  const stopCommands = commands.register(
    ...appCommands({
      pickProject: () => void pickProject(),
      openProject,
      closeProject: () => void session.closeRepo(),
      finishReview: () => (appState.dialog = "finish"),
      changesSinceReview: () => void showChangesSinceReview(),
      wholeChangeset: () => void opened.session?.showWhole(),
      refresh: () => void queue.refresh(),
      reconfirmAll: () => void opened.session?.reconfirmAll(),
      openChangeset: (key) => void opened.open(key),
      showScreen: (s) => shell.show(s),
      showChangesets: () => {
        shell.show("review");
        appState.switcherOpen = true;
      },
      openPullRequest: () => void openPullRequest(),
      openInBrowser: (at) => showInBrowser(at ?? opened.session?.pullRequest?.info?.url),
      showDialog: (d) => appState.showDialog(d),
      showKeymap: () => appState.showDialog("settings", "keymap"),
      showAccounts: () => appState.showDialog("settings", "accounts"),
      showProjectSettings: () => appState.showDialog("settings", "trust"),
      quit: () => void quit(),
      toggleSoftWrap: () => (softWrapPreference.value = softWrapPreference.value === "never" ? "prose" : "never"),
      toggleTool: (id) => {
        const bottom = shell.toggle(id);
        if (bottom === "problems") void focusProblems();
        else if (bottom === "find") void focusFind();
        else if (bottom === "pipelines") void focusPipelines();
      },
      stepProblem: (delta) => appState.whenReview((c) => void c.stepProblem(delta)),
      setTheme: (t) => (themePreference.value = t),
      commentOnChangeset,
      postComments: () => {
        shell.show("review");
        commentsUi.openOutgoing(false);
      },
      pickVote: () => {
        shell.show("review");
        commentsUi.openOutgoing(true);
      },
      stageVote: (verdict) => void platform.stageVote(verdict),
      clearSemanticCaches: () => void clearSemanticCaches(),
      showLog: () => void showLog(),
      restartBackend: () => void session.restartBackend(),
      restartLanguageServer: (language) => void restartLanguageServer(language),
    }),
    ...queueCommands({
      open: (key) => {
        if (!queue.onView || keyId(queue.onView) !== keyId(key)) void opened.open(key);
        shell.show("review");
      },
      copyText: (text) => void copyText(text),
    }),
    ...recentCommands({
      open: (path) => {
        if (path !== session.repo) commands.run(`file.openRecent:${path}`);
      },
      forget: (path) => void session.forget(path),
    }),
    ...onReviewScreen(searchCommands(() => appState.controller?.find.options.text)),
    ...viewCommands(reviewActions),
    ...fileCommands(reviewActions),
    ...threadCommands({
      attach: (thread, at) => void opened.session?.threads.attach(thread, at),
      remove: (thread) => void opened.session?.threads.remove(thread),
    }),
    ...paletteCommands({ show: (tab) => palette.show(tab) }),
  );
  keymap.load();
  findSets.show = (focus) => {
    shell.show("review");
    shell.showBottom("find");
    if (focus) void focusFind();
  };

  session.onOpen(() => shell.show("queue"));
  session.onClose(() => {
    palette.close();
    findSets.reset();
  });
  const stopContext = commands.provide(() => ({
    repoOpen: !!session.repo,
    reviewOpen: opened.session?.reviewOpen ?? false,
    queueShown: shell.screen === "queue",
    paletteOpen: palette.open,
    canQuit,
    changesetOpen: !!session.repo && !!opened.session?.layout,
    rangeShown: !!opened.session?.range,
    reconfirmable: (opened.session?.baseChangedCount ?? 0) > 0,
    pullRequest: opened.session?.pullRequest?.info !== undefined,
    outgoing: opened.session?.pullRequest?.outgoingCount ?? 0,
    voteSeparate: !!opened.session?.pullRequest?.info?.traits.verdictSeparate,
    voteBlock: voteBlocked(opened.session?.pullRequest?.info),
    manyCommits: (opened.session?.commitCount ?? 0) > 1,
    problems:
      appState.controller?.problemFile !== undefined &&
      problems.problemsOf([appState.controller.problemFile]).length > 0,
  }));
  return () => {
    stopContext();
    stopCommands();
  };
}
