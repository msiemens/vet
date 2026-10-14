<script lang="ts">
  // The Pipelines tool, a bottom panel: the runs of the commit on view with their jobs, and the steps
  // of the chosen job. The viewer can run the failed jobs, a job or the whole run again and stop a run
  // that has not finished; what the platform makes of it shows with the next read of the checks.
  import { untrack } from "svelte";
  import type { CheckRun } from "../../generated/CheckRun";
  import type { CommitChecks } from "../../generated/CommitChecks";
  import type { Pipeline } from "../../generated/Pipeline";
  import { phaseLook } from "../../lib/ciStatus";
  import { platformName } from "../../lib/platformName";
  import Banner from "../../lib/ui/Banner.svelte";
  import BottomToolHeader from "../../lib/ui/BottomToolHeader.svelte";
  import CiStatus from "../../lib/ui/CiStatus.svelte";
  import EmptyState from "../../lib/ui/EmptyState.svelte";
  import Icon from "../../lib/ui/Icon.svelte";
  import IconButton from "../../lib/ui/IconButton.svelte";
  import Table from "../../lib/ui/Table.svelte";
  import TreeRow from "../../lib/ui/TreeRow.svelte";
  import type { Column } from "../../lib/ui/columns";
  import { ListNav } from "../../lib/ui/listNav";
  import { checks as checkStore } from "../../state/checks.svelte";
  import { pipelines } from "../../state/pipelines.svelte";
  import { platform } from "../../state/platform.svelte";
  import { durationText } from "../checks";
  import {
    type ActionKind,
    clock,
    defaultJob,
    disabledReason,
    finished,
    rerunnable,
    runMeta,
    runOf,
    runRows,
  } from "./pipelineRows";

  let {
    checks,
    commit,
    onlog,
    onbrowser,
    onsignin,
    oneditor,
    onclose,
  }: {
    /** The checks of the commit on view; none until the platform has answered. */
    checks?: CommitChecks;
    /** The full id of that commit; none while the changeset loads. */
    commit?: string;
    /** Opens the log of a finished job in a tab. */
    onlog: (job: CheckRun, run: Pipeline | undefined) => void;
    /** Shows the page of the commit's checks on the platform. */
    onbrowser: (url: string) => void;
    /** Opens the settings to sign in to the platform. */
    onsignin: () => void;
    /** Gives the keyboard focus back to the editor. */
    oneditor: () => void;
    /** Hides the tool. */
    onclose: () => void;
  } = $props();

  type Item =
    | { kind: "run"; key: string; rows: { run: Pipeline; jobs: CheckRun[] } }
    | { kind: "job"; key: string; job: CheckRun; run: Pipeline }
    | { kind: "service"; key: string; job: CheckRun };

  const RUN_COLUMNS: Column<Item>[] = [
    { id: "name", label: "Run and job", flex: true, min: 120 },
    { id: "actions", label: "", width: 24, fixed: true },
    { id: "time", label: "Time", width: 60, fixed: true, align: "right" },
  ];
  const STEP_COLUMNS: Column<never>[] = [
    { id: "name", label: "Step", flex: true, min: 120 },
    { id: "time", label: "Time", width: 60, fixed: true, align: "right" },
  ];

  const status = $derived(platform.status);
  const words = $derived(platform.traits?.pipelines);
  const account = $derived(status?.host);
  const name = $derived(status && platformName(status.kind));
  const rows = $derived(checks && runRows(checks));
  const job = $derived(
    checks &&
      (checks.runs.find((r) => r.id === pipelines.selected && r.pipeline !== undefined)?.id ?? defaultJob(checks)),
  );
  const chosen = $derived(checks?.runs.find((r) => r.id === job));
  const chosenRun = $derived(checks && runOf(checks, job));
  /** The run the toolbar acts on: the chosen job's, else the first. */
  const acting = $derived(chosenRun ?? checks?.pipelines[0]);
  const actingJobs = $derived(checks?.runs.filter((r) => r.pipeline === acting?.id) ?? []);
  const anyRunning = $derived(checks?.pipelines.some((p) => !finished(p.phase)) ?? false);
  const signedOut = $derived(status?.connection.type === "signedOut" || status?.connection.type === "tokenExpired");
  const limitedUntil = $derived(checkStore.limitedUntil);
  const held = $derived(
    limitedUntil !== undefined
      ? `${name ?? "The platform"} is limiting requests until ${clock(limitedUntil)}.`
      : pipelines.isDenied(account)
        ? `The token for ${account} cannot change runs.`
        : signedOut
          ? `Sign in to ${account} to change runs.`
          : undefined,
  );

  const items = $derived.by<Item[]>(() => {
    if (!rows) return [];
    const out: Item[] = [];
    for (const group of rows.runs) {
      out.push({ kind: "run", key: `run:${group.run.id}`, rows: group });
      if (pipelines.folded.has(group.run.id)) continue;
      for (const j of group.jobs) out.push({ kind: "job", key: j.id, job: j, run: group.run });
    }
    for (const s of rows.services) out.push({ kind: "service", key: s.id, job: s });
    return out;
  });

  let cursor = $state(-1);
  let tree = $state<HTMLElement>();
  const nav = new ListNav<Item>({
    element: () => tree,
    items: () => items,
    single: (item) => {
      if (item.kind === "run") pipelines.toggle(item.rows.run.id);
      else if (item.kind === "job") pipelines.selected = item.job.id;
    },
    double: (item) => {
      if (item.kind === "run") pipelines.toggle(item.rows.run.id);
      else if (item.kind === "job") {
        pipelines.selected = item.job.id;
        if (logOff(item.job) === undefined) onlog(item.job, item.run);
      }
    },
    highlight: { get: () => cursor, set: (i) => (cursor = i) },
  });
  $effect(() => () => nav.cancel());

  // The chosen job's steps; a job that runs is read again whenever the checks were.
  $effect(() => {
    const id = job;
    const over = chosen !== undefined && finished(chosen.phase);
    const stamp = checkStore.loadedAt;
    untrack(() => void pipelines.showSteps(id, over, stamp));
  });

  function onkeydown(e: KeyboardEvent) {
    if (e.key === "Escape" && !e.altKey && !e.ctrlKey && !e.metaKey && !e.shiftKey) {
      e.preventDefault();
      e.stopPropagation();
      oneditor();
    } else if (nav.keydown(e)) {
      e.preventDefault();
      e.stopPropagation();
    }
  }

  const reason = (kind: ActionKind) =>
    disabledReason(kind, { run: acting, jobs: actingJobs, held, busy: pipelines.busy });

  function act(kind: ActionKind) {
    if (acting && commit) void pipelines.act({ type: kind, run: acting.id }, commit, account);
  }

  function rerunJob(check: CheckRun) {
    if (commit) void pipelines.act({ type: "rerunJob", check: check.id }, commit, account);
  }

  /** Why the job's log cannot be opened, if it cannot. */
  function logOff(check: CheckRun): string | undefined {
    if (!check.log) return "The platform has no log of this";
    return finished(check.phase) ? undefined : "The log is available when the job finishes";
  }

  const jobReason = $derived(held ?? (pipelines.busy ? "Waiting for the platform" : undefined));
  const timeOf = (check: CheckRun) => (check.duration === undefined ? "" : durationText(check.duration));
  const longest = (jobs: readonly CheckRun[]) =>
    jobs.reduce<number | undefined>((a, j) => (j.duration === undefined ? a : Math.max(a ?? 0, j.duration)), undefined);
</script>

<section aria-label="Pipelines" tabindex="-1">
  <BottomToolHeader title="Pipelines" count={checks?.pipelines.length} {onclose}>
    {#snippet controls()}
      <span class="mono commit" title="The commit these runs belong to">{commit?.slice(0, 7) ?? ""}</span>
      <span class="sep"></span>
      {#if words}
        {@render action("rerunFailed", words.rerunFailed, "reconfirm")}
        {@render action("rerunAll", words.rerunAll)}
        {@render action("cancel", words.cancel, "stop")}
      {/if}
      <span class="grow"></span>
      {#if checkStore.loadedAt !== undefined}
        <span class="when"
          >Updated {clock(checkStore.loadedAt)}{anyRunning && limitedUntil === undefined
            ? " · refreshes every 30 s"
            : ""}</span
        >
      {/if}
      {#if checks?.url}
        {@const url = checks.url}
        <IconButton icon="external" label="Open in browser" size="s" iconSize={13} onclick={() => onbrowser(url)} />
      {/if}
    {/snippet}
  </BottomToolHeader>
  {#if signedOut && status}
    <Banner
      tone="bad"
      message="{name} sync stopped. The token for {status.host} expired, so runs cannot be loaded or changed."
      actions={[{ label: "Sign in…", onclick: onsignin }]}
    />
  {:else if limitedUntil !== undefined}
    <Banner
      message="{name ?? 'The platform'} is limiting requests until {clock(limitedUntil)}.{checkStore.loadedAt ===
      undefined
        ? ''
        : ` Showing what was loaded at ${clock(checkStore.loadedAt)}.`} Runs refresh when the limit ends; actions wait until then."
    />
  {/if}
  <div class="body">
    {#if !checks}
      {#if signedOut && status}
        <div class="whole">
          <EmptyState text="Nothing loaded. Sign in to {status.host} and the runs of this pull request appear here." />
        </div>
      {/if}
    {:else if !checks.runs.length}
      <div class="whole"><EmptyState text="This commit has no checks." /></div>
    {:else}
      <div class="runs" bind:this={tree} {...nav.attributes} role="tree" aria-label="Runs and jobs" {onkeydown}>
        <Table
          name="pipeline-runs"
          columns={RUN_COLUMNS}
          --table-pad="0px"
          --table-pad-end="8px"
          --table-row="var(--h-row)"
          --table-head="var(--h-ctl-s)"
          --table-head-color="var(--text-3)"
          --table-head-size="var(--fs-mono-s)"
          scroll
        >
          {#each items as item, index (item.key)}
            {#if item.kind === "run"}
              {@const group = item.rows}
              <div
                class="row"
                role="treeitem"
                aria-selected="false"
                aria-expanded={!pipelines.folded.has(group.run.id)}
              >
                <TreeRow
                  open={!pipelines.folded.has(group.run.id)}
                  tone="strong"
                  letters={false}
                  cur={cursor === index}
                  title={group.run.name}
                  onclick={(e) => nav.click(e, index)}
                >
                  {#snippet glyph()}<CiStatus state={group.run.state} phase={group.run.phase} size={12} />{/snippet}
                  {group.run.name}<span class="mono meta">{runMeta(group.run)}</span>
                </TreeRow>
                <span></span>
                <span class="mono time"
                  >{longest(group.jobs) === undefined ? "" : durationText(longest(group.jobs) ?? 0)}</span
                >
              </div>
            {:else if item.kind === "job"}
              <div class="pick row" class:on={item.job.id === job} role="treeitem" aria-selected={item.job.id === job}>
                <TreeRow
                  depth={1}
                  open={undefined}
                  letters={false}
                  cur={cursor === index}
                  title={item.job.name}
                  onclick={(e) => nav.click(e, index)}
                >
                  {#snippet glyph()}<CiStatus state={item.job.state} phase={item.job.phase} size={12} />{/snippet}
                  {item.job.name}
                </TreeRow>
                <span class="act">
                  {#if words && rerunnable(item.job, item.run)}
                    <IconButton
                      icon="reconfirm"
                      label={words.rerunGroup}
                      title={jobReason ?? words.rerunGroup}
                      disabled={jobReason !== undefined}
                      size="xs"
                      iconSize={12}
                      onclick={() => rerunJob(item.job)}
                    />
                  {/if}
                </span>
                <span class="mono time">{timeOf(item.job)}</span>
              </div>
            {:else}
              <div class="row" role="treeitem" aria-selected="false">
                <TreeRow tone="plain" interactive={false} letters={false} cur={cursor === index} title={item.job.name}>
                  {#snippet glyph()}<CiStatus state={item.job.state} phase={item.job.phase} size={12} />{/snippet}
                  {item.job.name}<span class="mono meta">status from a service</span>
                </TreeRow>
                <span></span>
                <span class="mono time">{timeOf(item.job)}</span>
              </div>
            {/if}
          {/each}
        </Table>
      </div>
      <div class="detail">
        {#if chosen}
          <div class="head">
            <CiStatus state={chosen.state} phase={chosen.phase} size={12} />
            <b class="job">{chosen.name}</b>
            <span class="mono sub"
              >{chosenRun?.name} · {phaseLook(chosen.state, chosen.phase).title.toLowerCase()}{timeOf(chosen)
                ? ` · ${timeOf(chosen)}`
                : ""}</span
            >
            <span class="grow"></span>
            <button
              class="tool"
              type="button"
              disabled={logOff(chosen) !== undefined}
              title={logOff(chosen)}
              onclick={() => onlog(chosen, chosenRun)}
              ><Icon name="file" size={12} />{chosen.state === "failure" ? "Open log at failure" : "Open log"}</button
            >
          </div>
          {#if chosen.phase === "queued"}
            <EmptyState
              title="Queued."
              text="Waiting for the platform to start it. The {words?.step ?? 'step'}s appear when the {words?.group ??
                'job'} starts."
            />
          {:else if pipelines.stepsError !== undefined}
            <EmptyState text={pipelines.stepsError} error />
          {:else if pipelines.steps?.job === chosen.id}
            <Table
              name="pipeline-steps"
              columns={STEP_COLUMNS}
              --table-pad="14px"
              --table-pad-end="12px"
              --table-row="24px"
              --table-head="var(--h-ctl-s)"
              --table-head-color="var(--text-3)"
              --table-head-size="var(--fs-mono-s)"
            >
              {#each pipelines.steps.steps as step, index (index)}
                <div class="row step" class:failing={step.state === "failure"}>
                  <span class="stepname"
                    ><CiStatus state={step.state} phase={step.phase} size={12} /><span class="label">{step.name}</span
                    ></span
                  >
                  <span class="mono time"
                    >{step.phase === "running"
                      ? "…"
                      : step.duration === undefined
                        ? ""
                        : durationText(step.duration)}</span
                  >
                </div>
              {/each}
            </Table>
          {/if}
        {:else}
          <EmptyState text="Choose a {words?.group ?? 'job'} to see its {words?.step ?? 'step'}s." />
        {/if}
      </div>
    {/if}
  </div>
</section>

{#snippet action(kind: ActionKind, label: string, icon?: "reconfirm" | "stop")}
  {@const off = reason(kind)}
  <button class="tool" type="button" disabled={off !== undefined} title={off ?? label} onclick={() => act(kind)}>
    {#if icon}<Icon name={icon} size={12} />{/if}{label}
  </button>
{/snippet}

<style>
  section {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    border-top: 1px solid var(--line);
    background: var(--bg-panel);
  }
  .commit,
  .sep,
  section :global(.tool) {
    flex: none;
    white-space: nowrap;
  }
  .commit {
    font-size: var(--fs-mono-s);
    color: var(--text-2);
  }
  .sep {
    width: 1px;
    height: 16px;
    background: var(--line);
  }
  .grow {
    flex: 1;
  }
  .when {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text-3);
    font-size: var(--fs-mono-s);
  }
  .body {
    flex: 1;
    min-height: 0;
    display: flex;
    background: var(--bg-editor);
  }
  .runs {
    flex: none;
    width: var(--w-pipelines-runs);
    display: flex;
    border-right: 1px solid var(--line);
  }
  .detail {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .head {
    flex: none;
    display: flex;
    align-items: center;
    gap: 8px;
    height: var(--h-bar);
    padding: 0 12px;
    border-bottom: 1px solid var(--line-soft);
  }
  .job,
  .sub {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .job {
    font-weight: 650;
  }
  .sub {
    flex: 1 1 0;
  }
  .head .tool {
    flex: none;
  }
  .sub,
  .meta,
  .time {
    color: var(--text-3);
    font-size: var(--fs-mono-s);
  }
  .meta {
    margin-left: 6px;
    font-weight: 400;
  }
  .time {
    text-align: right;
  }
  .act {
    display: flex;
    justify-content: center;
  }
  .step.failing {
    background: var(--del-bg);
  }
  .stepname {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
    font-size: var(--fs-ctl);
  }
  .label {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .whole {
    flex: 1;
    min-width: 0;
  }
</style>
