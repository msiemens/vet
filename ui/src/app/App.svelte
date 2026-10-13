<script lang="ts">
  import { type ComponentProps, onMount, tick, untrack } from "svelte";
  import {
    AboutDialog,
    Banners,
    FinishDialog,
    Header,
    MainMenuButton,
    StatusFooter,
    TOOLS,
    TaskProgress,
    TitleBar,
    ToolRail,
    type Screen,
    baseNote,
    lensOnSelect,
    scopeChip,
    toggleBottom,
    toggleTool,
    toolFor,
  } from "../features/chrome";
  import { ChecksButton, badgeCount } from "../features/checks";
  import { LocationPicker } from "../features/locations";
  import {
    OutgoingButton,
    threadCommands,
    commentsSeen,
    outgoingComments,
    seenKey,
    seenNow,
    submitBlock,
    summarizeOutgoing,
    unreadCount,
  } from "../features/comments";
  import {
    type ChoiceHost,
    Palette,
    openChoice,
    palette,
    paletteCommands,
    watchDoubleShift,
  } from "../features/palette";
  import { NotificationCards, PlatformFooter, copyText } from "../features/platform";
  import { FindPanel } from "../features/find";
  import { PipelinesPanel, belongsTo, clock, headerTitle, logKey } from "../features/pipelines";
  import { ProblemsPanel } from "../features/problems";
  import { ChangesetSwitcher, OpenPullRequestDialog, QueueScreen, queueCommands, targetLabel } from "../features/queue";
  import {
    ON_SCREEN,
    ReviewScreen,
    type ReviewController,
    fileCommands,
    fileTab,
    reviewActions,
    modeChange,
    statusLetter,
    viewCommands,
  } from "../features/review";
  import { FindInFilesDialog, findInFiles, searchCommands } from "../features/search";
  import { SettingsDialog } from "../features/settings";
  import { StructurePanel } from "../features/structure";
  import { type OpenLocation, SymbolPanel } from "../features/symbols";
  import { type RangeFile, TimelinePanel } from "../features/timeline";
  import { ProjectSwitcher, recentCommands, WelcomeScreen } from "../features/welcome";
  import type { Thread } from "../generated/Thread";
  import { errorText } from "../lib/errorText";
  import { repoName } from "../lib/paths";
  import { platformName } from "../lib/platformName";
  import { type SoftWrap, storeSoftWrap, storedSoftWrap } from "../lib/softWrap";
  import { type Theme, applyTheme, storedTheme } from "../lib/theme/theme";
  import ContextMenu from "../lib/ui/ContextMenu.svelte";
  import { setDoubleClickMs } from "../lib/ui/doubleClick";
  import Toast from "../lib/ui/Toast.svelte";
  import { changeset } from "../state/changeset.svelte";
  import { checks } from "../state/checks.svelte";
  import {
    appVersion,
    canQuit,
    canRestartBackend,
    canShowFolder,
    hasNativeDialogs,
    hasNativeMenu,
    hostLabel,
    inTauri,
    openUrl,
    quit,
    showFolder,
    showWindow,
    systemDoubleClickMs,
  } from "../state/host";
  import { appCommands } from "../state/appCommands";
  import { backendSettings } from "../state/backendSettings.svelte";
  import { keyId } from "../state/changesetKey";
  import { commands } from "../state/commands";
  import { contextMenus } from "../state/contextMenu.svelte";
  import { type FindSet, findSets } from "../state/findSets.svelte";
  import type { Place } from "../state/foundPlace";
  import { keymap } from "../state/keymap.svelte";
  import { mainMenu } from "../state/mainMenu.svelte";
  import { pendingEdits } from "../state/pendingEdits.svelte";
  import { platform } from "../state/platform.svelte";
  import { problems } from "../state/problems.svelte";
  import { progress as backgroundTasks } from "../state/progress.svelte";
  import { queue } from "../state/queue.svelte";
  import { recent } from "../state/recent.svelte";
  import { semantic } from "../state/semantic.svelte";
  import { serverList } from "../state/serverList.svelte";
  import { serverLine } from "../state/serverStatus";
  import { session } from "../state/session.svelte";
  import { symbols } from "../state/symbols.svelte";
  import { threads } from "../state/threads.svelte";
  import { defaultRange } from "../state/timelineRange";
  import { versions } from "../state/versions.svelte";
  import ConnectionNotice from "./ConnectionNotice.svelte";

  /** How many of a pull request's latest versions have their checks asked for, for the timeline. */
  const MAX_CHECKED_VERSIONS = 20;

  let controller = $state.raw<ReviewController>();
  // The selected tool of each rail, and whether its sidebar is shown.
  const left = $state({ tool: "changes", open: true });
  const right = $state({ tool: "symbol", open: true });
  // The bottom tool whose panel is shown; they exclude each other.
  let bottomTool = $state<string>();
  let theme = $state<Theme>(storedTheme());
  let softWrap = $state<SoftWrap>(storedSoftWrap());
  let dialog = $state<"settings" | "about" | "finish" | "pullRequest">();
  let settingsPage = $state<ComponentProps<typeof SettingsDialog>["page"]>("general");
  let switcherOpen = $state(false);
  // The queue is the project's home; the review screen stays mounted behind it, so its views survive.
  let screen = $state<Screen>("queue");
  // Only the host knows these; a plain browser shows none.
  let version = $state<string>();
  let host = $state<string>();
  const reviewOpen = $derived(changeset.facts?.review.state === "open");
  const lens = $derived(changeset.facts?.lens);
  const commitCount = $derived(changeset.facts?.commits ?? 0);

  // Selecting the Commits tool reviews the changeset by commit and leaving it returns to all changes.
  let selectedTool = untrack(() => left.tool);
  $effect(() => {
    const tool = left.tool;
    const want = lensOnSelect(selectedTool, tool);
    selectedTool = tool;
    if (!want) return;
    untrack(() => {
      if (changeset.facts && lens !== want) void changeset.showLens(want);
    });
  });
  // A lens set elsewhere (a changeset reopened by commit) selects the tool that goes with it.
  $effect(() => {
    const next = toolFor(
      untrack(() => left.tool),
      lens,
      commitCount,
    );
    untrack(() => (left.tool = next));
  });
  /** Goes to the first flagged hunk; when the range or commit on view has none, to the first of the whole changeset. */
  async function goToFirstFlagged() {
    if (reviewActions.firstFlagged()) return;
    await changeset.showWhole();
    await tick();
    reviewActions.firstFlagged();
  }
  const baseChangedLeft = $derived(changeset.baseChanged ? changeset.baseChangedFiles().length : 0);

  /** The head commit of a pull request, whose checks the toolbar shows. */
  const headId = $derived(
    changeset.facts?.target.changeset.type === "pullRequest" ? changeset.facts.target.head : undefined,
  );
  /** The commits whose checks the views show: the commits by commit, the pushed versions and the head. */
  const checkedCommits = $derived(
    headId === undefined
      ? []
      : [
          ...new Set([
            ...changeset.commits.map((c) => c.commit.id),
            ...versions.items.slice(-MAX_CHECKED_VERSIONS).flatMap((v) => (v.tip ? [v.tip.id] : [])),
            headId,
          ]),
        ],
  );
  $effect(() => {
    const wanted = checkedCommits;
    untrack(() => checks.want(wanted));
  });
  /** The commit whose runs the Pipelines tool shows: the one on view by commit, else the head. */
  const pipelineCommit = $derived(
    changeset.shownCommit === undefined ? headId : changeset.commits[changeset.shownCommit]?.commit.id,
  );
  const platformSignedOut = $derived(
    platform.status?.connection.type === "signedOut" || platform.status?.connection.type === "tokenExpired",
  );
  const headChecksLook = $derived.by(() => {
    const head = checks.of(headId);
    const status = platform.status;
    if (!head) return undefined;
    const until = checks.limitedUntil;
    const limited =
      until === undefined || !status
        ? undefined
        : {
            until: clock(until),
            asOf: clock(checks.loadedAt ?? until),
            platform: platformName(status.kind),
          };
    const title = platformSignedOut
      ? `${status && platformName(status.kind)}: the token for ${status?.host} is not valid. Checks are not loaded.`
      : headerTitle(head, limited);
    return { head, title, stale: limited !== undefined };
  });
  /** The rail's count for Pipelines: what the head's checks are about, while they are not all passed. */
  const pipelineBadge = $derived.by(() => {
    const head = checks.of(headId);
    if (!head || platformSignedOut || head.state === "success" || head.state === undefined) {
      return { count: 0, text: "" };
    }
    const count = badgeCount(head);
    const word = head.state === "failure" ? "failing" : head.state === "pending" ? "running" : "waiting";
    return { count, text: `${count} ${word}`, bad: head.state === "failure" };
  });
  // The tool belongs to a pull request; while the changeset reloads there are no facts yet and it stays.
  $effect(() => {
    if (!belongsTo(changeset.facts?.target.changeset.type) && bottomTool === "pipelines") bottomTool = undefined;
  });

  /** Where the time the pull request's Comments tool window was last open is kept; none for another changeset. */
  const seenUnder = $derived(seenKey(changeset.facts?.target.changeset));
  const commentsShown = $derived(screen === "review" && left.open && left.tool === "comments");
  // While the tool window is open, what comes in is seen.
  $effect(() => {
    const key = seenUnder;
    if (!commentsShown || key === undefined) return;
    const at = seenNow(platform.info, threads.items, Math.floor(Date.now() / 1000));
    untrack(() => commentsSeen.mark(key, at));
  });
  const unreadComments = $derived(
    seenUnder === undefined || commentsShown
      ? 0
      : unreadCount(platform.info, threads.items, commentsSeen.at(seenUnder)),
  );

  const semanticSummary = $derived(semantic.summary(controller?.semanticPath));
  const range = $derived(changeset.range);
  const rangeFiles = $derived<RangeFile[]>(
    range ? (changeset.layout?.files ?? []).map((f) => ({ path: f.path, delta: f.delta, mode: modeChange(f) })) : [],
  );
  const shownCommit = $derived(
    changeset.shownCommit === undefined ? undefined : changeset.commits[changeset.shownCommit],
  );
  const scope = $derived(
    changeset.facts &&
      scopeChip({
        range,
        latest: versions.items.at(-1)?.n,
        commit:
          changeset.shownCommit === undefined
            ? undefined
            : {
                position: changeset.shownCommit + 1,
                of: changeset.facts.commits,
                shortId: shownCommit?.commit.shortId,
                subject: shownCommit?.commit.subject,
              },
        live: changeset.facts.target.changeset.type === "workingTree",
        pending: pendingEdits.waiting,
      }),
  );
  const note = $derived(changeset.facts && baseNote(changeset.facts.base, changeset.facts.target.changeset.type));
  const footer = $derived({
    commit:
      changeset.shownCommit === undefined ? undefined : `${changeset.shownCommit + 1} / ${changeset.commits.length}`,
    languageServer: serverLine(controller?.semanticPath, serverList.servers, backgroundTasks.tasks, semanticSummary),
    problems: problems.summary,
  });
  // The changeset's problems are asked about while the review is on screen.
  $effect(() => {
    if (!session.backend || !session.repo) return;
    return problems.start();
  });

  const openLocation: OpenLocation = (path, line, side) => void controller?.openLocation(ON_SCREEN, path, line, side);

  // Registered before the first render, so the menu below is built from them.
  const stopCommands = commands.register(
    ...appCommands({
      pickProject: () => void pickProject(),
      openProject,
      closeProject: () => void session.closeRepo(),
      finishReview: () => (dialog = "finish"),
      changesSinceReview: () => void showChangesSinceReview(),
      wholeChangeset: () => void changeset.showWhole(),
      refresh: () => void queue.refresh(),
      reconfirmAll: () => void changeset.reconfirmAll(),
      openChangeset: (key) => void changeset.openChangeset(key),
      showScreen: (s) => (screen = s),
      showChangesets: () => {
        screen = "review";
        switcherOpen = true;
      },
      openPullRequest: () => void openPullRequest(),
      openInBrowser: (at) => browse(at ?? platform.info?.url),
      showDialog: (d) => showDialog(d),
      showKeymap: () => showDialog("settings", "keymap"),
      showAccounts: () => showDialog("settings", "accounts"),
      quit: () => void quit(),
      toggleSoftWrap: () => setSoftWrap(softWrap === "never" ? "prose" : "never"),
      toggleTool: (id) => {
        if (TOOLS.bottom.some((t) => t.id === id)) {
          bottomTool = toggleBottom(bottomTool, id);
          if (bottomTool === "problems") void focusProblems();
          if (bottomTool === "find") void focusFind();
          if (bottomTool === "pipelines") void focusPipelines();
          return;
        }
        const side = TOOLS.left.some((t) => t.id === id) ? left : right;
        Object.assign(side, toggleTool(side, id));
      },
      stepProblem: (delta) => void controller?.stepProblem(delta),
      setTheme: (t) => applyTheme((theme = t)),
      commentOnChangeset,
      postComments: () => {
        screen = "review";
        platform.votePicking = false;
        platform.outgoingOpen = true;
      },
      pickVote: () => {
        screen = "review";
        platform.votePicking = true;
        platform.outgoingOpen = true;
      },
      stageVote: (verdict) => void platform.stageVote(verdict),
      clearSemanticCaches: () => void clearSemanticCaches(),
      showLog: () => void showLog(),
      restartBackend: () => void session.restartBackend(),
      restartLanguageServer: (language) => void restartLanguageServer(language),
    }),
    ...queueCommands({
      open: (key) => {
        if (!queue.onView || keyId(queue.onView) !== keyId(key)) void changeset.openChangeset(key);
        screen = "review";
      },
      copyText: (text) => void copyText(text),
    }),
    ...recentCommands({
      open: (path) => {
        if (path !== session.repo) commands.run(`file.openRecent:${path}`);
      },
      forget: (path) => void session.forget(path),
    }),
    ...searchCommands(() => controller?.find.options.text),
    ...viewCommands(reviewActions),
    ...fileCommands(reviewActions),
    ...threadCommands({
      attach: (thread, at) => void threads.attach(thread, at),
      remove: (thread) => void threads.remove(thread),
    }),
    ...paletteCommands({ show: (tab) => palette.show(tab) }),
  );
  keymap.load();
  findSets.show = (focus) => {
    screen = "review";
    bottomTool = "find";
    if (focus) void focusFind();
  };
  session.onOpen(() => (screen = "queue"));
  session.onClose(() => {
    palette.close();
    findSets.reset();
  });
  /** Comments and a staged vote that wait to be sent to the platform of the pull request on view. */
  const outgoing = $derived(
    platform.info === undefined
      ? 0
      : summarizeOutgoing(outgoingComments(threads.items), platform.info.pendingVote).count,
  );
  const voteBlock = $derived.by(() => {
    const info = platform.info;
    if (info?.pendingVote?.state.type === "posting") return "The vote is being sent";
    return info && submitBlock(info.platform.connection);
  });
  const stopContext = commands.provide(() => ({
    repoOpen: !!session.repo,
    reviewOpen,
    queueShown: screen === "queue",
    paletteOpen: palette.open,
    canQuit,
    changesetOpen: !!session.repo && !!changeset.layout,
    rangeShown: !!range,
    reconfirmable: baseChangedLeft > 0,
    pullRequest: platform.info !== undefined,
    outgoing,
    voteSeparate: !!platform.traits?.verdictSeparate,
    voteBlock,
    manyCommits: commitCount > 1,
    problems: controller?.problemFile !== undefined && problems.problemsOf([controller.problemFile]).length > 0,
  }));

  // The main menu follows the state it shows.
  $effect(() => {
    const shown = (side: typeof left, id: string) => side.open && side.tool === id;
    const traits = platform.traits;
    mainMenu.update({
      native: hasNativeMenu,
      recent: recent.list.map(({ path }) => ({ path, name: repoName(path) })),
      screen,
      viewMode: controller?.activeMode,
      softWrap: softWrap !== "never",
      theme,
      tools: [
        ...TOOLS.left.map((t) => ({ id: t.id, shown: shown(left, t.id) })),
        ...TOOLS.right.map((t) => ({ id: t.id, shown: shown(right, t.id) })),
        ...TOOLS.bottom.map((t) => ({ id: t.id, shown: bottomTool === t.id })),
      ],
      range: !!range,
      canRestart: canRestartBackend,
      batchedReview: !!traits?.batchedReview,
      votes: traits?.verdictSeparate
        ? {
            verdicts: traits.verdicts,
            chosen: platform.info?.pendingVote?.verdict ?? platform.info?.viewerVerdict ?? undefined,
          }
        : undefined,
    });
  });

  onMount(() => {
    // Capturing, so a command's key reaches it before any control or editor handles the key.
    const onkeydown = (e: KeyboardEvent) => void commands.dispatch(e);
    window.addEventListener("keydown", onkeydown, true);
    window.addEventListener("keydown", leaveScope);
    // The webview's own menu stays on text fields; elsewhere a handler opened the app's menu, or there is none.
    const onContextMenu = (e: MouseEvent) => {
      if (!(e.target instanceof Element && e.target.closest("input, textarea"))) e.preventDefault();
    };
    window.addEventListener("contextmenu", onContextMenu);
    applyTheme(theme);
    // The viewer may change the mouse settings while the window stays open.
    const readDoubleClick = () =>
      void systemDoubleClickMs().then(
        (ms) => ms !== undefined && setDoubleClickMs(ms),
        (e: unknown) => console.error(e),
      );
    readDoubleClick();
    window.addEventListener("focus", readDoubleClick);
    // The splash covers the start until the backend is connected (or the connection failed, which
    // the window then shows), so the window never shows empty.
    let shown: Promise<void> | undefined;
    const reveal = () => {
      shown ??= showWindow().then(
        (frame) => {
          if (frame === "native") console.warn("app-drawn title bar unavailable; the host log says why");
        },
        (e: unknown) => {
          console.error(e);
        },
      );
    };
    if (inTauri) void appVersion().then((v) => (version = v));
    void hostLabel().then((label) => (host = label));
    session.onConnect(reveal);
    void session.connect().then(reveal);
    // Result sets belong to the changeset on view.
    const stopSets = changeset.onOpening((opening) => {
      if (opening.type === "requested") findSets.reset();
    });
    const stopMenu = mainMenu.listen();
    const stopDoubleShift = watchDoubleShift(
      () => !!session.repo && !commands.isCapturing(),
      () => commands.run("palette.all"),
    );
    return () => {
      stopSets();
      stopMenu();
      stopDoubleShift();
      window.removeEventListener("keydown", onkeydown, true);
      window.removeEventListener("keydown", leaveScope);
      window.removeEventListener("contextmenu", onContextMenu);
      window.removeEventListener("focus", readDoubleClick);
      stopContext();
      stopCommands();
    };
  });

  function showDialog(d: "settings" | "about", page: typeof settingsPage = "general") {
    settingsPage = page;
    dialog = d;
  }

  /** Opens the folder of the log files in the file manager; a plain browser says where it is. */
  async function showLog() {
    try {
      const path = await session.connected().logDirectory();
      if (canShowFolder) await showFolder(path);
      else session.notice = `The log is in ${path}`;
    } catch (e) {
      session.notice = `Cannot show the log: ${errorText(e)}`;
    }
  }

  async function clearSemanticCaches() {
    const problem = await backendSettings.clearCaches();
    session.notice = problem === undefined ? "Semantic caches cleared" : `Cannot clear the semantic caches: ${problem}`;
  }

  async function restartLanguageServer(language: string) {
    const problem = await serverList.restart(language);
    if (problem !== undefined) session.notice = `Cannot restart the ${language} language server: ${problem}`;
  }

  /** Asks for a pull request by its number, in the repository the project has on GitHub. */
  async function openPullRequest() {
    if (!queue.sources.length) await queue.loadPullRequests();
    if (queue.sources.length) dialog = "pullRequest";
    else session.notice = "This project has no GitHub remote to open a pull request from";
  }

  /** Every way of opening a project (recent list, menu, folder dialog, typed path) ends here. */
  function openProject(path: string) {
    void session.openRepo(path);
  }

  async function pickProject() {
    const path = await session.pickRepo();
    if (path) openProject(path);
  }

  /** The review view compares the last reviewed version with the latest; with nothing after it, the changeset shows. */
  async function showChangesSinceReview() {
    const since = defaultRange(versions.items);
    if (!since) session.notice = "Nothing has changed since your last review";
    await changeset.setRange(since);
  }

  /** Shows a tool window: the Timeline under the editors, or one of the left rail's. */
  function showTool(tool: "changes" | "all" | "comments" | "timeline") {
    if (tool === "timeline") bottomTool = "timeline";
    else Object.assign(left, { tool, open: true });
  }

  /** Shows the timeline, or moves the focus into it where it is shown. */
  async function focusTimeline() {
    bottomTool = "timeline";
    await tick();
    document.querySelector<HTMLElement>('[aria-label="Timeline"]')?.focus();
  }

  /** Shows a web address in the browser. */
  function browse(url: string | undefined) {
    if (url === undefined) return;
    void openUrl(url).catch((e: unknown) => {
      session.notice = `Could not open the browser: ${errorText(e)}`;
    });
  }

  /** Shows the Pipelines tool, or moves the focus into it where it is shown. */
  async function focusPipelines() {
    screen = "review";
    bottomTool = "pipelines";
    await tick();
    const panel = document.querySelector<HTMLElement>('[aria-label="Pipelines"]');
    (panel?.querySelector<HTMLElement>("[data-list]") ?? panel)?.focus();
  }

  /** Moves the focus into the Problems list once it is up. */
  async function focusProblems() {
    await tick();
    document.querySelector<HTMLElement>('[aria-label="Problems"] [data-list]')?.focus();
  }

  /** Moves the focus into the Find list once it is up. */
  async function focusFind() {
    await tick();
    document.querySelector<HTMLElement>('[aria-label="Find"] [data-list]')?.focus();
  }

  /** Opens a place of a result set where it was found. */
  function openPlace(set: FindSet, place: Place, ownTab: boolean) {
    screen = "review";
    const { spec } = set;
    if (spec.type === "usages") {
      const { file, side, line, col, width } = place;
      void controller?.openUsage({ path: file.path, side, line, col, width, external: file.external }, ownTab);
      return;
    }
    void controller?.openFound({
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

  /** Escape, with the focus nowhere in particular or in the timeline or the chip, goes back to all changes. */
  function leaveScope(e: KeyboardEvent) {
    if (e.key !== "Escape" || e.defaultPrevented || e.metaKey || e.ctrlKey || e.altKey || e.shiftKey) return;
    if (!scope?.narrowed) return;
    const at = e.target instanceof Element ? e.target : undefined;
    if (at && at !== document.body && !at.closest("[data-escapes-scope]")) return;
    e.preventDefault();
    void changeset.showWhole();
  }

  /** Shows a thread where it is: at its code, or in the Comments tool window's Conversation tab. */
  function openThread(thread: Thread) {
    screen = "review";
    if (thread.kind !== "changeset") return void controller?.goToThread(thread);
    Object.assign(left, { tool: "comments", open: true });
    platform.commentsTab = "conversation";
  }

  /** Does what was chosen in Search Everywhere, which has closed and given the focus back. */
  const choiceHost: ChoiceHost = {
    showReview: () => (screen = "review"),
    commitShown: () => changeset.shownCommit !== undefined,
    showWhole: async () => {
      await changeset.showWhole();
      await tick();
    },
    openOnScreen: (path, line) => void controller?.openSource(ON_SCREEN, path, line),
    openUnpinned: (path, line) => void controller?.openSource(fileTab(path), path, line),
    revealFile: (path) => void controller?.revealInChanges(path),
    revealSymbol: (symbol, change) => {
      symbols.choose(symbol, change, true);
      void controller?.revealSymbol({ symbol, change });
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

  /** Opens the Comments tool window at the form for a comment on the changeset as a whole. */
  function commentOnChangeset() {
    screen = "review";
    Object.assign(left, { tool: "comments", open: true });
    platform.commentsTab = "conversation";
    platform.changesetComment.composing = true;
  }

  function setSoftWrap(next: SoftWrap) {
    softWrap = next;
    storeSoftWrap(next);
    controller?.setSoftWrap(next);
  }
</script>

{#if session.backend && session.repo}
  {#snippet baseChangedActions()}
    <button
      class="ctl"
      type="button"
      title={commands.hint("review.reconfirmAll", "Mark every hunk seen against an old base seen again")}
      disabled={!commands.isEnabled("review.reconfirmAll")}
      onclick={() => commands.run("review.reconfirmAll")}>{commands.label("review.reconfirmAll")}</button
    >
    <button class="ctl" type="button" title="Show all changes again" onclick={() => changeset.showAll()}
      >Back to all changes</button
    >
  {/snippet}
  {#snippet headChecks()}
    {#if headChecksLook}
      <ChecksButton
        checks={headChecksLook.head}
        title={headChecksLook.title}
        stale={headChecksLook.stale}
        warn={platformSignedOut}
        onopen={() => void focusPipelines()}
      />
    {/if}
  {/snippet}
  {#snippet outgoingComments()}
    <!-- A changeset that is no pull request has nowhere to post to. -->
    {#if headId !== undefined}<OutgoingButton onopen={openThread} />{/if}
  {/snippet}
  {#snippet projectSwitcher()}
    {#if session.repo}
      <ProjectSwitcher
        name={repoName(session.repo)}
        path={session.repo}
        recent={recent.list}
        {host}
        canPick={hasNativeDialogs}
      />
    {/if}
  {/snippet}
  {#snippet changesetSwitcher()}
    {@const target = changeset.facts?.target}
    <ChangesetSwitcher
      label={target && targetLabel(target)}
      draft={target?.draft}
      title={target?.branch && `On ${target.branch}`}
      bind:open={switcherOpen}
    />
  {/snippet}
  <Header
    {screen}
    project={projectSwitcher}
    changesets={changesetSwitcher}
    baseNote={note}
    {scope}
    onscope={versions.items.length ? () => void focusTimeline() : undefined}
    onwhole={() => void changeset.showWhole()}
    filtered={changeset.baseChanged ? baseChangedActions : undefined}
    checks={headChecks}
    outgoing={outgoingComments}
    review={changeset.facts?.review}
    completion={changeset.shownCommit === undefined
      ? changeset.viewCompletion
      : (changeset.facts?.completion ?? changeset.viewCompletion)}
    refreshing={queue.refreshing}
  />

  <Banners
    degraded={changeset.layout?.degraded}
    failures={changeset.restoreFailures}
    offBase={changeset.facts?.base.headOffBase ? changeset.facts.base.commit.shortId : undefined}
    pullRequest={changeset.facts?.trust.pullRequest}
    ontrust={() => void changeset.trustPullRequest()}
  />

  <div class="body">
    {#if screen === "queue"}
      <QueueScreen
        onopen={() => (screen = "review")}
        onopenthread={openThread}
        onopenlog={(run) => {
          screen = "review";
          void controller?.openLog(run, logKey(run, undefined));
        }}
      />
    {/if}
    <div class="review" class:away={screen !== "review"} inert={screen !== "review"}>
      <ToolRail
        active={left.tool}
        open={left.open}
        badges={{
          comments: { count: unreadComments, text: `${unreadComments} new` },
          problems: { count: problems.summary?.badge ?? 0, text: problems.summary?.badgeText ?? "" },
          pipelines: pipelineBadge,
        }}
        bottom={bottomTool ?? null}
      />
      <ReviewScreen
        backend={session.backend}
        {changeset}
        leftTool={left.tool}
        leftOpen={left.open}
        rightTool={right.tool}
        rightOpen={right.open}
        {softWrap}
        bottomOpen={bottomTool !== undefined}
        onfilter={() => showDialog("settings", "review")}
        ontool={showTool}
        scope={scope?.label ?? "All changes"}
        onready={(c) => void (controller = c)}
        onclose={() => (controller = undefined)}
      >
        {#snippet side(tool)}
          {#if tool === "symbol"}
            <SymbolPanel onopen={openLocation} />
          {:else if tool === "structure"}
            <StructurePanel
              onjump={(symbol, removed) => void controller?.jumpToOutlineSymbol(symbol, removed)}
              oneditor={() => controller?.focusEditor()}
            />
          {/if}
        {/snippet}
        {#snippet bottom({ selected, select })}
          {#if bottomTool === "timeline"}
            <TimelinePanel
              versions={versions.items}
              base={changeset.facts?.base.commit}
              {range}
              byCommit={changeset.shownCommit !== undefined}
              files={rangeFiles}
              {selected}
              onrange={(r) => void changeset.setRange(r)}
              onselect={select}
              onclose={() => (bottomTool = undefined)}
            />
          {:else if bottomTool === "problems"}
            <ProblemsPanel
              openFiles={controller?.openPaths ?? []}
              currentFile={controller?.problemFile}
              onshow={(p, jump) => void controller?.showProblem(p.path, p.line, jump)}
              onfile={(path) => void controller?.revealFileInChanges(path)}
              oneditor={() => controller?.focusEditor()}
              oninstall={() => showDialog("settings", "servers")}
              onclose={() => (bottomTool = undefined)}
            />
          {:else if bottomTool === "find" && session.backend}
            <FindPanel
              backend={session.backend}
              onopen={openPlace}
              onclose={() => (bottomTool = undefined)}
              oneditor={() => controller?.focusEditor()}
            />
          {:else if bottomTool === "pipelines"}
            <PipelinesPanel
              checks={pipelineCommit === undefined ? undefined : checks.of(pipelineCommit)}
              commit={pipelineCommit}
              onlog={(job, run) => void controller?.openLog(job, logKey(job, run))}
              onbrowser={browse}
              onsignin={() => showDialog("settings", "accounts")}
              oneditor={() => controller?.focusEditor()}
              onclose={() => (bottomTool = undefined)}
            />
          {/if}
        {/snippet}
      </ReviewScreen>
      <ToolRail side="right" active={right.tool} open={right.open} />
    </div>
  </div>
{:else if session.backend}
  {#snippet mainMenuButton()}<MainMenuButton />{/snippet}
  <div class="body">
    <WelcomeScreen
      menu={mainMenu.native ? undefined : mainMenuButton}
      recent={recent.list}
      failed={session.openError}
      canPick={hasNativeDialogs}
      {version}
      onforget={(path) => void session.forget(path)}
    />
  </div>
{:else}
  <TitleBar />
  <p class="message">{session.status}</p>
{/if}

<ConnectionNotice link={session.link} canRestart={canRestartBackend} onrestart={() => session.restartBackend()} />

{#if session.notice}
  {#key session.notice}
    <Toast message={session.notice} onclose={() => (session.notice = undefined)} />
  {/key}
{/if}

<NotificationCards />

{#if dialog === "settings"}
  <SettingsDialog
    bind:page={settingsPage}
    {theme}
    {softWrap}
    onsoftwrap={setSoftWrap}
    onclose={() => (dialog = undefined)}
    filter={session.repo ? changeset.facts?.filter : undefined}
    excluded={{ lines: changeset.layout?.excludedLines ?? 0, items: changeset.layout?.excludedItems ?? 0 }}
    onfilter={(f) => changeset.setFilter(f)}
    trusted={session.repo ? changeset.facts?.trust.project : undefined}
    ontrusted={(trusted) => void changeset.setProjectTrusted(trusted)}
  />
{:else if dialog === "about"}
  <AboutDialog onclose={() => (dialog = undefined)} />
{:else if dialog === "finish"}
  <FinishDialog
    flagged={changeset.facts?.completion.flaggedHunks ?? 0}
    ongoto={() => {
      dialog = undefined;
      void goToFirstFlagged();
    }}
    onclose={() => (dialog = undefined)}
    onfinish={() => {
      dialog = undefined;
      void changeset.finishReview();
    }}
  />
{:else if dialog === "pullRequest"}
  <OpenPullRequestDialog
    remotes={queue.sources.map((s) => s.remote)}
    onclose={() => (dialog = undefined)}
    onopen={(remote, number) => {
      dialog = undefined;
      void changeset.openChangeset({ type: "pullRequest", remote, number });
    }}
  />
{/if}

{#if palette.open}
  <Palette
    onchoose={(choice) => void openChoice(choice, choiceHost)}
    onsettings={() => showDialog("settings", "servers")}
  />
{/if}

{#if findInFiles.open && session.backend && session.repo}
  <FindInFilesDialog
    backend={session.backend}
    view={changeset.view}
    files={changeset.facts?.files ?? 0}
    range={changeset.range ?? defaultRange(versions.items)}
    revision={changeset.layout?.shown.revision ?? 0}
    onopen={(result) => {
      screen = "review";
      void controller?.openFound(result);
    }}
    onkeep={(kept) => findSets.keepSearch(kept)}
  />
{/if}

<LocationPicker
  statusOf={(path) => {
    const file = changeset.layoutOf(path);
    return file && statusLetter(file);
  }}
  onsettings={() => showDialog("settings", "servers")}
/>

{#if contextMenus.drawn}
  {@const request = contextMenus.drawn}
  <ContextMenu
    x={request.x}
    y={request.y}
    items={request.items}
    onselect={(id) => request.onselect(id)}
    onclose={() => contextMenus.close()}
  />
{/if}

<StatusFooter
  info={footer}
  onserver={() => showDialog("settings", "servers")}
  onproblems={() => {
    screen = "review";
    bottomTool = "problems";
  }}
>
  {#snippet progress()}<TaskProgress tasks={backgroundTasks.tasks} /><PlatformFooter />{/snippet}
</StatusFooter>

<style>
  .body {
    flex: 1;
    display: flex;
    position: relative;
    min-height: 0;
  }
  .review {
    flex: 1;
    min-width: 0;
    display: flex;
  }
  /* Laid out but not seen, so the editors keep their size and scroll positions behind the queue. */
  .review.away {
    position: absolute;
    inset: 0;
    visibility: hidden;
  }
  .message {
    flex: 1;
    margin: 16px;
    color: var(--text-2);
  }
</style>
