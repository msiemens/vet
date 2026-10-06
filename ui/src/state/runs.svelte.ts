// Run targets, the pending approval and the runs this session started. The backend
// decides whether a run needs approval and what it ran; this holds and distributes that.

import { SvelteMap } from "svelte/reactivity";
import type { ApprovalChange } from "../generated/ApprovalChange";
import type { ApprovalFile } from "../generated/ApprovalFile";
import type { Event as ServerEvent } from "../generated/Event";
import type { RunTarget } from "../generated/RunTarget";
import type { RunTargetEntry } from "../generated/RunTargetEntry";
import type { Backend, RunStarted } from "./backend";
import { changeset } from "./changeset.svelte";
import { RunLog } from "./runLog";

/** A run as the UI knows it; replaced on each state change. `log` is not reactive. */
export interface Run {
  id: number;
  log: RunLog;
  /** Unknown until the `Run` request answers; output can arrive before that. */
  targetId?: string;
  /** The tree that runs, as a snapshot id. */
  snapshot?: string;
  changedSinceReview?: number;
  /** `StopRun` was sent; the exit follows. */
  stopping?: boolean;
  /** Set by `RunExited`; `code` is `null` when a signal ended the process. */
  exit?: { code: number | null; droppedBytes: number };
}

export interface Approval {
  targetId: string;
  hash: string;
  /** No run was ever approved in this repository. */
  firstRun: boolean;
  changed: ApprovalChange[];
  /** The text behind `changed`, once loaded. */
  files?: ApprovalFile[];
  /** Why the text could not be loaded. */
  failed?: string;
}

const errorText = (error: unknown) => (error instanceof Error ? error.message : String(error));

class RunStore {
  targets = $state.raw<RunTargetEntry[]>([]);
  /** Waiting for the viewer's decision. */
  approval = $state.raw<Approval>();
  /** The targets dialog is open. */
  editing = $state(false);
  /** The output panel is open. */
  panel = $state(false);
  /** The run the panel shows. */
  shown = $state<number>();
  /** The target the main button runs. */
  last = $state<string>();
  private byId = new SvelteMap<number, Run>();
  private repo?: string;

  /** In start order. */
  runs = $derived([...this.byId.values()].sort((a, b) => a.id - b.id));
  current = $derived(this.shown === undefined ? undefined : this.byId.get(this.shown));
  primary = $derived<RunTargetEntry | undefined>(
    this.targets.find((t) => t.target.id === this.last) ?? this.targets[0],
  );

  private get backend(): Backend {
    if (!changeset.backend) throw new Error("not connected to the backend");
    return changeset.backend;
  }

  /** The open repository changed: nothing of the previous one applies. */
  forRepo(repo: string | undefined) {
    this.repo = repo;
    this.targets = [];
    this.approval = undefined;
    this.editing = false;
    this.panel = false;
    this.shown = undefined;
    this.last = undefined;
    this.byId.clear();
  }

  /** Targets depend on the target snapshot; load them again when it moves. */
  async loadTargets() {
    try {
      this.targets = await this.backend.runTargets();
      this.last ??= this.stored();
    } catch (error) {
      changeset.notice = errorText(error);
    }
  }

  /** Runs a target; the backend may answer with an approval to ask the viewer for. */
  async start(targetId: string, approvedHash?: string) {
    try {
      const answer = await this.backend.run(targetId, approvedHash);
      this.remember(targetId);
      if (answer.type === "runNeedsApproval") {
        const { hash, firstRun, changed } = answer;
        this.approval = { targetId, hash, firstRun, changed };
        void this.loadDiff(hash);
      } else {
        this.approval = undefined;
        this.started(targetId, answer);
      }
    } catch (error) {
      changeset.notice = errorText(error);
    }
  }

  /** The viewer approved what the dialog showed. */
  approve() {
    const approval = this.approval;
    return approval ? this.start(approval.targetId, approval.hash) : Promise.resolve();
  }

  async stop(runId: number) {
    const run = this.byId.get(runId);
    if (run) this.byId.set(runId, { ...run, stopping: true });
    try {
      await this.backend.stopRun(runId);
    } catch (error) {
      changeset.notice = errorText(error);
    }
  }

  /** `false` after showing why it failed. */
  async save(target: RunTarget): Promise<boolean> {
    return this.edit(() => this.backend.saveRunTarget(target));
  }

  /** Drops an edit of a discovered target or deletes a custom one. */
  async remove(id: string): Promise<boolean> {
    if (this.last === id) this.last = undefined;
    return this.edit(() => this.backend.resetRunTarget(id));
  }

  onEvent(event: ServerEvent) {
    if (event.type === "runOutput") {
      this.ensure(event.runId).log.push(event.stream, event.bytes);
    } else if (event.type === "runExited") {
      const run = this.ensure(event.runId);
      run.log.finish();
      this.byId.set(run.id, { ...run, exit: { code: event.code, droppedBytes: event.droppedBytes } });
      this.prune();
    }
  }

  /** A run is created when its id is first seen, by output or by the `Run` answer. */
  private ensure(id: number): Run {
    let run = this.byId.get(id);
    if (!run) this.byId.set(id, (run = { id, log: new RunLog() }));
    return run;
  }

  private started(targetId: string, answer: RunStarted) {
    const run = this.ensure(answer.runId);
    this.byId.set(run.id, {
      ...run,
      targetId,
      snapshot: answer.snapshot,
      changedSinceReview: answer.changedSinceReview,
    });
    this.shown = run.id;
    this.panel = true;
    this.prune();
  }

  /** Keeps what is running or unattributed, and the latest run of each target. */
  private prune() {
    const latest: Record<string, number> = {};
    for (const run of this.runs) if (run.targetId !== undefined) latest[run.targetId] = run.id;
    for (const run of this.runs) {
      if (run.exit && run.targetId !== undefined && latest[run.targetId] !== run.id) this.byId.delete(run.id);
    }
    if (this.shown !== undefined && !this.byId.has(this.shown)) this.shown = this.runs.at(-1)?.id;
  }

  private async loadDiff(hash: string) {
    let update: Partial<Approval>;
    try {
      update = { files: await this.backend.runApprovalDiff(hash) };
    } catch (error) {
      update = { failed: errorText(error) };
    }
    if (this.approval?.hash === hash) this.approval = { ...this.approval, ...update };
  }

  private async edit(change: () => Promise<void>): Promise<boolean> {
    try {
      await change();
      await this.loadTargets();
      return true;
    } catch (error) {
      changeset.notice = errorText(error);
      return false;
    }
  }

  private key = () => `vet.run.last:${this.repo ?? ""}`;

  private stored(): string | undefined {
    try {
      return localStorage.getItem(this.key()) ?? undefined;
    } catch {
      return undefined;
    }
  }

  private remember(targetId: string) {
    this.last = targetId;
    try {
      localStorage.setItem(this.key(), targetId);
    } catch {
      // Not remembered across sessions.
    }
  }
}

export const runs = new RunStore();
