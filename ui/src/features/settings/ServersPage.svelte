<script lang="ts">
  import { onMount } from "svelte";
  import type { ServerInfo } from "../../generated/ServerInfo";
  import type { Backend } from "../../state/backend";
  import { hasNativeDialogs, pickFile } from "../../state/host";
  import { appendLog, serverRow, type InstallProgress } from "./serverRows";

  let { backend }: { backend: Backend | undefined } = $props();

  // The servers resolve per repository, so every request needs a session.
  const available = $derived(backend?.session !== undefined);

  let servers = $state.raw<ServerInfo[]>([]);
  let loading = $state(false);
  let loadError = $state<string>();
  let installs = $state.raw<Partial<Record<string, InstallProgress>>>({});
  let logs = $state.raw<Record<string, string[]>>({});
  let logOpen = $state.raw<Record<string, boolean>>({});
  /** The language whose path editor is open, and what is typed in it. */
  let editing = $state<string>();
  let draft = $state("");
  let pathError = $state<{ language: string; message: string }>();

  const rows = $derived(servers.map((info) => serverRow(info, installs[info.language])));

  const message = (e: unknown) => (e instanceof Error ? e.message : String(e));

  async function load() {
    if (!backend) return;
    loading = true;
    try {
      servers = await backend.listServers();
      loadError = undefined;
    } catch (e) {
      loadError = message(e);
    } finally {
      loading = false;
    }
  }

  onMount(() => {
    if (available) void load();
    return backend?.onEvent((event) => {
      if (event.session !== backend.session) return;
      if (event.type === "serverInstallOutput") {
        logs = { ...logs, [event.language]: appendLog(logs[event.language] ?? [], event.line) };
      } else if (event.type === "serversChanged") {
        void load();
      }
    });
  });

  async function install(language: string) {
    if (!backend) return;
    installs = { ...installs, [language]: { running: true } };
    logs = { ...logs, [language]: [] };
    logOpen = { ...logOpen, [language]: true };
    try {
      servers = await backend.installServer(language);
      installs = { ...installs, [language]: { running: false } };
    } catch (e) {
      installs = { ...installs, [language]: { running: false, error: message(e) } };
    }
  }

  function edit(language: string, current: string | undefined) {
    editing = language;
    draft = current ?? "";
    pathError = undefined;
  }

  async function setPath(language: string, path: string | null) {
    if (!backend) return;
    try {
      servers = await backend.setServerPath(language, path);
      // The result of an earlier install no longer describes what resolves now.
      installs = { ...installs, [language]: { running: false } };
      editing = undefined;
      pathError = undefined;
    } catch (e) {
      pathError = { language, message: message(e) };
    }
  }

  async function browse() {
    try {
      const picked = await pickFile();
      if (picked) draft = picked;
    } catch (e) {
      if (editing) pathError = { language: editing, message: message(e) };
    }
  }
</script>

{#if !available}
  <p class="empty">Language servers are resolved per repository. Open a repository to see and change them.</p>
{:else if loadError}
  <p class="problem">{loadError}</p>
{:else if loading && !servers.length}
  <p class="empty">Looking for language servers…</p>
{:else}
  <ul>
    {#each rows as row (row.language)}
      {@const log = logs[row.language] ?? []}
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
                onclick={() => void install(row.language)}>{row.status === "failed" ? "Retry" : "Install…"}</button
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
              class="mono"
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

        {#if log.length || installs[row.language]?.running}
          <details
            open={logOpen[row.language]}
            ontoggle={(e) => (logOpen = { ...logOpen, [row.language]: e.currentTarget.open })}
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
