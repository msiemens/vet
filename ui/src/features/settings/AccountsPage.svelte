<script lang="ts">
  import { onMount } from "svelte";
  import { accounts } from "../../state/accounts.svelte";
  import Facts from "../../lib/ui/Facts.svelte";
  import Icon from "../../lib/ui/Icon.svelte";
  import StatusDot from "../../lib/ui/StatusDot.svelte";
  import type { Column } from "../../lib/ui/columns";
  import { type Sort, sortRows } from "../../lib/ui/sorting";
  import { hasKeychain } from "../../state/host";
  import AccountSheet from "./AccountSheet.svelte";
  import SettingsTable from "./SettingsTable.svelte";
  import { accountRow, accountTone, type AccountRow } from "./accountRows";

  const COLUMNS: Column<AccountRow>[] = [
    { id: "platform", label: "", width: 50, fixed: true },
    { id: "host", label: "Host", flex: true, min: 80, sortBy: (row) => row.host },
    { id: "account", label: "Account", width: 130, min: 60 },
    { id: "credential", label: "Credential", width: 100, min: 60 },
    { id: "status", label: "", width: 28, fixed: true },
  ];

  let sort = $state<Sort>();
  const rows = $derived(
    sortRows(
      accounts.accounts.map((a) => accountRow(a, hasKeychain)),
      COLUMNS,
      sort,
    ),
  );
  let selectedKey = $state<string>();
  const selected = $derived<AccountRow | undefined>(rows.find((r) => r.key === selectedKey) ?? rows[0]);

  let sheet = $state<"add" | "replace">();
  let checking = $state(false);
  /** What went wrong with the selected account's last action, with the account it was about. */
  let problem = $state<{ key: string; message: string }>();
  /** A token that works but that the keychain would not keep. */
  let keychainWarning = $state<string>();

  onMount(() => void accounts.load());

  function done(key: string, warning?: string) {
    sheet = undefined;
    selectedKey = key;
    keychainWarning = warning;
    problem = undefined;
  }

  async function checkAgain(host: string, key: string) {
    checking = true;
    const message = await accounts.recheck(host);
    checking = false;
    problem = message === undefined ? undefined : { key, message };
  }

  async function remove(host: string, key: string) {
    const message = await accounts.remove(host);
    problem = message === undefined ? undefined : { key, message };
  }
</script>

<div class="page">
  {#if accounts.loadError}
    <p class="problem">{accounts.loadError}</p>
  {/if}
  <SettingsTable
    name="accounts"
    columns={COLUMNS}
    gap="4px"
    bind:sort
    role="listbox"
    aria-label="Accounts"
    style="min-height: 150px"
  >
    {#each rows as row (row.key)}
      <button
        class="flat pick row"
        class:on={row === selected}
        type="button"
        role="option"
        aria-selected={row === selected}
        onclick={() => (selectedKey = row.key)}
      >
        <span class="mark" title={row.platform.name}><Icon name={row.platform.icon} size={16} /></span>
        <span class="host">{row.host}</span>
        <span>{row.user ?? ""}</span>
        <span>{row.credential}</span>
        <StatusDot tone={accountTone(row.status)} title={row.statusLabel} />
      </button>
    {/each}
    {#if !rows.length}
      <p class="empty">
        {accounts.loading
          ? "Looking for accounts…"
          : "No accounts yet. Sign in with gh auth login or az login, or add a personal access token."}
      </p>
    {/if}
    {#snippet below()}
      <div class="strip">
        <button type="button" aria-label="Add account" onclick={() => (sheet = "add")}>+</button>
        <button
          type="button"
          aria-label="Remove account"
          disabled={!selected?.removable}
          onclick={() => selected && void remove(selected.host, selected.key)}>−</button
        >
      </div>
    {/snippet}
  </SettingsTable>

  {#if selected}
    <Facts>
      <dt>Host</dt>
      <dd class="mono">{selected.host}</dd>
      <dt>Account</dt>
      <dd>{selected.user ?? "unknown"}</dd>
      <dt>Credential</dt>
      <dd>{selected.storage}</dd>
      <dt>Status</dt>
      <dd class="status {selected.status}">{selected.statusLabel}</dd>
      {#if selected.expiry}
        <dt>Expiry</dt>
        <dd class:soon={selected.expiry.soon}>{selected.expiry.label}</dd>
      {/if}
      {#if selected.problem}
        <dt></dt>
        <dd class="problem">{selected.problem}</dd>
      {/if}
      {#if selected.note}
        <dt></dt>
        <dd class="hint">{selected.note}</dd>
      {/if}
      {#if problem?.key === selected.key}
        <dt></dt>
        <dd class="problem" role="alert">{problem.message}</dd>
      {/if}
      <dt></dt>
      <dd class="actions">
        {#if selected.removable}
          <button class="ctl" type="button" onclick={() => (sheet = "replace")}>Replace token…</button>
        {/if}
        <button
          class="ctl"
          type="button"
          disabled={checking}
          onclick={() => void checkAgain(selected.host, selected.key)}>{checking ? "Checking…" : "Check again"}</button
        >
      </dd>
    </Facts>
  {/if}
  {#if keychainWarning}<p class="problem" role="alert">{keychainWarning}</p>{/if}
</div>

{#if sheet}
  <AccountSheet
    replace={sheet === "replace" ? selected?.host : undefined}
    onclose={() => (sheet = undefined)}
    ondone={done}
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
  .host,
  .mono {
    font-family: var(--font-mono);
    font-size: var(--fs-mono);
  }
  .mark {
    display: flex;
    color: var(--text-2);
  }
  .row > :global(.dot) {
    justify-self: center;
  }
  .empty {
    padding: 12px 10px;
    color: var(--text-3);
  }
  .strip {
    display: flex;
    border-top: 1px solid var(--line);
    background: var(--bg-panel);
  }
  .strip button {
    width: 28px;
    height: 22px;
    border: 0;
    border-right: 1px solid var(--line);
    background: transparent;
    color: var(--text);
    font: inherit;
  }
  .strip button:enabled:hover {
    background: var(--hover);
  }
  .strip button:disabled {
    opacity: 0.4;
  }
  .status.signedIn {
    color: var(--add-text);
  }
  .status.rejected,
  .status.unusable,
  .soon {
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
  }
  .actions {
    display: flex;
    gap: 8px;
  }
</style>
