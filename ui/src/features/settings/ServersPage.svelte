<script lang="ts">
  import { onMount } from "svelte";
  import { formatBytes } from "../../lib/formatBytes";
  import { match } from "../../lib/speedSearch";
  import Icon from "../../lib/ui/Icon.svelte";
  import Marked from "../../lib/ui/Marked.svelte";
  import PathText from "../../lib/ui/PathText.svelte";
  import ScrollArea from "../../lib/ui/ScrollArea.svelte";
  import { SpeedSearch } from "../../lib/ui/speedSearch.svelte";
  import SpeedSearchField from "../../lib/ui/SpeedSearchField.svelte";
  import Table from "../../lib/ui/Table.svelte";
  import type { Column } from "../../lib/ui/columns";
  import { type Sort, sortRows } from "../../lib/ui/sorting";
  import { backendSettings } from "../../state/backendSettings.svelte";
  import { commands } from "../../state/commands";
  import { serverList } from "../../state/serverList.svelte";
  import { servers } from "../../state/servers.svelte";
  import ServerPathSheet from "./ServerPathSheet.svelte";
  import { serverRow, type ServerRow } from "./serverRows";

  const CACHES_HINT =
    "Answers kept from the language servers, in memory and between launches; the least recently used go first when the budget is exceeded. Clearing them only makes the next answers slower.";

  /** The server whose custom path sheet is open. */
  let editing = $state<ServerRow>();
  /** Why going back to the automatic server or restarting a server failed, with its language. */
  let actionError = $state<{ language: string; message: string }>();

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
    actionError = message === undefined ? undefined : { language, message };
  }

  let restarting = $state<string>();

  async function restart(language: string) {
    actionError = undefined;
    restarting = language;
    const message = await serverList.restart(language);
    restarting = undefined;
    if (message !== undefined) actionError = { language, message };
  }

  /** What the Source select offers besides the source in use; picking one acts and the select shows the source in use again. */
  function chooseSource(row: ServerRow, choice: string) {
    actionError = undefined;
    if (choice === "install") void servers.install(row.language);
    else if (choice === "custom") editing = row;
    else if (choice === "automatic") void automatic(row.language);
  }

  let cacheError = $state<string>();

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
            <span class="clip muted" class:mono={row.status === "custom"} title={row.source}>
              {#if row.status === "custom"}<PathText path={row.source} />{:else}{row.source}{/if}
            </span>
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
          <button
            class="ctl"
            type="button"
            disabled={busy || restarting !== undefined}
            onclick={() => void restart(selected.language)}>Restart</button
          >
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

        {#if actionError?.language === selected.language}
          <dt></dt>
          <dd class="problem">{actionError.message}</dd>
        {/if}

        <dt>Path</dt>
        <dd class="mono">
          {#if selected.path}<PathText path={selected.path} />{:else}–{/if}
        </dd>
        <dt>Version</dt>
        <dd>
          {#if selected.version}<span class="mono">{selected.version}</span>{:else if !selected.pinnedVersion}–{/if}
          {#if selected.pinnedVersion}<span class="muted">pinned</span>
            <span class="mono">{selected.pinnedVersion}</span>{/if}
        </dd>

        {#if selected.features === null}
          <dt>Features</dt>
          <dd class="hint">Known once the server has started</dd>
        {:else if selected.features}
          <dt class="top">Features</dt>
          <dd class="checks">
            {#each selected.features as feature (feature.name)}
              <span class="check" class:yes={feature.offered} title={feature.offered ? "Offered" : "Not offered"}>
                <Icon name={feature.offered ? "check" : "close"} size={12} />{feature.name}
              </span>
            {/each}
          </dd>
        {/if}

        {#if selected.tsserver}
          <dt>tsserver</dt>
          <dd class="tsserver">
            {#if selected.tsserver.type === "using"}
              <span>{selected.tsserver.whose}</span>
              <span class="mono of">(<PathText path={selected.tsserver.path} />)</span>
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
    <span class="label"
      >Semantic caches
      <span class="info" role="img" aria-label={CACHES_HINT} title={CACHES_HINT}><Icon name="info" size={12} /></span
      ></span
    >
    <dl>
      {#if backendSettings.usage !== undefined}
        <dt>On disk</dt>
        <dd>{formatBytes(backendSettings.usage)}</dd>
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
        <dd class="problem">{backendSettings.loadError}</dd>
      {/if}
      {#if cacheError}
        <dt></dt>
        <dd class="problem">{cacheError}</dd>
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
    </dl>
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
  .of {
    display: flex;
    min-width: 0;
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
    flex-direction: column;
    gap: 8px;
  }
  .label {
    color: var(--text-2);
  }
  .info {
    display: inline-flex;
    vertical-align: middle;
    color: var(--text-3);
  }
  dt.top {
    align-self: start;
  }
  .checks {
    display: grid;
    grid-template-columns: repeat(3, max-content);
    gap: 3px 18px;
  }
  .check {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    white-space: nowrap;
    color: var(--text);
  }
  .check.yes {
    color: var(--text-2);
  }
  .check :global(svg) {
    flex: none;
    color: var(--del-text);
  }
  .check.yes :global(svg) {
    color: var(--ok);
  }
  .budget {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .budget input {
    width: 80px;
  }
</style>
