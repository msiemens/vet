<script lang="ts">
  import { onMount } from "svelte";
  import DevTools from "../dev/DevTools.svelte";
  import { hudListeners } from "../dev/hud.svelte";
  import { Header, StatusFooter, TitleBar, ToolRail, mockFooter, mockHeader } from "../features/chrome";
  import { FilterPopover } from "../features/filter";
  import { ReviewScreen, type ReviewController } from "../features/review";
  import { RunButton } from "../features/run";
  import { StartScreen } from "../features/start";
  import { SymbolPanel } from "../features/symbols";
  import { type Theme, applyTheme, storedTheme } from "../lib/theme/theme";
  import Toast from "../lib/ui/Toast.svelte";
  import { changeset } from "../state/changeset.svelte";
  import { canPickFolder, repoName, showWindow } from "../state/host";
  import { recent } from "../state/recent.svelte";

  let controller = $state.raw<ReviewController>();
  // The selected tool of each rail, and whether its sidebar is shown.
  const left = $state({ tool: "files", open: true });
  const right = $state({ tool: "symbol", open: true });
  let tools: DevTools;
  let devOpen = $state(false);
  let status = $state("");
  let theme = $state<Theme>(storedTheme());

  const completion = $derived.by(() => {
    const c = changeset.manifest?.completion;
    if (!c) return { seenPct: 0, oldPct: 0 };
    const total = c.lines + c.items;
    const pct = (n: number) => (total ? (n / total) * 100 : 0);
    return {
      seenPct: pct(c.seenLines + c.acknowledgedItems),
      oldPct: pct(c.oldBaseLines),
      title: `${c.seenLines} of ${c.lines} lines · ${c.acknowledgedItems} of ${c.items} other changes · ${c.oldBaseLines} lines against an old base`,
    };
  });
  const excluded = $derived({
    lines: changeset.manifest?.excludedLines ?? 0,
    items: changeset.manifest?.excludedItems ?? 0,
  });
  const footer = $derived({ ...mockFooter, snapshot: `#${changeset.manifest?.revision ?? 0}` });

  // The controller belongs to the repository's screen.
  $effect(() => {
    if (!changeset.repo) controller = undefined;
  });

  onMount(() => {
    applyTheme(theme);
    showWindow().catch((e: unknown) => {
      console.error(e);
    });
    void changeset.connect();
  });

  function ready(c: ReviewController, firstPaintMs: number) {
    controller = c;
    status = `${changeset.manifest?.name}: ${changeset.files.length} files, ${c.changes.totalHunks} hunks; first hunk painted in ${firstPaintMs.toFixed(0)} ms`;
    void tools.runIfRequested();
  }
</script>

{#if changeset.backend && changeset.repo}
  <Header
    info={{ ...mockHeader, project: repoName(changeset.repo) }}
    {completion}
    onopenrepo={() => void changeset.pickRepo()}
  >
    {#snippet filter()}<FilterPopover {excluded} />{/snippet}
    {#snippet run()}<RunButton />{/snippet}
  </Header>

  <div class="body">
    <ToolRail bind:active={left.tool} bind:open={left.open} />
    <ReviewScreen
      backend={changeset.backend}
      hud={hudListeners}
      leftTool={left.tool}
      leftOpen={left.open}
      rightTool={right.tool}
      rightOpen={right.open}
      onready={ready}
    >
      {#snippet side(tool)}{#if tool === "symbol"}<SymbolPanel />{/if}{/snippet}
    </ReviewScreen>
    <ToolRail side="right" bind:active={right.tool} bind:open={right.open} />
  </div>
{:else if changeset.backend}
  <TitleBar />
  <div class="body">
    <StartScreen
      recent={recent.list}
      failed={changeset.openError}
      canPick={canPickFolder}
      onpick={() => void changeset.pickRepo()}
      onopen={(path) => void changeset.openRepo(path)}
      onforget={(path) => {
        void recent.forget(path);
        changeset.openError = undefined;
      }}
    />
  </div>
{:else}
  <TitleBar />
  <p class="message">{changeset.status}</p>
{/if}

{#if changeset.notice}
  {#key changeset.notice}
    <Toast message={changeset.notice} onclose={() => (changeset.notice = undefined)} />
  {/key}
{/if}

<DevTools
  bind:this={tools}
  {controller}
  bind:open={devOpen}
  bind:status
  {theme}
  ontheme={(t) => applyTheme((theme = t))}
/>

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
