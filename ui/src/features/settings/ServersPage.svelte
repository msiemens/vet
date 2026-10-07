<script lang="ts">
  import { onMount } from "svelte";
  import { errorText } from "../../lib/errorText";
  import { hasNativeDialogs, pickFile } from "../../state/host";
  import { servers } from "../../state/servers.svelte";
  import { serverRow } from "./serverRows";

  /** The language whose path editor is open, and what is typed in it. */
  let editing = $state<string>();
  let draft = $state("");
  let pathError = $state<{ language: string; message: string }>();

  const rows = $derived(servers.servers.map((info) => serverRow(info, servers.installs[info.language])));

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

  async function browse() {
    try {
      const picked = await pickFile();
      if (picked) draft = picked;
    } catch (e) {
      if (editing) pathError = { language: editing, message: errorText(e) };
    }
  }
</script>

{#if !servers.available}
  <p class="empty">Language servers are resolved per repository. Open a repository to see and change them.</p>
{:else if servers.loadError}
  <p class="problem">{servers.loadError}</p>
{:else if servers.loading && !servers.servers.length}
  <p class="empty">Looking for language servers…</p>
{:else}
  <ul>
    {#each rows as row (row.language)}
      {@const log = servers.logs[row.language] ?? []}
      <li>
        <div class="head">
          <div class="text">
            <span class="title">{row.title}</span>
            <span class="hint">{row.server}{row.pinnedVersion ? `, pinned ${row.pinnedVersion}` : ""}</span>
          </div>
          <span class="status {row.status}">{row.statusLabel}</span>
          <div class="actions">
            {#if row.pinnedVersion}
              <button
                class="ctl"
                type="button"
                disabled={!row.canInstall}
                title={row.installBlocker}
                onclick={() => void servers.install(row.language)}
                >{row.status === "failed" ? "Retry" : "Install…"}</button
              >
            {/if}
            <button
              class="ctl"
              type="button"
              disabled={row.status === "installing"}
              onclick={() => edit(row.language, row.customPath ?? row.path)}>Custom path…</button
            >
            {#if row.customPath}
              <button
                class="ctl"
                type="button"
                disabled={row.status === "installing"}
                onclick={() => void setPath(row.language, null)}>Reset</button
              >
            {/if}
          </div>
        </div>

        {#if row.path}
          <div class="detail">
            <span>{row.origin}</span>
            <span class="mono">{row.path}</span>
            {#if row.version}<span>{row.version}</span>{/if}
          </div>
        {/if}
        {#each row.notes as note (note)}<div class="hint">{note}</div>{/each}
        {#if row.problem}<div class="problem">{row.problem}</div>{/if}
        {#if row.installBlocker && row.pinnedVersion}<div class="hint">Cannot install: {row.installBlocker}</div>{/if}

        {#if editing === row.language}
          <form
            class="path"
            onsubmit={(e) => {
              e.preventDefault();
              void setPath(row.language, draft.trim() || null);
            }}
          >
            <input
              type="text"
              class="field mono"
              placeholder="Path to the server executable"
              aria-label="Server path for {row.title}"
              bind:value={draft}
            />
            {#if hasNativeDialogs}<button class="ctl" type="button" onclick={() => void browse()}>Browse…</button>{/if}
            <button class="ctl" type="submit">Set</button>
            <button class="ctl" type="button" onclick={() => (editing = undefined)}>Cancel</button>
          </form>
          {#if pathError?.language === row.language}<div class="problem">{pathError.message}</div>{/if}
        {/if}

        {#if log.length || servers.installs[row.language]?.running}
          <details
            open={servers.logOpen[row.language]}
            ontoggle={(e) => servers.setLogOpen(row.language, e.currentTarget.open)}
          >
            <summary>Install output</summary>
            <pre class="log selectable">{log.join("\n")}</pre>
          </details>
        {/if}
      </li>
    {/each}
  </ul>
{/if}

<style>
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
  }
  li {
    padding: 10px 0;
    border-bottom: 1px solid var(--line-soft);
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: var(--fs-ctl);
  }
  li:first-child {
    padding-top: 0;
  }
  li:last-child {
    border-bottom: 0;
    padding-bottom: 0;
  }
  .head {
    display: flex;
    align-items: center;
    gap: 12px;
  }
  .text {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .title {
    font-size: var(--fs);
  }
  .hint {
    color: var(--text-3);
    font-size: 12px;
  }
  .actions {
    display: flex;
    gap: 6px;
  }
  .status {
    font-size: 12px;
    color: var(--text-2);
  }
  .status.found,
  .status.custom {
    color: var(--add-text);
  }
  .status.missing,
  .status.failed {
    color: var(--del-text);
  }
  .detail {
    display: flex;
    flex-wrap: wrap;
    gap: 4px 12px;
    color: var(--text-2);
    font-size: 12px;
  }
  .mono {
    font-family: var(--font-mono);
    font-size: var(--fs-mono-s);
    overflow-wrap: anywhere;
  }
  .problem {
    color: var(--del-text);
    font-size: 12px;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .empty {
    margin: 0;
    color: var(--text-3);
  }
  .path {
    display: flex;
    gap: 6px;
  }
  .path input {
    flex: 1;
    min-width: 0;
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
</style>
