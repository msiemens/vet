<script lang="ts">
  import { onMount } from "svelte";
  import { formatBytes } from "../../lib/formatBytes";
  import { match } from "../../lib/speedSearch";
  import Marked from "../../lib/ui/Marked.svelte";
  import ScrollArea from "../../lib/ui/ScrollArea.svelte";
  import { SpeedSearch } from "../../lib/ui/speedSearch.svelte";
  import SpeedSearchField from "../../lib/ui/SpeedSearchField.svelte";
  import Table from "../../lib/ui/Table.svelte";
  import type { Column } from "../../lib/ui/columns";
  import { type Sort, sortRows } from "../../lib/ui/sorting";
  import { backendSettings } from "../../state/backendSettings.svelte";
  import { servers } from "../../state/servers.svelte";
  import ServerPathSheet from "./ServerPathSheet.svelte";
  import { serverRow, type ServerRow } from "./serverRows";

  /** The server whose custom path sheet is open. */
  let editing = $state<ServerRow>();
  /** Why going back to the automatic server failed, with its language. */
  let pathError = $state<{ language: string; message: string }>();

  const COLUMNS: Column<ServerRow>[] = [
    { id: "language", label: "Language", width: 90, min: 60, sortBy: (row) => row.title },
    { id: "server", label: "Server", flex: true, min: 80 },
    { id: "version", label: "Version", width: 80, min: 50 },
    { id: "source", label: "Source", width: 120, min: 60 },
    { id: "status", label: "", width: 28, fixed: true },
  ];

  const search = new SpeedSearch();
  let tableEl = $state<HTMLElement>();
  let sort = $state<Sort>();
  const rows = $derived(
    sortRows(
      servers.servers
        .map((info) => serverRow(info, servers.installs[info.language]))
        .flatMap((row) => {
          const m = match(search.query, row.title);
          return m ? [{ ...row, ranges: m.ranges }] : [];
        }),
      COLUMNS,
      sort,
    ),
  );
  let selectedLanguage = $state<string>();
  const selected = $derived(rows.find((r) => r.language === selectedLanguage) ?? rows.at(0));
  const log = $derived(selected ? (servers.logs[selected.language] ?? []) : []);

  onMount(() => {
    void backendSettings.load();
    return servers.start();
  });

  async function automatic(language: string) {
    const message = await servers.setPath(language, null);
    pathError = message === undefined ? undefined : { language, message };
  }

  /** What the Source select offers besides the source in use; picking one acts and the select shows the source in use again. */
  function chooseSource(row: ServerRow, choice: string) {
    pathError = undefined;
    if (choice === "install") void servers.install(row.language);
    else if (choice === "custom") editing = row;
    else if (choice === "automatic") void automatic(row.language);
  }

  let cacheError = $state<string>();

  async function clearCaches() {
    cacheError = await backendSettings.clearCaches();
  }

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
  {#if !servers.available}
    <p class="empty">Language servers are resolved per repository. Open a repository to see and change them.</p>
  {:else if servers.loadError}
    <p class="problem">{servers.loadError}</p>
  {:else if servers.loading && !servers.servers.length}
    <p class="empty">Looking for language servers…</p>
  {:else}
    {#if search.active}<SpeedSearchField query={search.query} found={rows.length > 0} />{/if}
    <div class="box">
      <Table
        name="servers"
        columns={COLUMNS}
        bind:sort
        scroll={false}
        ruled
        bind:body={tableEl}
        role="listbox"
        aria-label="Language servers"
        tabindex={-1}
        onkeydown={(e) => search.keydown(e, tableEl, "button:enabled")}
        {...search.attributes}
        --table-gap="8px"
        --table-pad="10px"
        --table-pad-end="6px"
        --table-row="30px"
        --table-head="30px"
        --table-head-color="var(--text-3)"
      >
        {#each rows as row (row.language)}
          <button
            class="flat row"
            type="button"
            role="option"
            aria-selected={row === selected}
            onclick={() => (selectedLanguage = row.language)}
          >
            <span><Marked text={row.title} ranges={row.ranges} /></span>
            <span class="clip">{row.server}</span>
            <span class="mono clip">{row.version ?? "–"}</span>
            <span class="clip muted" class:mono={row.status === "custom"} title={row.source}>{row.source}</span>
            <span class="dot {row.status}" title={row.statusLabel}></span>
          </button>
        {/each}
        {#if !rows.length}<p class="empty pad">No language server matches.</p>{/if}
      </Table>
    </div>

    {#if selected}
      {@const busy = selected.status === "installing"}
      <dl>
        <dt><label for="server-source">Source</label></dt>
        <dd class="source">
          <select
            id="server-source"
            class="field"
            disabled={busy}
            value="current"
            onchange={(e) => {
              chooseSource(selected, e.currentTarget.value);
              e.currentTarget.value = "current";
            }}
          >
            <option value="current">{selected.origin ?? "Not found"}</option>
            {#if selected.pinnedVersion && selected.origin !== "Installed by Vet"}
              <option value="install" disabled={!selected.canInstall} title={selected.installBlocker}
                >Installed by Vet…</option
              >
            {/if}
            <option value="custom">{selected.customPath ? "Change custom path…" : "Custom path…"}</option>
            {#if selected.customPath}<option value="automatic">Automatic</option>{/if}
          </select>
          {#if selected.pinnedVersion && !selected.path}
            <button
              class="ctl"
              type="button"
              disabled={!selected.canInstall}
              title={selected.installBlocker}
              onclick={() => void servers.install(selected.language)}
              >{selected.status === "failed" ? "Retry" : "Install…"}</button
            >
          {/if}
        </dd>

        {#if pathError?.language === selected.language}
          <dt></dt>
          <dd class="problem">{pathError.message}</dd>
        {/if}

        <dt>Path</dt>
        <dd class="mono">{selected.path ?? "–"}</dd>
        <dt>Version</dt>
        <dd>
          {#if selected.version}<span class="mono">{selected.version}</span>{:else if !selected.pinnedVersion}–{/if}
          {#if selected.pinnedVersion}<span class="muted">pinned</span>
            <span class="mono">{selected.pinnedVersion}</span>{/if}
        </dd>

        {#if selected.tsserver}
          <dt>tsserver</dt>
          <dd class="tsserver">
            {#if selected.tsserver.type === "using"}
              <span>{selected.tsserver.whose} ({selected.tsserver.path})</span>
            {:else}
              <span class="bad">Not found</span>
              {#if selected.pinnedVersion}
                <button
                  class="ctl"
                  type="button"
                  disabled={!selected.canInstall}
                  title={selected.installBlocker}
                  onclick={() => void servers.install(selected.language)}>Install fallback</button
                >
              {/if}
            {/if}
          </dd>
          {#if selected.tsserver.type === "missing"}
            <dt></dt>
            <dd class="hint">{selected.tsserver.reason}</dd>
          {/if}
        {/if}

        {#if busy}
          <dt></dt>
          <dd class="hint">Installing…</dd>
        {/if}
        {#if selected.problem}
          <dt></dt>
          <dd class="problem">{selected.problem}</dd>
        {/if}
        {#if selected.installBlocker && selected.pinnedVersion}
          <dt></dt>
          <dd class="hint">Cannot install: {selected.installBlocker}</dd>
        {/if}

        {#if log.length || servers.installs[selected.language]?.running}
          <dt></dt>
          <dd>
            <details
              open={servers.logOpen[selected.language]}
              ontoggle={(e) => servers.setLogOpen(selected.language, e.currentTarget.open)}
            >
              <summary>Install output</summary>
              <div class="log">
                <ScrollArea><pre class="selectable">{log.join("\n")}</pre></ScrollArea>
              </div>
            </details>
          </dd>
        {/if}
      </dl>
    {/if}
  {/if}

  <footer>
    <div class="text">
      <span class="label">Semantic caches</span>
      <span class="hint">
        Answers kept from the language servers, in memory and between launches. Clearing them only makes the next
        answers slower.
      </span>
      <div class="budget">
        {#if backendSettings.usage !== undefined}<span>{formatBytes(backendSettings.usage)} on disk ·</span>{/if}
        <label for="cache-budget">Keep up to</label>
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
        <span>MiB, least recently used go first</span>
      </div>
      {#if backendSettings.loadError}<span class="problem">{backendSettings.loadError}</span>{/if}
      {#if cacheError}<span class="problem">{cacheError}</span>{/if}
    </div>
    <button class="ctl" type="button" disabled={backendSettings.clearing} onclick={() => void clearCaches()}
      >Clear semantic caches</button
    >
  </footer>
</div>

{#if editing}
  <ServerPathSheet
    language={editing.language}
    title={editing.title}
    server={editing.server}
    current={editing.customPath ?? editing.path}
    onclose={() => (editing = undefined)}
  />
{/if}

<style>
  .page {
    display: flex;
    flex-direction: column;
    gap: 16px;
    font-size: var(--fs-ctl);
  }
  p {
    margin: 0;
  }
  .box {
    border: 1px solid var(--line);
    border-radius: var(--radius);
    background: var(--bg-editor);
    overflow: hidden;
  }
  .box .row:last-child {
    border-bottom: 0;
  }
  .clip {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .muted {
    color: var(--text-2);
  }
  .mono {
    font-family: var(--font-mono);
    font-size: var(--fs-mono);
  }
  .dot {
    justify-self: center;
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--text-3);
  }
  .dot.found,
  .dot.custom {
    background: var(--ok);
  }
  .dot.missing,
  .dot.failed {
    background: var(--del-text);
  }
  .dot.installing {
    background: var(--warn);
  }
  .empty {
    color: var(--text-3);
  }
  .pad {
    padding: 12px 10px;
  }
  dl {
    display: grid;
    grid-template-columns: 100px 1fr;
    gap: 6px 12px;
    margin: 0;
    align-items: center;
  }
  dt {
    text-align: right;
    color: var(--text-2);
  }
  dd {
    margin: 0;
    min-width: 0;
    overflow-wrap: anywhere;
  }
  .source,
  .tsserver {
    display: flex;
    gap: 6px;
    align-items: center;
  }
  .source select {
    min-width: 180px;
  }
  .bad {
    color: var(--del-text);
  }
  .hint {
    color: var(--text-3);
    font-size: 12px;
  }
  .problem {
    color: var(--del-text);
    font-size: 12px;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  summary {
    color: var(--text-2);
    font-size: 12px;
  }
  .log {
    margin: 4px 0 0;
    max-height: 160px;
    display: flex;
    flex-direction: column;
    background: var(--bg-panel);
    border: 1px solid var(--line-soft);
  }
  .log pre {
    margin: 0;
    padding: 6px 8px;
    font-family: var(--font-mono);
    font-size: var(--fs-mono-s);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  footer {
    margin-top: 8px;
    padding-top: 12px;
    border-top: 1px solid var(--line-soft);
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
  }
  .text {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .label {
    color: var(--text-2);
  }
  .budget {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 6px;
    margin-top: 6px;
    color: var(--text-2);
  }
  .budget input {
    width: 80px;
  }
</style>
