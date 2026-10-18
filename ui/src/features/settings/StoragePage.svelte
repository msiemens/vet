<script lang="ts">
  import { onMount } from "svelte";
  import { formatBytes } from "../../lib/formatBytes";
  import { repoName } from "../../lib/paths";
  import Facts from "../../lib/ui/Facts.svelte";
  import PathText from "../../lib/ui/PathText.svelte";
  import type { Column } from "../../lib/ui/columns";
  import { backendSettings } from "../../state/backendSettings.svelte";
  import { commands } from "../../state/commands";
  import { removeProject } from "../../state/removalPrompt.svelte";
  import { storage } from "../../state/storage.svelte";
  import SettingsTable from "./SettingsTable.svelte";
  import { largestFirst, projectTotal } from "./storageRows";

  const COLUMNS: Column[] = [
    { id: "project", label: "Project", flex: true, min: 120 },
    { id: "size", label: "Size", width: 90, min: 70, align: "right" },
    { id: "actions", label: "", width: 150, fixed: true },
  ];

  const projects = $derived(storage.all && largestFirst(storage.all.projects));
  let cacheError = $state<string>();

  onMount(() => {
    void backendSettings.load();
    return storage.watch("all");
  });

  /** A whole number of MiB, at least one; anything else puts the kept budget back in the field. */
  async function setBudget(input: HTMLInputElement) {
    const mb = Number(input.value);
    const kept = backendSettings.settings?.semanticCacheMb;
    if (!Number.isInteger(mb) || mb < 1) {
      input.value = String(kept ?? "");
      cacheError = "The budget is a whole number of MiB, at least 1.";
      return;
    }
    if (mb === kept) return;
    cacheError = await backendSettings.change({ semanticCacheMb: mb });
  }
</script>

<div class="page">
  <Facts>
    {#if storage.all}
      <dt>Semantic cache</dt>
      <dd>{formatBytes(storage.all.app.semanticCache)}</dd>
      <dt>Language servers</dt>
      <dd>{formatBytes(storage.all.app.languageServers)}</dd>
      <dt>Databases</dt>
      <dd>{formatBytes(storage.all.app.databases)}</dd>
    {/if}
    <dt><label for="cache-budget">Keep up to</label></dt>
    <dd class="budget">
      <input
        id="cache-budget"
        class="field"
        type="number"
        min="1"
        step="1"
        disabled={!backendSettings.settings}
        value={backendSettings.settings?.semanticCacheMb ?? ""}
        onchange={(e) => void setBudget(e.currentTarget)}
      />
      <span>MiB</span>
    </dd>
    {#if backendSettings.loadError}
      <dt></dt>
      <dd class="settings-problem">{backendSettings.loadError}</dd>
    {/if}
    {#if cacheError}
      <dt></dt>
      <dd class="settings-problem">{cacheError}</dd>
    {/if}
    <dt></dt>
    <dd>
      <button
        class="ctl"
        type="button"
        title={commands.hint("semantic.clearCaches")}
        disabled={backendSettings.clearing || !commands.isEnabled("semantic.clearCaches")}
        onclick={() => commands.run("semantic.clearCaches")}>{commands.label("semantic.clearCaches")}</button
      >
    </dd>
  </Facts>

  {#if storage.loadError}
    <p class="settings-problem">{storage.loadError}</p>
  {:else if !projects}
    <p class="empty">Measuring disk usage…</p>
  {:else if !projects.length}
    <p class="empty">No projects yet</p>
  {:else}
    <SettingsTable name="storage" columns={COLUMNS} row="var(--h-switcher-row)" aria-label="Projects">
      {#each projects as project (project.root)}
        <div class="row">
          <span class="text">
            <span class="name">
              <span class="clip" title={project.root}>{repoName(project.root)}</span>
              {#if !project.folderExists}<span class="badge">Folder missing</span>{/if}
            </span>
            <span class="mono path"><PathText path={project.root} /></span>
          </span>
          <span class="size">{formatBytes(projectTotal(project))}</span>
          <span class="actions">
            <button
              class="ctl small"
              type="button"
              disabled={storage.busy === project.root}
              onclick={() => void storage.clear(project.root)}>Clear</button
            >
            <button class="ctl small" type="button" onclick={() => void removeProject(project.root)}>Remove…</button>
          </span>
        </div>
      {/each}
    </SettingsTable>
  {/if}
</div>

<style>
  .page {
    --w-facts-label: 120px;
    display: flex;
    flex-direction: column;
    gap: 16px;
    font-size: var(--fs-ctl);
  }
  p {
    margin: 0;
  }
  .empty {
    color: var(--text-3);
  }
  .budget {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .budget input {
    width: 80px;
  }
  .text {
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .name {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .clip {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .path {
    display: flex;
    font-size: var(--fs-mono-s);
  }
  .size {
    text-align: right;
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 6px;
  }
</style>
