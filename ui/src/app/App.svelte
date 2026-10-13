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
    lensOnSelect,
    toggleTool,
    toolFor,
  } from "../features/chrome";
  import { ChecksButton } from "../features/checks";
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
  import { NotificationCards, PlatformFooter, copyText } from "../features/platform";
  import { ChangesetSwitcher, OpenPullRequestDialog, QueueScreen, queueCommands, targetLabel } from "../features/queue";
  import {
    ON_SCREEN,
    ReviewScreen,
    type ReviewController,
    fileCommands,
    reviewActions,
    modeChange,
    statusLetter,
    viewCommands,
  } from "../features/review";
  import { SettingsDialog } from "../features/settings";
  import { StructurePanel } from "../features/structure";
  import { type OpenLocation, SymbolPanel } from "../features/symbols";
  import { type RangeFile, RangeChips, TimelinePanel } from "../features/timeline";
  import { ProjectSwitcher, recentCommands, WelcomeScreen } from "../features/welcome";
  import type { Thread } from "../generated/Thread";
  import { errorText } from "../lib/errorText";
  import { repoName } from "../lib/paths";
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
  import { keymap } from "../state/keymap.svelte";
  import { mainMenu } from "../state/mainMenu.svelte";
  import { platform } from "../state/platform.svelte";
  import { progress as backgroundTasks } from "../state/progress.svelte";
  import { queue } from "../state/queue.svelte";
  import { recent } from "../state/recent.svelte";
  import { semantic } from "../state/semantic.svelte";
  import { serverList } from "../state/serverList.svelte";
  import { serverLine } from "../state/serverStatus";
  import { session } from "../state/session.svelte";
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
  let timelineOpen = $state(false);
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

  const semanticSummary = $derived(semantic.summary(controller?.activePath));
  const range = $derived(changeset.range);
  const rangeFiles = $derived<RangeFile[]>(
    range ? (changeset.layout?.files ?? []).map((f) => ({ path: f.path, delta: f.delta, mode: modeChange(f) })) : [],
  );
  const footer = $derived({
    range: range && `v${range.from} → v${range.to}`,
    commit:
      changeset.shownCommit === undefined ? undefined : `${changeset.shownCommit + 1} / ${changeset.commits.length}`,
    languageServer: serverLine(controller?.activePath, serverList.servers, backgroundTasks.tasks, semanticSummary),
    diagnostics: semanticSummary.diagnostics,
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
      wholeChangeset: () => void changeset.setRange(undefined),
      refresh: () => void queue.refresh(),
      reconfirmAll: () => void changeset.reconfirmAll(),
      openChangeset: (key) => void changeset.openChangeset(key),
      showScreen: (s) => (screen = s),
      showChangesets: () => {
        screen = "review";
        switcherOpen = true;
      },
      openPullRequest: () => void openPullRequest(),
      openInBrowser: (at) => {
        const url = at ?? platform.info?.url;
        if (url === undefined) return;
        void openUrl(url).catch((e: unknown) => {
          session.notice = `Could not open the browser: ${errorText(e)}`;
        });
      },
      showDialog: (d) => showDialog(d),
      showKeymap: () => showDialog("settings", "keymap"),
      showAccounts: () => showDialog("settings", "accounts"),
      quit: () => void quit(),
      toggleSoftWrap: () => setSoftWrap(softWrap === "never" ? "prose" : "never"),
      toggleTool: (id) => {
        if (id === "timeline") {
          toggleTimeline();
          return;
        }
        const side = TOOLS.left.some((t) => t.id === id) ? left : right;
        Object.assign(side, toggleTool(side, id));
      },
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
    ...viewCommands(reviewActions),
    ...fileCommands(reviewActions),
    ...threadCommands({
      attach: (thread, at) => void threads.attach(thread, at),
      remove: (thread) => void threads.remove(thread),
    }),
  );
  keymap.load();
  session.onOpen(() => (screen = "queue"));
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
    canQuit,
    changesetOpen: !!session.repo && !!changeset.layout,
    rangeShown: !!range,
    reconfirmable: baseChangedLeft > 0,
    pullRequest: platform.info !== undefined,
    outgoing,
    voteSeparate: !!platform.traits?.verdictSeparate,
    voteBlock,
    manyCommits: commitCount > 1,
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
        { id: "timeline", shown: timelineOpen },
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
    const stopMenu = mainMenu.listen();
    return () => {
      stopMenu();
      window.removeEventListener("keydown", onkeydown, true);
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

  /** Opening the timeline shows the changes since the last review. */
  function toggleTimeline() {
    timelineOpen = !timelineOpen;
    if (timelineOpen) void changeset.setRange(defaultRange(versions.items));
  }

  /** Shows the review at a file. */
  function openFile(path: string) {
    screen = "review";
    void controller?.revealInChanges(path);
  }

  /** Shows a thread where it is: at its code, or in the Comments tool window's Conversation tab. */
  function openThread(thread: Thread) {
    screen = "review";
    if (thread.kind !== "changeset") return void controller?.goToThread(thread);
    Object.assign(left, { tool: "comments", open: true });
    platform.commentsTab = "conversation";
  }

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
  {#snippet rangeChips()}
    {#if range}
      <RangeChips {range} versions={versions.items} onrange={(r) => void changeset.setRange(r)} />
    {/if}
  {/snippet}
  {#snippet backToChangeset()}
    {@const base = changeset.base?.name}
    {@const latest = versions.items.at(-1)?.n}
    <button
      class="ctl back"
      type="button"
      title={commands.hint("changeset.whole", "Show the whole changeset again")}
      disabled={!commands.isEnabled("changeset.whole")}
      onclick={() => commands.run("changeset.whole")}
      >Back to {base ?? "…"}{latest === undefined ? "" : ` → v${latest}`}</button
    >
  {/snippet}
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
    {@const head = checks.of(headId)}
    {#if head && headId}
      <ChecksButton checks={head} shortId={headId.slice(0, 7)} onlog={(run) => void controller?.openLog(run)} />
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
    chips={range ? rangeChips : undefined}
    back={backToChangeset}
    filtered={changeset.baseChanged ? baseChangedActions : undefined}
    checks={headChecks}
    outgoing={outgoingComments}
    base={changeset.base}
    target={changeset.facts?.target}
    review={changeset.facts?.review}
    loadBase={() => changeset.baseCandidates()}
    onbase={(choice) => void changeset.setBase(choice)}
    completion={changeset.shownCommit === undefined
      ? changeset.viewCompletion
      : (changeset.facts?.completion ?? changeset.viewCompletion)}
    refreshing={queue.refreshing}
  />

  <Banners
    degraded={changeset.layout?.degraded}
    failures={changeset.restoreFailures}
    pullRequest={changeset.facts?.trust.pullRequest}
    ontrust={() => void changeset.trustPullRequest()}
  />

  <div class="body">
    {#if screen === "queue"}
      <QueueScreen onopen={() => (screen = "review")} onopenfile={openFile} onopenthread={openThread} />
    {/if}
    <div class="review" class:away={screen !== "review"} inert={screen !== "review"}>
      <ToolRail active={left.tool} open={left.open} counts={{ comments: unreadComments }} timeline={timelineOpen} />
      <ReviewScreen
        backend={session.backend}
        {changeset}
        leftTool={left.tool}
        leftOpen={left.open}
        rightTool={right.tool}
        rightOpen={right.open}
        {softWrap}
        onservers={() => showDialog("settings", "servers")}
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
              onsettings={() => showDialog("settings", "servers")}
            />
          {/if}
        {/snippet}
        {#snippet bottom({ selected, select })}
          {#if timelineOpen}
            <TimelinePanel
              versions={versions.items}
              {range}
              files={rangeFiles}
              {selected}
              onrange={(r) => void changeset.setRange(r)}
              onselect={select}
              onclose={() => (timelineOpen = false)}
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

<StatusFooter info={footer} onserver={() => showDialog("settings", "servers")}>
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
