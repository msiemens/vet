// The Problems tool's state: the diagnostics of the whole changeset, asked file by file in the
// background once a review is on screen, with the origins the base comparison gives them. The
// semantic store holds the answers; this decides which files to ask about, joins them with the
// hunks' review state and says how complete the picture is (SPEC §7.3, I6).

import { untrack } from "svelte";
import type { Unavailable } from "../generated/Unavailable";
import { changeset } from "./changeset.svelte";
import { Gate } from "./gate";
import { type Counts, type FileHunks, type Problem, countsOf, inOrder, problemsOf, step } from "./problemRows";
import { type Origins, type Summary, summarize } from "./problemsSummary";
import { queue } from "./queue.svelte";
import { type FileDiagnostics, semantic } from "./semantic.svelte";
import { serverList } from "./serverList.svelte";
import { languageOf } from "./serverStatus";
import { session } from "./session.svelte";

/** Files asked about at once. */
const FILES_AT_ONCE = 4;

/** A file no language serves. */
const UNSUPPORTED: Unavailable = {
  kind: "unsupported",
  label: "No language support",
  message: "No language server handles this type of file",
  language: null,
};

/** A file the panel lists as not analysed, and why. */
export interface NotAnalysed {
  path: string;
  why: Unavailable;
}

class ProblemsState {
  /** Files beyond the changeset's that the panel wants answers for (the open tabs). */
  private extra = $state.raw<readonly string[]>([]);
  private gate = new Gate(FILES_AT_ONCE);
  private run = 0;
  private watchers = $state(0);

  /** The changeset's files that have text to analyse, whatever view the review shows. */
  readonly changed = $derived.by(() => {
    const files = queue.onViewFrame?.layout.files ?? [];
    return files.filter((f) => f.status !== "deleted" && f.targetLines > 0).map((f) => f.path);
  });
  /** How many files the changeset changes, with or without text. */
  readonly changedCount = $derived(queue.onViewFrame?.layout.files.length ?? 0);
  /** The changed files a language serves, which are asked about. */
  readonly analysable = $derived(this.changed.filter((p) => this.served(p)));
  readonly problems: Problem[] = $derived(this.problemsOf(this.analysable));
  readonly origins: Origins = $derived(this.originsOf(this.analysable));
  readonly counts: Counts = $derived(countsOf(this.problems, this.origins === "known"));
  readonly progress = $derived.by(() => {
    const answered = this.analysable.filter((p) => semantic.files.has(p) || semantic.unavailableOf(p)).length;
    return { total: this.analysable.length, answered };
  });
  readonly summary: Summary | undefined = $derived(
    summarize({
      counts: this.counts,
      origins: this.origins,
      files: this.progress,
      changed: this.changedCount,
    }),
  );

  constructor() {
    $effect.root(() => {
      $effect(() => {
        const { backend } = session;
        const revision = changeset.layout?.shown.revision;
        const paths =
          this.watchers > 0 ? [...new Set([...this.analysable, ...this.extra.filter((p) => this.served(p))])] : [];
        // eslint-disable-next-line @typescript-eslint/no-unused-expressions -- answers dropped are asked for again
        semantic.generation;
        untrack(() => {
          if (backend && revision !== undefined) this.load(revision, paths);
        });
      });
    });
  }

  /** The review screen is up: the changeset's files are watched and asked about. Returns the function that ends that. */
  start(): () => void {
    untrack(() => this.watchers++);
    const stopQueue = queue.watch();
    return () => {
      untrack(() => this.watchers--);
      stopQueue();
    };
  }

  /** Wants answers for these files too, for as long as they are the ones wanted. */
  want(paths: readonly string[]) {
    if (paths.length !== this.extra.length || paths.some((p, i) => p !== this.extra[i])) this.extra = paths;
  }

  /** Whether a language server serves the file; true while the servers are not known. */
  served(path: string): boolean {
    return !serverList.servers.length || languageOf(path, serverList.servers) !== undefined;
  }

  private load(revision: number, paths: readonly string[]) {
    const run = ++this.run;
    for (const path of paths) {
      void this.gate.run(async () => {
        if (run === this.run) await semantic.diagnostics(revision, path);
      });
    }
  }

  /** What the file's answer is: the current one, or the last revision's while the current has not come. */
  answerOf(path: string): { answer: FileDiagnostics; stale: boolean } | undefined {
    const current = semantic.files.get(path);
    if (current) return { answer: current, stale: false };
    const held = semantic.previous.get(path);
    return held && { answer: held, stale: true };
  }

  private hunksOf(path: string): FileHunks | undefined {
    const file = queue.onViewFile(path);
    return file && { hunks: file.hunks, states: file.review.hunks };
  }

  /** The problems of these files, each with the review state of its hunk. */
  problemsOf(paths: readonly string[]): Problem[] {
    return paths.flatMap((path) => {
      const found = this.answerOf(path);
      return found ? problemsOf(path, found.answer.diagnostics, this.hunksOf(path)) : [];
    });
  }

  /** Whether the answers for these files say which problems the change introduced. */
  originsOf(paths: readonly string[]): Origins {
    const answers = paths.flatMap((p) => this.answerOf(p)?.answer ?? []);
    if (!answers.length || answers.some((a) => !a.compared)) return "pending";
    return answers.some((a) => a.originUnavailable) ? "unknown" : "known";
  }

  /** Why the origins are unknown, from the first file that says. */
  unknownWhy(paths: readonly string[]): Unavailable | undefined {
    return paths.flatMap((p) => this.answerOf(p)?.answer.originUnavailable ?? [])[0];
  }

  /** Whether any of these files shows the last revision's answer. */
  staleIn(paths: readonly string[]): boolean {
    return paths.some((p) => this.answerOf(p)?.stale);
  }

  /** Files of `paths` that have no problems list: a server is missing, failed or there is none; those still waiting are left out. */
  notAnalysed(paths: readonly string[]): NotAnalysed[] {
    return paths.flatMap((path): NotAnalysed[] => {
      if (!this.served(path)) return [{ path, why: UNSUPPORTED }];
      const why = semantic.unavailableOf(path);
      return why ? [{ path, why }] : [];
    });
  }

  /** How many of `paths` a server has answered for, and how many it can be asked about. */
  progressOf(paths: readonly string[]): { answered: number; total: number } {
    const asked = paths.filter((p) => this.served(p));
    return {
      total: asked.length,
      answered: asked.filter((p) => semantic.files.has(p) || semantic.unavailableOf(p)).length,
    };
  }

  /** The problem last shown, by id. */
  selected = $state<string>();
  /** Whether the panel lists only what the change introduced; the footer's item asks for that. */
  origin = $state<"new" | "all">("all");

  /** The problem `delta` after the selected one, new ones first and across files; it becomes the selected one. */
  step(delta: 1 | -1): Problem | undefined {
    const next = step(inOrder(this.problems), this.selected, delta);
    this.selected = next?.id;
    return next;
  }

  /** Asks the base side again, after a comparison failed. */
  retry() {
    semantic.invalidate();
  }
}

export const problems = new ProblemsState();
