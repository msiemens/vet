<script lang="ts">
  import type { ReviewFilter } from "../../generated/ReviewFilter";
  import type { SoftWrap } from "../../lib/softWrap";
  import { THEMES, type Theme } from "../../lib/theme/theme";
  import Dialog from "../../lib/ui/Dialog.svelte";
  import GroupHeader from "../../lib/ui/GroupHeader.svelte";
  import ScrollArea from "../../lib/ui/ScrollArea.svelte";
  import Segmented from "../../lib/ui/Segmented.svelte";
  import { commands } from "../../state/commands";
  import AccountsPage from "./AccountsPage.svelte";
  import GeneralPage from "./GeneralPage.svelte";
  import KeymapPage from "./KeymapPage.svelte";
  import ReviewFilterPage from "./ReviewFilterPage.svelte";
  import ServersPage from "./ServersPage.svelte";
  import TrustPage from "./TrustPage.svelte";

  let {
    theme,
    softWrap,
    onsoftwrap,
    onclose,
    project,
    filter,
    excluded,
    onfilter,
    trusted,
    ontrusted,
    page = $bindable("general"),
  }: {
    /** The page shown; the viewer's choice of another is passed back. */
    page?: Page;
    theme: Theme;
    softWrap: SoftWrap;
    onsoftwrap: (wrap: SoftWrap) => void;
    onclose: () => void;
    /** The open project's name; none without an open repository. */
    project?: string;
    /** The open repository's review filter; none until the manifest brought it. */
    filter?: ReviewFilter;
    /** The changed lines and non-text items the filter hides. */
    excluded: { lines: number; items: number };
    /** Saves the repository's review filter; resolves with the backend's message if it refused it. */
    onfilter: (filter: ReviewFilter) => Promise<string | undefined>;
    /** Whether the open repository is marked trusted; none without an open repository. */
    trusted?: boolean;
    /** Marks the repository trusted or not. */
    ontrusted: (trusted: boolean) => void;
  } = $props();

  // A Backends page is not listed: SPEC names no setting of its own for it.
  const APPLICATION = [
    { id: "general", label: "General" },
    { id: "appearance", label: "Appearance" },
    { id: "keymap", label: "Keymap" },
    { id: "servers", label: "Language servers" },
    { id: "accounts", label: "Accounts" },
  ] as const;
  const PROJECT = [
    { id: "trust", label: "Trust" },
    { id: "filter", label: "Review filter" },
  ] as const;
  type Page = (typeof APPLICATION)[number]["id"] | (typeof PROJECT)[number]["id"];

  // Without a project its pages are greyed and cannot be the one shown.
  const shown = $derived<Page>(project === undefined && PROJECT.some((p) => p.id === page) ? "general" : page);
</script>

{#snippet pick(p: { id: Page; label: string }, disabled = false)}
  <button
    class="flat pick"
    class:on={shown === p.id}
    type="button"
    {disabled}
    aria-current={shown === p.id ? "page" : undefined}
    onclick={() => (page = p.id)}>{p.label}</button
  >
{/snippet}

<Dialog title="Settings" {onclose} width="780px" height="min(520px, 76vh)" resizable="settings" padded={false}>
  <div class="layout">
    <nav aria-label="Settings pages">
      <GroupHeader kind="label">Application</GroupHeader>
      {#each APPLICATION as p (p.id)}{@render pick(p)}{/each}
      <GroupHeader kind="label">
        Project
        {#snippet trailing()}<span class="project">{project}</span>{/snippet}
      </GroupHeader>
      {#if project === undefined}<p class="note">Open a project first</p>{/if}
      {#each PROJECT as p (p.id)}{@render pick(p, project === undefined)}{/each}
    </nav>
    <ScrollArea>
      <div class="page">
        {#if shown === "general"}
          <GeneralPage {softWrap} {onsoftwrap} />
        {:else if shown === "keymap"}
          <KeymapPage />
        {:else if shown === "trust"}
          {#if trusted !== undefined}<TrustPage {trusted} {ontrusted} />{/if}
        {:else if shown === "filter"}
          {#if filter}<ReviewFilterPage {filter} {excluded} onsave={onfilter} />{/if}
        {:else if shown === "servers"}
          <ServersPage />
        {:else if page === "accounts"}
          <AccountsPage />
        {:else}
          <div class="row">
            <span>Theme</span>
            <Segmented
              label="Theme"
              value={theme}
              onchange={(id) => commands.run(`view.theme:${id}`)}
              options={THEMES}
            />
          </div>
        {/if}
      </div>
    </ScrollArea>
  </div>
</Dialog>

<style>
  .layout {
    display: flex;
    height: 100%;
  }
  nav {
    flex: none;
    width: 190px;
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 10px;
    overflow: auto;
    border-right: 1px solid var(--line-soft);
    background: var(--bg-panel);
  }
  nav button {
    height: 26px;
    padding: 0 10px;
    border-radius: var(--radius);
    font-size: var(--fs-ctl);
    text-align: left;
  }
  nav .note {
    padding: 0 10px 4px;
    font-size: 12px;
  }
  .project {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    font: 400 var(--fs-mono-s) var(--font-mono);
    color: var(--text-2);
  }
  /* Less on the right, where the scroll bar's gutter adds its own. */
  .page {
    padding: 16px calc(16px - var(--w-scrollbar)) 16px 16px;
  }
  .row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    font-size: var(--fs-ctl);
  }
</style>
