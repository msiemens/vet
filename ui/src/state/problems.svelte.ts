// The Problems tool's state: the diagnostics of the whole changeset, which the backend analyses
// file by file in the background once the review screen shows it, comparing each with the base and
// pushing each answer (SPEC §6.6, A13). The semantic store holds the answers; this joins them with
// the hunks' review state and says how complete the picture is (SPEC §6.3, I3).

import type { Unavailable } from "../generated/Unavailable";
import { opened } from "./opened.svelte";
import type { FileDiagnostics } from "./semantic.svelte";
import { type Counts, type FileHunks, type Problem, countsOf, problemsOf } from "./problemRows";
import { type Origins, type Summary, originsOf, summarize } from "./problemsSummary";
import { notices } from "./notices.svelte";
import { servedBy } from "./servedBy.svelte";
import { semantic } from "./semantic.svelte";
import { session } from "./session.svelte";

/** A file the panel lists as not analysed, and why. */
export interface NotAnalysed {
  path: string;
  why: Unavailable;
}

class ProblemsState {
  /** The changeset's files that have text to analyse, whatever view the review shows. */
  readonly changed = $derived.by(() => {
    const files = opened.session?.wholeFrame?.layout.files ?? [];
    return files.filter((f) => f.status !== "deleted" && f.targetLines > 0).map((f) => f.path);
  });
  /** How many files the changeset changes, with or without text. */
  readonly changedCount = $derived(opened.session?.wholeFrame?.layout.files.length ?? 0);
  /** The changed files a language serves, which are analysed. */
  readonly analysable = $derived(this.changed.filter((p) => this.served(p)));
  readonly problems: Problem[] = $derived(this.problemsOf(this.analysable));
  readonly origins: Origins = $derived(this.originsOf(this.analysable));
  readonly counts: Counts = $derived(countsOf(this.problems));
  readonly progress = $derived.by(() => {
    const answered = this.analysable.filter((p) => semantic.files.has(p) || semantic.unavailableOf(p)).length;
    return { total: this.analysable.length, answered };
  });
  readonly summary: Summary | undefined = $derived(
    summarize({
      counts: this.counts,
      comparing: this.origins === "pending",
      unknownFiles: this.unknownFiles(this.analysable).length,
      files: this.progress,
      changed: this.changedCount,
    }),
  );

  /** Whether a language server serves the file; true while that is not known. */
  served(path: string): boolean {
    return servedBy.of(path)?.type !== "unsupported";
  }

  /** What the file's answer is: the current one, or the last revision's while the current has not come. */
  answerOf(path: string): { answer: FileDiagnostics; stale: boolean } | undefined {
    const current = semantic.files.get(path);
    if (current) return { answer: current, stale: false };
    const held = semantic.previous.get(path);
    return held && { answer: held, stale: true };
  }

  private hunksOf(path: string): FileHunks | undefined {
    const file = opened.session?.whole.reviewed(path);
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
    return originsOf(paths.flatMap((p) => this.answerOf(p)?.answer ?? []));
  }

  /** The files of `paths` whose problems could not be told new from existing. */
  unknownFiles(paths: readonly string[]): string[] {
    return paths.filter((p) => this.answerOf(p)?.answer.originUnavailable);
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
      const serving = servedBy.of(path);
      if (serving?.type === "unsupported") return [{ path, why: serving.why }];
      const why = semantic.unavailableOf(path);
      return why ? [{ path, why }] : [];
    });
  }

  /** The files of `paths` a server can be asked about and has not answered for yet. */
  unanswered(paths: readonly string[]): string[] {
    return paths.filter((p) => this.served(p) && !semantic.files.has(p) && !semantic.unavailableOf(p));
  }

  /** How many of `paths` a server has answered for, and how many it can be asked about. */
  progressOf(paths: readonly string[]): { answered: number; total: number } {
    const total = paths.filter((p) => this.served(p)).length;
    return { total, answered: total - this.unanswered(paths).length };
  }

  /** The problem last shown, by id. */
  selected = $state<string>();
  /** Whether the panel lists only what the change introduced. */
  origin = $state<"new" | "all">("all");

  /** Asks the backend to analyse the base side again, after a comparison failed. */
  retry() {
    void session.backend?.refreshProblems().catch((error: unknown) => {
      notices.failed("Could not analyse the problems again", error);
    });
  }
}

export const problems = new ProblemsState();
