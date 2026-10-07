<script lang="ts">
  import { onMount } from "svelte";
  import { accounts } from "../../state/accounts.svelte";
  import { hasKeychain } from "../../state/host";
  import { accountRow, usesCli } from "./accountRows";

  const rows = $derived(accounts.accounts.map(accountRow));

  let adding = $state(false);
  let host = $state("github.com");
  let token = $state("");
  let busy = $state(false);
  let addError = $state<string>();
  /** A token that works but that the keychain would not keep. */
  let keychainWarning = $state<string>();
  /** What went wrong removing a token, with the host it was about. */
  let removeError = $state<{ host: string; message: string }>();

  onMount(() => void accounts.load());

  function startAdding() {
    adding = true;
    token = "";
    addError = undefined;
    keychainWarning = undefined;
  }

  async function add() {
    busy = true;
    const result = await accounts.addToken(host, token);
    busy = false;
    if (result.ok) {
      adding = false;
      keychainWarning = result.warning;
    } else {
      addError = result.error;
    }
  }

  async function remove(host: string) {
    const message = await accounts.remove(host);
    removeError = message === undefined ? undefined : { host, message };
  }
</script>

<div class="page">
  {#if accounts.loadError}
    <p class="problem">{accounts.loadError}</p>
  {:else if accounts.loading && !rows.length}
    <p class="empty">Looking for accounts…</p>
  {:else if !rows.length}
    <p class="empty">
      No accounts yet. Sign in with <span class="mono">gh auth login</span> or <span class="mono">az login</span>, or
      add a personal access token.
    </p>
  {:else}
    <ul>
      {#each rows as row (row.key)}
        <li>
          <div class="head">
            <div class="text">
              <span class="title">{row.host}</span>
              <span class="hint">{row.platform}, {row.source}{row.user ? `, ${row.user}` : ""}</span>
            </div>
            <span class="status {row.status}">{row.statusLabel}</span>
            {#if row.removable}
              <button class="ctl" type="button" onclick={() => void remove(row.host)}>Remove</button>
            {/if}
          </div>
          {#if row.problem}<div class="problem">{row.problem}</div>{/if}
          {#if row.note}<div class="hint">{row.note}</div>{/if}
          {#if removeError?.host === row.host}<div class="problem">{removeError.message}</div>{/if}
        </li>
      {/each}
    </ul>
    {#if usesCli(accounts.accounts)}
      <p class="hint">
        A signed-in <span class="mono">gh</span> or <span class="mono">az</span> was found, so Vet needs no token for those
        hosts.
      </p>
    {/if}
  {/if}

  {#if adding}
    <form
      onsubmit={(e) => {
        e.preventDefault();
        void add();
      }}
    >
      <label>
        <span>Host</span>
        <input
          class="field mono"
          type="text"
          placeholder="github.com, dev.azure.com or a GitHub Enterprise host"
          autocomplete="off"
          spellcheck="false"
          bind:value={host}
        />
      </label>
      <label>
        <span>Token</span>
        <input
          class="field mono"
          type="password"
          placeholder="Personal access token"
          autocomplete="off"
          spellcheck="false"
          bind:value={token}
        />
      </label>
      <p class="hint">
        The platform checks the token before Vet keeps it. {hasKeychain
          ? "It is kept in the system keychain, never in Vet's own data."
          : "This window has no keychain, so it is kept until the backend stops."}
      </p>
      {#if addError}<p class="problem" role="alert">{addError}</p>{/if}
      <div class="actions">
        <button class="ctl" type="submit" disabled={busy || !host.trim() || !token.trim()}
          >{busy ? "Checking…" : "Add"}</button
        >
        <button class="ctl" type="button" disabled={busy} onclick={() => (adding = false)}>Cancel</button>
      </div>
    </form>
  {:else}
    <div class="actions">
      <button class="ctl" type="button" onclick={startAdding}>Add personal access token…</button>
    </div>
    {#if keychainWarning}<p class="problem" role="alert">{keychainWarning}</p>{/if}
  {/if}
</div>

<style>
  .page {
    display: flex;
    flex-direction: column;
    gap: 12px;
    font-size: var(--fs-ctl);
  }
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
  }
  li:first-child {
    padding-top: 0;
  }
  li:last-child {
    border-bottom: 0;
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
  .status {
    font-size: 12px;
    color: var(--text-2);
  }
  .status.signedIn {
    color: var(--add-text);
  }
  .status.rejected {
    color: var(--del-text);
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
  p {
    margin: 0;
  }
  .mono {
    font-family: var(--font-mono);
    font-size: var(--fs-mono-s);
  }
  form {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding-top: 12px;
    border-top: 1px solid var(--line-soft);
  }
  label {
    display: flex;
    flex-direction: column;
    gap: 4px;
    color: var(--text-2);
  }
  .actions {
    display: flex;
    gap: 6px;
  }
</style>
