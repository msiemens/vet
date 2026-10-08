<script lang="ts">
  import { onMount } from "svelte";
  import { errorText } from "../../lib/errorText";
  import { match } from "../../lib/speedSearch";
  import Marked from "../../lib/ui/Marked.svelte";
  import { SpeedSearch } from "../../lib/ui/speedSearch.svelte";
  import SpeedSearchField from "../../lib/ui/SpeedSearchField.svelte";
  import { hasNativeDialogs, pickFile } from "../../state/host";
  import { servers } from "../../state/servers.svelte";
  import { serverRow, type ServerRow } from "./serverRows";

  /** The language whose path editor is open, and what is typed in it. */
  let editing = $state<string>();
  let draft = $state("");
  let pathError = $state<{ language: string; message: string }>();

  const search = new SpeedSearch();
  let tableEl = $state<HTMLElement>();
  const rows = $derived(
    servers.servers
      .map((info) => serverRow(info, servers.installs[info.language]))
      .flatMap((row) => {
        const m = match(search.query, row.title);
        return m ? [{ ...row, ranges: m.ranges }] : [];
      }),
  );
  let selectedLanguage = $state<string>();
  const selected = $derived(rows.find((r) => r.language === selectedLanguage) ?? rows.at(0));
  const log = $derived(selected ? (servers.logs[selected.language] ?? []) : []);

  onMount(() => servers.start());

  function edit(language: string, current: string | undefined) {
    editing = language;
    draft = current ?? "";
    pathError = undefined;
  }

  async function setPath(language: string, path: string | null) {
    const message = await servers.setPath(language, path);
    pathError = message === undefined ? undefined : { language, message };
    if (message === undefined) editing = undefined;
  }

  /** What the Source select offers besides the source in use; picking one acts and the select shows the source in use again. */
  function chooseSource(row: ServerRow, choice: string) {
    if (choice === "install") void servers.install(row.language);
    else if (choice === "custom") edit(row.language, row.customPath ?? row.path);
    else if (choice === "automatic") void setPath(row.language, null);
  }

  let clearError = $state<string>();

  async function clearCaches() {
    clearError = await servers.clearCaches();
  }

  async function browse() {
    try {
      const picked = await pickFile();
      if (picked) draft = picked;
    } catch (e) {
      if (editing) pathError = { language: editing, message: errorText(e) };
    }
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
      <div
        bind:this={tableEl}
        class="table"
        role="listbox"
        aria-label="Language servers"
        tabindex="-1"
        onkeydown={(e) => search.keydown(e, tableEl, "button:enabled")}
        {...search.attributes}
      >
        <div class="head">
          <span>Language</span><span>Server</span><span>Version</span><span>Source</span><span></span>
        </div>
        {#each rows as row (row.language)}
          <button
            class="row"
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
      </div>
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

        {#if editing === selected.language}
          <dt></dt>
          <dd>
            <form
              class="path"
              onsubmit={(e) => {
                e.preventDefault();
                void setPath(selected.language, draft.trim() || null);
              }}
            >
              <input
                type="text"
                class="field mono"
                placeholder="Path to the server executable"
                aria-label="Server path for {selected.title}"
                bind:value={draft}
              />
              {#if hasNativeDialogs}<button class="ctl" type="button" onclick={() => void browse()}>Browse…</button
                >{/if}
              <button class="ctl" type="submit">Set</button>
              <button class="ctl" type="button" onclick={() => (editing = undefined)}>Cancel</button>
            </form>
            {#if pathError?.language === selected.language}<div class="problem">{pathError.message}</div>{/if}
          </dd>
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
              <pre class="log selectable">{log.join("\n")}</pre>
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
      {#if clearError}<span class="problem">{clearError}</span>{/if}
    </div>
    <button class="ctl" type="button" disabled={servers.clearing} onclick={() => void clearCaches()}
      >Clear semantic caches</button
    >
  </footer>
</div>

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
  .table {
    display: flex;
    flex-direction: column;
  }
  .head,
  .row {
    display: grid;
    grid-template-columns: 0.8fr 1.5fr 0.7fr 1fr 28px;
    align-items: center;
    column-gap: 8px;
    padding: 0 6px 0 10px;
    height: 30px;
    text-align: left;
  }
  .head {
    color: var(--text-3);
    border-bottom: 1px solid var(--line-soft);
  }
  .row {
    border: 0;
    border-bottom: 1px solid var(--line-soft);
    background: transparent;
    color: var(--text);
    font: inherit;
  }
  .row:last-child {
    border-bottom: 0;
  }
  .row:hover {
    background: var(--hover);
  }
  .row[aria-selected="true"] {
    background: var(--toggle);
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
  .tsserver,
  .path {
    display: flex;
    gap: 6px;
    align-items: center;
  }
  .source select {
    min-width: 180px;
  }
  .path input {
    flex: 1;
    min-width: 0;
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
    overflow: auto;
    padding: 6px 8px;
    background: var(--bg-panel);
    border: 1px solid var(--line-soft);
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
</style>
