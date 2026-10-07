<script lang="ts">
  import { onMount, untrack } from "svelte";
  import DevTools from "../dev/DevTools.svelte";
  import { probe } from "../dev/probe.svelte";
  import type { WatchDegraded } from "../generated/WatchDegraded";
  import Banner from "../lib/ui/Banner.svelte";
  import {
    AboutDialog,
    FinishDialog,
    Header,
    degradedMessage,
    limitedMessage,
    StatusFooter,
    TOOLS,
    TitleBar,
    ToolRail,
    toggleTool,
  } from "../features/chrome";
  import { ReviewScreen, type ReviewController } from "../features/review";
  import { SettingsDialog } from "../features/settings";
  import { StartScreen } from "../features/start";
  import { type OpenLocation, SymbolPanel, symbols } from "../features/symbols";
  import { percentages } from "../lib/completion";
  import { KEYS, matches } from "../lib/keymap";
  import { repoName } from "../lib/paths";
  import { type Theme, applyTheme, storedTheme } from "../lib/theme/theme";
  import Toast from "../lib/ui/Toast.svelte";
  import type { Mode } from "../lib/viewMode";
  import { changeset } from "../state/changeset.svelte";
  import { canQuit, hasNativeDialogs, hasNativeMenu, quit, showWindow } from "../state/host";
  import type { Command } from "../state/mainMenu";
  import { mainMenu } from "../state/mainMenu.svelte";
  import { recent } from "../state/recent.svelte";
  import { semantic } from "../state/semantic.svelte";
  import { session } from "../state/session.svelte";

  let controller = $state.raw<ReviewController>();
  // The selected tool of each rail, and whether its sidebar is shown.
  const left = $state({ tool: "files", open: true });
  const right = $state({ tool: "symbol", open: true });
  let tools: DevTools;
  let devOpen = $state(false);
  let status = $state("");
  let theme = $state<Theme>(storedTheme());
  let mode = $state<Mode>("unified");
  let dialog = $state<"settings" | "about" | "finish">();
  const reviewOpen = $derived(changeset.manifest?.review.state === "open");

  const completion = $derived.by(() => {
    const c = changeset.manifest?.completion;
    if (!c) return { seenPct: 0, oldPct: 0 };
    return {
      ...percentages(c),
      title: `${c.seenLines} of ${c.lines} lines · ${c.acknowledgedItems} of ${c.items} other changes · ${c.oldBaseLines} lines against an old base`,
    };
  });
  const degraded = $derived(changeset.manifest?.degraded);
  let dismissedDegraded = $state<WatchDegraded>();
  const failures = $derived(changeset.restoreFailures);
  // How many failures the viewer dismissed: a longer list is news again, and a shorter one
  // (a project got fixed) lowers it so a failure that comes back is shown.
  let dismissedFailures = $state(0);
  $effect(() => {
    if (failures.length < untrack(() => dismissedFailures)) dismissedFailures = failures.length;
  });
  const semanticSummary = $derived(semantic.summary(controller?.activePath));
  const footer = $derived({
    languageServer: semanticSummary.label,
    languageServerTitle: semanticSummary.title,
    diagnostics: semanticSummary.diagnostics,
  });

  // The controller belongs to the repository's screen.
  $effect(() => {
    if (!session.repo) {
      controller = undefined;
      symbols.reset();
    }
  });

  // The changed symbols belong to a revision; another one replaces them.
  $effect(() => {
    const backend = session.backend;
    const revision = changeset.manifest?.revision;
    if (backend && revision !== undefined) untrack(() => void symbols.load(backend, revision));
  });

  const openLocation: OpenLocation = (path, line, side) => void controller?.openLocation(path, line, side);

  // The main menu follows the state it shows.
  $effect(() => {
    const shown = (side: typeof left, id: string) => side.open && side.tool === id;
    mainMenu.update({
      native: hasNativeMenu,
      repoOpen: !!session.repo,
      reviewOpen,
      canQuit,
      recent: recent.list.map((path) => ({ path, name: repoName(path) })),
      mode,
      theme,
      tools: [
        ...TOOLS.left.map((t) => ({ id: t.id, label: t.label, shown: shown(left, t.id) })),
        ...TOOLS.right.map((t) => ({ id: t.id, label: t.label, shown: shown(right, t.id) })),
      ],
      devPanel: devOpen,
    });
  });

  onMount(() => {
    applyTheme(theme);
    showWindow().then(
      (frame) => {
        if (frame === "native") console.warn("app-drawn title bar unavailable; the host log says why");
      },
      (e: unknown) => {
        console.error(e);
      },
    );
    void session.connect();
    return mainMenu.listen(execute);
  });

  function execute(command: Command) {
    switch (command.type) {
      case "open":
        void session.pickRepo();
        break;
      case "openRecent":
        void session.openRepo(command.path);
        break;
      case "finishReview":
        if (reviewOpen) dialog = "finish";
        break;
      case "settings":
      case "about":
        dialog = command.type;
        break;
      case "quit":
        void quit();
        break;
      case "mode":
        setMode(command.mode);
        break;
      case "tool": {
        const side = TOOLS.left.some((t) => t.id === command.id) ? left : right;
        Object.assign(side, toggleTool(side, command.id));
        break;
      }
      case "theme":
        applyTheme((theme = command.theme));
        break;
      case "devPanel":
        devOpen = !devOpen;
        break;
    }
  }

  function setMode(next: Mode) {
    mode = next;
    controller?.setMode(next);
  }

  function ready(c: ReviewController, firstPaintMs: number) {
    controller = c;
    status = `${changeset.manifest?.name}: ${changeset.files.length} files, ${c.changes.totalHunks} hunks; first hunk painted in ${firstPaintMs.toFixed(0)} ms`;
    void tools.runIfRequested();
  }
</script>

<svelte:window
  onkeydowncapture={(e) => {
    if (!matches(KEYS.settings.spec, e)) return;
    e.preventDefault();
    dialog = "settings";
  }}
/>

{#if session.backend && session.repo}
  <Header
    project={repoName(session.repo)}
    base={changeset.manifest?.base}
    target={changeset.manifest?.target}
    review={changeset.manifest?.review}
    loadBase={() => changeset.baseCandidates()}
    onbase={(choice) => void changeset.setBase(choice)}
    {completion}
    onfinish={() => (dialog = "finish")}
  />

  {#if degraded && dismissedDegraded !== degraded}
    <Banner message={degradedMessage[degraded]} ondismiss={() => (dismissedDegraded = degraded)} />
  {/if}

  {#if failures.length > dismissedFailures}
    <Banner message={limitedMessage(failures)} ondismiss={() => (dismissedFailures = failures.length)}>
      {#snippet details()}
        <ul class="failures">
          {#each failures as failure (failure.project + failure.tool + failure.message)}
            <li>
              <strong>{failure.project}</strong> ({failure.tool}):
              <pre>{failure.message}</pre>
            </li>
          {/each}
        </ul>
      {/snippet}
    </Banner>
  {/if}

  <div class="body">
    <ToolRail bind:active={left.tool} bind:open={left.open} />
    <ReviewScreen
      backend={session.backend}
      {probe}
      leftTool={left.tool}
      leftOpen={left.open}
      rightTool={right.tool}
      rightOpen={right.open}
      {mode}
      onmode={setMode}
      onready={ready}
      onfile={(path) => {
        symbols.file = path;
      }}
    >
      {#snippet side(tool)}{#if tool === "symbol"}<SymbolPanel onopen={openLocation} />{/if}{/snippet}
    </ReviewScreen>
    <ToolRail side="right" bind:active={right.tool} bind:open={right.open} />
  </div>
{:else if session.backend}
  <TitleBar />
  <div class="body">
    <StartScreen
      recent={recent.list}
      failed={session.openError}
      canPick={hasNativeDialogs}
      onpick={() => void session.pickRepo()}
      onopen={(path) => void session.openRepo(path)}
      onforget={(path) => void session.forget(path)}
    />
  </div>
{:else}
  <TitleBar />
  <p class="message">{session.status}</p>
{/if}

{#if session.notice}
  {#key session.notice}
    <Toast message={session.notice} onclose={() => (session.notice = undefined)} />
  {/key}
{/if}

{#if dialog === "settings"}
  <SettingsDialog
    {theme}
    ontheme={(t) => applyTheme((theme = t))}
    onclose={() => (dialog = undefined)}
    backend={session.repo ? session.backend : undefined}
    project={session.repo ? changeset.manifest : undefined}
    onfilter={(f) => changeset.setFilter(f)}
  />
{:else if dialog === "about"}
  <AboutDialog onclose={() => (dialog = undefined)} />
{:else if dialog === "finish"}
  <FinishDialog
    onclose={() => (dialog = undefined)}
    onfinish={() => {
      dialog = undefined;
      void changeset.finishReview();
    }}
  />
{/if}

<DevTools bind:this={tools} {controller} bind:open={devOpen} bind:status />

<StatusFooter info={footer}>
  {#snippet end()}<button class="flat dev" type="button" aria-pressed={devOpen} onclick={() => (devOpen = !devOpen)}
      >Dev</button
    >{/snippet}
</StatusFooter>

<style>
  .body {
    flex: 1;
    display: flex;
    min-height: 0;
  }
  .failures {
    margin: 0;
    padding-left: 16px;
  }
  .failures pre {
    margin: 2px 0 6px;
    font-family: var(--font-mono);
    font-size: 11px;
    white-space: pre-wrap;
  }
  .message {
    flex: 1;
    margin: 16px;
    color: var(--text-2);
  }
  .dev {
    height: 18px;
    padding: 0 6px;
    border-radius: var(--radius);
    color: var(--text-3);
    font-size: 11px;
  }
</style>
