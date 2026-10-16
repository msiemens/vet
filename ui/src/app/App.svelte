<script lang="ts">
  import { onMount, untrack } from "svelte";
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
    focusPipelines,
    focusTimeline,
    headerFacts,
    settleCommits,
    shell,
  } from "../features/chrome";
  import { ChecksButton, headChecksLook, pipelineBadge } from "../features/checks";
  import { LocationPicker } from "../features/locations";
  import { OutgoingButton } from "../features/comments";
  import { Palette, openChoice, palette } from "../features/palette";
  import { NotificationCards, PlatformFooter } from "../features/platform";
  import { FindPanel } from "../features/find";
  import { PipelinesPanel, logKey } from "../features/pipelines";
  import { ProblemsPanel } from "../features/problems";
  import { ChangesetSwitcher, OpenPullRequestDialog, QueueScreen, targetLabel } from "../features/queue";
  import { ON_SCREEN, ReviewScreen, modeChange } from "../features/review";
  import { FindInFilesDialog, findInFiles } from "../features/search";
  import { SettingsDialog } from "../features/settings";
  import { StructurePanel } from "../features/structure";
  import { type OpenLocation, SymbolPanel } from "../features/symbols";
  import { type RangeFile, TimelinePanel } from "../features/timeline";
  import { ProjectSwitcher, WelcomeScreen } from "../features/welcome";
  import { letterOfStatus } from "../lib/changeLetter";
  import { repoName } from "../lib/paths";
  import type { SoftWrap } from "../lib/softWrap";
  import { applyTheme } from "../lib/theme/theme";
  import ContextMenu from "../lib/ui/ContextMenu.svelte";
  import Toast from "../lib/ui/Toast.svelte";
  import { changeGate } from "../lib/ui/changeGate";
  import { changeset } from "../state/changeset.svelte";
  import { checks } from "../state/checks.svelte";
  import { canRestartBackend, hasNativeDialogs, hasNativeMenu } from "../state/host";
  import { commands } from "../state/commands";
  import { contextMenus } from "../state/contextMenu.svelte";
  import { findSets } from "../state/findSets.svelte";
  import { mainMenu } from "../state/mainMenu.svelte";
  import { notices } from "../state/notices.svelte";
  import { platform } from "../state/platform.svelte";
  import { problems } from "../state/problems.svelte";
  import { progress as backgroundTasks } from "../state/progress.svelte";
  import { queue } from "../state/queue.svelte";
  import { recent } from "../state/recent.svelte";
  import { semantic } from "../state/semantic.svelte";
  import { servedBy } from "../state/servedBy.svelte";
  import { serverList } from "../state/serverList.svelte";
  import { serverLine } from "../state/serverStatus";
  import { session } from "../state/session.svelte";
  import { threads } from "../state/threads.svelte";
  import { defaultRange } from "../state/timelineRange";
  import { versions } from "../state/versions.svelte";
  import { softWrapPreference, themePreference } from "./appPreferences.svelte";
  import { appState } from "./appState.svelte";
  import { choiceHost, goToFirstFlagged, openCheckLog, openPlace, openThread, showInBrowser } from "./actions";
  import { registerCommands } from "./commands";
  import { installWindow } from "./window";
  import ConnectionNotice from "./ConnectionNotice.svelte";

  const theme = $derived(themePreference.value);
  const softWrap = $derived(softWrapPreference.value);
  $effect(() => applyTheme(theme));
  // The viewer's choice, here or in another window, rebuilds the views.
  $effect(() => {
    const wrap = softWrap;
    untrack(() => appState.controller?.setSoftWrap(wrap));
  });

  // The Commits tool and the review lens are one choice seen from two places: whichever side moved
  // decides both, so picking the tool asks for the matching lens and a lens set elsewhere selects
  // the tool that goes with it. The pair settled on last is the single source, so neither chases the
  // other.
  let settledCommits = untrack(() => ({
    tool: shell.left.tool,
    lens: changeset.lens,
    commits: changeset.commitCount,
  }));
  $effect(() => {
    const settled = settleCommits(settledCommits, {
      tool: shell.left.tool,
      lens: changeset.lens,
      commits: changeset.commitCount,
    });
    settledCommits = settled;
    untrack(() => {
      if (settled.askLens !== undefined) {
        if (changeset.facts && changeset.lens !== settled.askLens) void changeset.showLens(settled.askLens);
      } else if (settled.tool !== shell.left.tool) {
        shell.left.tool = settled.tool;
      }
    });
  });

  const headLook = $derived(
    headChecksLook(
      checks.of(changeset.headCommit),
      platform.status,
      platform.signedOut,
      checks.limitedUntil,
      checks.loadedAt,
    ),
  );
  const pipelineRail = $derived(pipelineBadge(checks.of(changeset.headCommit), platform.signedOut));

  /** The Comments tool window is on screen. */
  const commentsShown = $derived(shell.screen === "review" && shell.left.open && shell.left.tool === "comments");
  // While the tool window is open, what comes in is seen: the backend keeps the time the viewer
  // last looked and answers the counts (D96). The gate makes the effect run again when new comments
  // arrive while it is open, so the backend moves the seen time past them.
  const commentsChanged = changeGate();
  $effect(() => {
    if (commentsChanged(commentsShown, platform.info, threads.items) && commentsShown) {
      untrack(() => void platform.markCommentsSeen());
    }
  });
  /** The rail's count for Comments: what the backend says the viewer has not read of the pull request as a whole. */
  const unreadComments = $derived(commentsShown ? 0 : platform.unreadCount);

  const semanticSummary = $derived(semantic.summary(appState.controller?.semanticPath));
  const range = $derived(changeset.range);
  const rangeFiles = $derived<RangeFile[]>(
    range ? (changeset.layout?.files ?? []).map((f) => ({ path: f.path, delta: f.delta, mode: modeChange(f) })) : [],
  );
  const footer = $derived({
    commit: changeset.shownCommit === undefined ? undefined : `${changeset.shownCommit + 1} / ${changeset.commitCount}`,
    languageServer: serverLine(
      appState.controller?.semanticPath,
      appState.controller?.semanticPath === undefined ? undefined : servedBy.of(appState.controller.semanticPath),
      serverList.servers,
      serverList.runtimes,
      semanticSummary,
    ),
    problems: problems.summary,
  });
  // The changed files the window has open, so the backend's background analysis reads them first.
  $effect(() => {
    const backend = session.backend;
    const paths = [...(appState.controller?.openPaths ?? [])];
    if (backend) untrack(() => void backend.setOpenFiles(paths).catch(() => undefined));
  });

  const openLocation: OpenLocation = (path, line, side) =>
    void appState.controller?.openLocation(ON_SCREEN, path, line, side);

  // Registered before the first render, so the menu below is built from them.
  const stopCommands = registerCommands();

  // The main menu follows the state it shows.
  $effect(() => {
    const shown = (side: { open: boolean; tool: string }, id: string) => side.open && side.tool === id;
    const traits = platform.traits;
    mainMenu.update({
      native: hasNativeMenu,
      recent: recent.list.map(({ path }) => ({ path, name: repoName(path) })),
      screen: shell.screen,
      viewMode: appState.controller?.activeMode,
      softWrap: softWrap !== "never",
      theme,
      tools: [
        ...TOOLS.left.map((t) => ({ id: t.id, shown: shown(shell.left, t.id) })),
        ...TOOLS.right.map((t) => ({ id: t.id, shown: shown(shell.right, t.id) })),
        ...TOOLS.bottom.map((t) => ({ id: t.id, shown: shell.bottom === t.id })),
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
    const stopWindow = installWindow();
    return () => {
      stopWindow();
      stopCommands();
    };
  });

  /** Shows a tool window: the Timeline under the editors, or one of the left rail's. */
  function showTool(tool: "changes" | "all" | "comments" | "timeline") {
    if (tool === "timeline") shell.showBottom("timeline");
    else shell.showLeft(tool);
  }

  function setSoftWrap(next: SoftWrap) {
    softWrapPreference.value = next;
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
    {#if headLook}
      <ChecksButton
        checks={headLook.head}
        title={headLook.title}
        stale={headLook.stale}
        warn={platform.signedOut}
        onopen={() => void focusPipelines()}
      />
    {/if}
  {/snippet}
  {#snippet outgoingComments()}
    <!-- A changeset that is no pull request has nowhere to post to. -->
    {#if changeset.headCommit !== undefined}<OutgoingButton onopen={openThread} />{/if}
  {/snippet}
  {#snippet projectSwitcher()}
    {#if session.repo}
      <ProjectSwitcher
        name={repoName(session.repo)}
        path={session.repo}
        recent={recent.list}
        host={appState.host}
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
      bind:open={appState.switcherOpen}
    />
  {/snippet}
  <Header
    screen={shell.screen}
    project={projectSwitcher}
    changesets={changesetSwitcher}
    baseNote={headerFacts.note}
    scope={headerFacts.scope}
    onscope={versions.items.length ? () => void focusTimeline() : undefined}
    onwhole={() => void changeset.showWhole()}
    filtered={changeset.baseChanged ? baseChangedActions : undefined}
    checks={headChecks}
    outgoing={outgoingComments}
    review={changeset.facts?.review}
    completion={changeset.shownCompletion}
    refreshing={queue.refreshing}
  />

  <Banners
    degraded={changeset.layout?.degraded}
    failures={changeset.restoreFailures}
    offBase={changeset.facts?.base.headOffBase ? changeset.facts.base.commit.shortId : undefined}
    pullRequest={changeset.facts?.trust.pullRequest}
    ontrust={() => void changeset.trustPullRequest()}
    ontrustproject={() => appState.showDialog("settings", "trust")}
  />

  <div class="body">
    {#if shell.screen === "queue"}
      <QueueScreen onopen={() => shell.show("review")} onopenthread={openThread} onopenlog={openCheckLog} />
    {:else}
      <div class="review">
        <ToolRail
          active={shell.left.tool}
          open={shell.left.open}
          badges={{
            comments: { count: unreadComments, text: `${unreadComments} new` },
            problems: { count: problems.summary?.badge ?? 0, text: problems.summary?.badgeText ?? "" },
            pipelines: pipelineRail,
          }}
          bottom={shell.bottom ?? null}
        />
        <ReviewScreen
          backend={session.backend}
          {changeset}
          leftTool={shell.left.tool}
          leftOpen={shell.left.open}
          rightTool={shell.right.tool}
          rightOpen={shell.right.open}
          {softWrap}
          bottomOpen={shell.bottom !== undefined}
          onfilter={() => appState.showDialog("settings", "filter")}
          ontool={showTool}
          scope={headerFacts.scope?.label ?? "All changes"}
          onready={(c) => appState.setController(c)}
          onclose={() => appState.clearController()}
        >
          {#snippet side(tool)}
            {#if tool === "symbol"}
              <SymbolPanel onopen={openLocation} />
            {:else if tool === "structure"}
              <StructurePanel
                onjump={(symbol, removed) => void appState.controller?.jumpToOutlineSymbol(symbol, removed)}
                oneditor={() => appState.controller?.focusEditor()}
              />
            {/if}
          {/snippet}
          {#snippet bottom({ selected, select })}
            {#if shell.bottom === "timeline"}
              <TimelinePanel
                versions={versions.items}
                base={changeset.facts?.base.commit}
                {range}
                byCommit={changeset.shownCommit !== undefined}
                files={rangeFiles}
                {selected}
                onrange={(r) => void changeset.setRange(r)}
                onselect={select}
                onclose={() => shell.hideBottom()}
              />
            {:else if shell.bottom === "problems"}
              <ProblemsPanel
                openFiles={appState.controller?.openPaths ?? []}
                currentFile={appState.controller?.problemFile}
                onshow={(p, jump) => void appState.controller?.showProblem(p.path, p.line, jump)}
                onfile={(path) => void appState.controller?.revealFileInChanges(path)}
                oneditor={() => appState.controller?.focusEditor()}
                oninstall={() => appState.showDialog("settings", "servers")}
                onclose={() => shell.hideBottom()}
              />
            {:else if shell.bottom === "find" && session.backend}
              <FindPanel
                backend={session.backend}
                onopen={openPlace}
                onclose={() => shell.hideBottom()}
                oneditor={() => appState.controller?.focusEditor()}
              />
            {:else if shell.bottom === "pipelines"}
              <PipelinesPanel
                checks={changeset.pipelineCommit === undefined ? undefined : checks.of(changeset.pipelineCommit)}
                commit={changeset.pipelineCommit}
                onlog={(job, run) => void appState.controller?.openLog(job, logKey(job, run))}
                onbrowser={showInBrowser}
                onsignin={() => appState.showDialog("settings", "accounts")}
                oneditor={() => appState.controller?.focusEditor()}
                onclose={() => shell.hideBottom()}
              />
            {/if}
          {/snippet}
        </ReviewScreen>
        <ToolRail side="right" active={shell.right.tool} open={shell.right.open} />
      </div>
    {/if}
  </div>
{:else if session.backend}
  {#snippet mainMenuButton()}<MainMenuButton />{/snippet}
  <div class="body">
    <WelcomeScreen
      menu={mainMenu.native ? undefined : mainMenuButton}
      recent={recent.list}
      failed={session.openError}
      canPick={hasNativeDialogs}
      version={appState.version}
      onforget={(path) => void session.forget(path)}
    />
  </div>
{:else}
  <TitleBar />
  <p class="message">{session.status}</p>
{/if}

<ConnectionNotice link={session.link} canRestart={canRestartBackend} onrestart={() => session.restartBackend()} />

{#if notices.text}
  {#key notices.text}
    <Toast message={notices.text} onclose={() => notices.close()} />
  {/key}
{/if}

<NotificationCards />

{#if appState.dialog === "settings"}
  <SettingsDialog
    bind:page={appState.settingsPage}
    {theme}
    {softWrap}
    onsoftwrap={setSoftWrap}
    onclose={() => appState.closeDialog()}
    project={session.repo ? repoName(session.repo) : undefined}
    filter={session.repo ? changeset.facts?.filter : undefined}
    excluded={{ lines: changeset.layout?.excludedLines ?? 0, items: changeset.layout?.excludedItems ?? 0 }}
    onfilter={(f) => changeset.setFilter(f)}
    trusted={session.repo ? changeset.facts?.trust.project : undefined}
    ontrusted={(trusted) => void changeset.setProjectTrusted(trusted)}
  />
{:else if appState.dialog === "about"}
  <AboutDialog onclose={() => appState.closeDialog()} />
{:else if appState.dialog === "finish"}
  <FinishDialog
    flagged={changeset.facts?.completion.flaggedHunks ?? 0}
    ongoto={() => {
      appState.closeDialog();
      void goToFirstFlagged();
    }}
    onclose={() => appState.closeDialog()}
    onfinish={() => {
      appState.closeDialog();
      void changeset.finishReview();
    }}
  />
{:else if appState.dialog === "pullRequest"}
  <OpenPullRequestDialog
    remotes={queue.sources.map((s) => s.remote)}
    onclose={() => appState.closeDialog()}
    onopen={(remote, number) => {
      appState.closeDialog();
      void changeset.openChangeset({ type: "pullRequest", remote, number });
    }}
  />
{/if}

{#if palette.open}
  <Palette
    onchoose={(choice) => void openChoice(choice, choiceHost)}
    onsettings={() => appState.showDialog("settings", "servers")}
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
      shell.show("review");
      appState.whenReview((c) => void c.openFound(result));
    }}
    onkeep={(kept) => findSets.keepSearch(kept)}
  />
{/if}

<LocationPicker
  statusOf={(path) => {
    const file = changeset.layoutOf(path);
    return file && letterOfStatus(file.status);
  }}
  onsettings={() => appState.showDialog("settings", "servers")}
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
  onserver={() => appState.showDialog("settings", "servers")}
  onproblems={() => {
    shell.show("review");
    shell.showBottom("problems");
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
  .message {
    flex: 1;
    margin: 16px;
    color: var(--text-2);
  }
</style>
