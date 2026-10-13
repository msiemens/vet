// What a view that shows no file says, by the reason it shows none: a glyph, a title, one sentence,
// panels of rows and the actions that fit. Only facts the backend sent become rows; a row whose fact
// is unknown is left out.

import type { Base } from "../../generated/Base";
import type { ChangesetKey } from "../../generated/ChangesetKey";
import type { CommitUnit } from "../../generated/CommitUnit";
import type { ExcludedBy } from "../../generated/ExcludedBy";
import type { ExcludedGroup } from "../../generated/ExcludedGroup";
import type { InterdiffInfo } from "../../generated/InterdiffInfo";
import type { LineDelta } from "../../generated/LineDelta";
import type { PullRequestInfo } from "../../generated/PullRequestInfo";
import type { ReviewFilter } from "../../generated/ReviewFilter";
import type { ReviewInfo } from "../../generated/ReviewInfo";
import type { Target } from "../../generated/Target";
import type { VersionInfo } from "../../generated/VersionInfo";
import { platformName } from "../../lib/platformName.ts";
import { shortTime } from "../../lib/shortTime.ts";
import { chooseRow } from "../../state/timelineRange.ts";

/** A piece of a row's value: plain text, or code (a hash, a branch, a path). */
export type Seg = string | { mono: string };

/** What a button or link of the block does; the screen carries it out. */
export interface Action {
  id:
    | "browse"
    | "timeline"
    | "comments"
    | "other"
    | "provider"
    | "back"
    | "next"
    | "filter"
    | "show"
    | "narrow"
    | "reset"
    | "pick"
    | "step"
    | "openFile";
  label: string;
  primary?: boolean;
  /** For `narrow`: the filter the review is narrowed to. */
  filter?: ReviewFilter;
  /** For `openFile`: the file. For `pick`: the version. For `step`: the commit's index. */
  path?: string;
  n?: number;
}

export interface Row {
  label?: string;
  value: Seg[];
  meta?: string;
  /** The row's own link. */
  action?: Action;
  /** The whole row acts. */
  open?: Action;
  /** The row shows the live dot before its value. */
  live?: boolean;
}

export interface Panel {
  heading?: string;
  count?: number;
  /** A link in the heading. */
  link?: Action;
  rows: Row[];
}

export interface EmptyView {
  glyph: "emptyWatch" | "emptyMinus" | "emptyMerge" | "emptyCheck" | "emptyFilter" | "emptyCommit" | "emptyRange";
  title: string;
  lead: string;
  panels: Panel[];
  actions: Action[];
}

/** What the screen knows when a view shows no file. */
export interface EmptyInput {
  key: ChangesetKey;
  target: Target;
  /** Which view is empty. */
  view: "changeset" | "commit" | "range";
  /** The shown view's base: a commit's parent, an interdiff's base, or the changeset's. */
  base: Base;
  /** The changeset's own base. */
  changesetBase: Base;
  review: ReviewInfo;
  excluded: readonly ExcludedGroup[];
  /** The project's filter, and the narrowing of this review if there is one. */
  filter: ReviewFilter;
  narrowing?: ReviewFilter;
  repo?: string;
  versions: readonly VersionInfo[];
  interdiff?: InterdiffInfo;
  commits: readonly CommitUnit[];
  commitIndex?: number;
  pullRequest?: PullRequestInfo;
  threads: { total: number; open: number };
  now: number;
}

const mono = (text: string): Seg => ({ mono: text });
const plural = (n: number, one: string, many = `${one}s`) => `${n} ${n === 1 ? one : many}`;
const delta = (d: LineDelta) => `+${d.added} −${d.removed}`;
const BROWSE: Action = { id: "browse", label: "Browse all files" };
const OTHER: Action = { id: "other", label: "Open another changeset" };
const BACK: Action = { id: "back", label: "Back to all changes" };
const TIMELINE: Action = { id: "timeline", label: "Open timeline", primary: true };

/** What a view without a file says. The backend's reasons are tried from the most specific: the filter, then the view, then the changeset. */
export function emptyView(input: EmptyInput): EmptyView {
  if (input.excluded.length) return filtered(input);
  if (input.view === "commit") return emptyCommit(input);
  if (input.view === "range") return emptyRange(input);
  if (input.changesetBase.containedIn) return contained(input);
  if (input.key.type === "workingTree") return emptyWorkingTree(input);
  const base = input.changesetBase.commit;
  return {
    glyph: "emptyMinus",
    title: "No changes",
    lead: "Nothing differs from the base.",
    panels: [{ rows: [{ label: "Base", value: [mono(base.shortId)], meta: base.subject }] }],
    actions: [BROWSE],
  };
}

// ---- the filter leaves everything out ----

/** The name of the rule that leaves files out, as a panel's heading. */
export function ruleName(by: ExcludedBy): string {
  switch (by.kind) {
    case "narrowing":
      return "Outside the narrowing";
    case "outsideRoots":
      return "Outside review roots";
    case "notIncluded":
      return "Not included";
    case "pattern":
      return `Match ${by.pattern}`;
    case "generated":
      return "Generated";
    case "binary":
      return "Binary";
  }
}

/** The names of the rules that together leave files out. */
export const rulesName = (by: readonly ExcludedBy[]): string => by.map(ruleName).join(", ");

/** The rule in a word or two, where a row has little room. */
export function shortRule(by: ExcludedBy): string {
  switch (by.kind) {
    case "narrowing":
      return "narrowing";
    case "outsideRoots":
      return "outside roots";
    case "notIncluded":
      return "not included";
    case "pattern":
      return "pattern";
    case "generated":
      return "generated";
    case "binary":
      return "binary";
  }
}

/**
 * The filter that shows what `by` leaves out, for this review only: the project's filter without those
 * rules. None for a narrowing, which ends instead.
 */
export function lifting(project: ReviewFilter, by: readonly ExcludedBy[]): ReviewFilter | undefined {
  if (by.some((b) => b.kind === "narrowing")) return undefined;
  return by.reduce<ReviewFilter>((filter, b) => {
    switch (b.kind) {
      case "outsideRoots":
        return { ...filter, roots: [] };
      case "notIncluded":
        return { ...filter, include: [] };
      case "pattern":
        return { ...filter, exclude: filter.exclude.filter((p) => p !== b.pattern) };
      case "generated":
        return { ...filter, reviewGenerated: true };
      case "binary":
        return { ...filter, hideBinary: false };
      case "narrowing":
        return filter;
    }
  }, project);
}

/** The files of a group listed in its panel before the rest are counted. */
const LISTED = 5;

function filtered(input: EmptyInput): EmptyView {
  const narrowed = input.excluded.find((g) => g.by.some((b) => b.kind === "narrowing"));
  const count = input.excluded.reduce((sum, g) => sum + g.count, 0);
  const lines = input.excluded.reduce((sum, g) => sum + g.delta.added + g.delta.removed, 0);
  const panels: Panel[] = [];
  if (narrowed) {
    panels.push({
      heading: "Changed outside it",
      rows: narrowed.folders.map((f) => ({
        value: [mono(f.path)],
        meta: `${plural(f.files, "file")} · ${delta(f.delta)}`,
        action: {
          id: "narrow",
          label: "Review only this",
          filter: f.dir
            ? { ...input.filter, roots: [f.path] }
            : { ...input.filter, roots: [], include: [`/${f.path}`] },
        },
      })),
    });
  }
  for (const group of input.excluded) {
    if (group === narrowed) continue;
    const lift = lifting(input.filter, group.by);
    const rows: Row[] = group.files.slice(0, LISTED).map((file) => ({
      value: [mono(file.path)],
      meta: delta(file.delta),
      open: { id: "openFile", label: file.path, path: file.path },
    }));
    if (group.count > rows.length) rows.push({ value: [`+ ${group.count - rows.length} more`] });
    panels.push({
      heading: rulesName(group.by),
      count: group.count,
      link: lift && !input.narrowing ? { id: "narrow", label: "Review these", filter: lift } : undefined,
      rows,
    });
  }
  if (input.narrowing) {
    const roots = input.narrowing.roots;
    const where = roots.length ? roots.join(", ") : "what it is narrowed to";
    return {
      glyph: "emptyFilter",
      title: `Nothing in ${where}`,
      lead: narrowed
        ? `This review is narrowed to ${where}. The ${plural(narrowed.count, "changed file")} ${narrowed.count === 1 ? "is" : "are"} elsewhere.`
        : `This review is narrowed to ${where}, and the project's filter leaves out everything that changed.`,
      panels,
      actions: [
        { id: "reset", label: "Reset to the project filter", primary: true },
        { id: "filter", label: "Edit review filter…" },
      ],
    };
  }
  return {
    glyph: "emptyFilter",
    title: "Everything is filtered out",
    lead: `${count === 1 ? "The changed file" : `All ${count} changed files`} (${plural(lines, "line")}) ${count === 1 ? "is" : "are"} excluded by the review filter.`,
    panels,
    actions: [
      { id: "filter", label: "Edit review filter…" },
      { id: "show", label: "Show in the tree" },
    ],
  };
}

// ---- a commit or a range without a difference ----

function emptyCommit(input: EmptyInput): EmptyView {
  const parent = input.base.commit;
  const at = input.commitIndex;
  const parentIndex = input.commits.findIndex((c) => c.commit.id === parent.id);
  const rows: Row[] = [
    {
      label: "Parent",
      value: [mono(parent.shortId), ` ${parent.subject}`],
      action: parentIndex >= 0 ? { id: "step", label: "Open", n: parentIndex } : undefined,
    },
  ];
  const hasNext = at !== undefined && at + 1 < input.commits.length;
  return {
    glyph: "emptyCommit",
    title: "This commit changes no files",
    lead: "Its tree is identical to its parent's.",
    panels: [{ rows }],
    actions: [...(hasNext ? [{ id: "next", label: "Next commit", primary: true } as const] : []), BACK],
  };
}

function emptyRange(input: EmptyInput): EmptyView {
  const range = input.interdiff;
  const version = (n: number) => input.versions.find((v) => v.n === n);
  const rows: Row[] = [];
  if (range) {
    for (const n of range.from === 0 ? [range.to] : [range.from, range.to]) {
      const v = version(n);
      if (!v) continue;
      rows.push({
        label: `v${n}`,
        value: [shortTime(v.at, input.now, "detailed"), ...(v.tip ? [" ", mono(v.tip.shortId)] : [])],
        meta: v.base ? `based on ${v.base.shortId}` : undefined,
      });
    }
  }
  if (range?.rebased) {
    rows.push({ label: "Why", value: ["The base moved on. Files and lines are unchanged."] });
  }
  const pushes = range
    ? range.from + 1 === range.to
      ? `Push v${range.to}`
      : `Pushes v${range.from + 1}–v${range.to}`
    : "These pushes";
  return {
    glyph: "emptyRange",
    title: `${pushes} changed nothing`,
    lead: range?.rebased
      ? `The changes are the same as before; the base moved on.`
      : "Nothing differs between the versions compared.",
    panels: rows.length ? [{ rows }] : [],
    actions: [TIMELINE, BACK],
  };
}

// ---- the changeset itself is empty ----

/** The version that `version` took back: the one before it, when `version` changed the same files by the lines it had added and removed. */
function takenBack(versions: readonly VersionInfo[], version: VersionInfo): number | undefined {
  const before = versions.find((v) => v.n === version.n - 1);
  if (!before || !version.files.length || version.files.length !== before.files.length) return undefined;
  const swapped = version.files.every((f) => {
    const was = before.files.find((b) => b.path === f.path);
    return was?.delta.added === f.delta.removed && was.delta.removed === f.delta.added;
  });
  return swapped ? before.n : undefined;
}

function emptyWorkingTree(input: EmptyInput): EmptyView {
  const undone = input.review.state === "open" && input.versions.length > 1;
  if (undone) {
    const rows: Row[] = [...input.versions].reverse().map((v) => {
      const total = v.files.reduce(
        (sum, f) => ({ added: sum.added + f.delta.added, removed: sum.removed + f.delta.removed }),
        {
          added: 0,
          removed: 0,
        },
      );
      const names = v.files.map((f) => f.path.slice(f.path.lastIndexOf("/") + 1)).join(", ");
      const undid = takenBack(input.versions, v);
      return {
        label: `v${v.n}`,
        value: [shortTime(v.at, input.now, "detailed"), ...(v.files.length ? [" ", mono(delta(total))] : [])],
        meta: names ? (undid ? `${names} · undid v${undid}` : names) : v.n === 1 ? "opened" : undefined,
        action: v.files.length ? ({ id: "pick", label: "Show", n: v.n } as const) : undefined,
      };
    });
    return {
      glyph: "emptyMinus",
      title: "No changes left",
      lead: "The working tree matches what it was when opened. The edits made since were undone.",
      panels: [{ heading: "Versions", rows }],
      actions: [TIMELINE, BROWSE],
    };
  }
  const rows: Row[] = [];
  if (input.repo) rows.push({ label: "Watching", value: [mono(input.repo)], live: true });
  const base = input.changesetBase.commit;
  rows.push({
    label: "HEAD",
    value: [...(input.target.branch ? [mono(input.target.branch), " "] : []), mono(base.shortId)],
    meta: base.subject,
  });
  rows.push({ label: "Compared with", value: ["HEAD when opened"], meta: "commits made on top do not move it" });
  return {
    glyph: "emptyWatch",
    title: "No changes yet",
    lead: "Vet is watching this folder. A file appears here about a second after it stops changing.",
    panels: [{ rows }],
    actions: [BROWSE],
  };
}

function contained(input: EmptyInput): EmptyView {
  const held = input.changesetBase.containedIn;
  if (!held) return emptyWorkingTree(input);
  const pr = input.pullRequest;
  const isPr = input.key.type === "pullRequest";
  const provider: Action[] = pr
    ? [{ id: "provider", label: `Open on ${platformName(pr.platform.kind)}`, primary: true }]
    : [];
  const name = input.target.name;
  const head = input.target.head?.slice(0, 7);

  if (held.sameTip) {
    const tip = input.changesetBase.commit;
    const rows: Row[] = [
      {
        label: isPr ? "Source" : "Branch",
        value: [mono(isPr ? (pr?.sourceBranch ?? name) : name), ...(head ? [" ", mono(head)] : [])],
      },
      { label: "Target", value: [mono(held.branch), " ", mono(tip.shortId)], meta: tip.subject },
    ];
    if (pr) rows.push({ label: "Opened by", value: [pr.authorName] });
    return {
      glyph: "emptyMinus",
      title: isPr ? "No commits to review yet" : "No commits yet",
      lead: isPr
        ? `The source branch is at the same commit as ${held.branch}. Vet refreshes when the author pushes.`
        : `${name} has not moved off ${held.branch}. Commits made on it show up here.`,
      panels: [{ rows }],
      actions: [...provider, ...(isPr ? [OTHER] : [BROWSE])],
    };
  }

  if (pr?.state === "merged") {
    const rows: Row[] = [
      {
        label: "Source",
        value: [mono(pr.sourceBranch)],
        meta: head ? `head ${head} is in ${held.branch}` : undefined,
      },
    ];
    if (held.mergedBy) {
      rows.push({
        label: "Merge commit",
        value: [mono(held.mergedBy.shortId), ` ${held.mergedBy.subject}`],
        meta: `${held.mergedBy.author} · ${shortTime(held.mergedBy.time, input.now, "detailed")}`,
      });
    }
    if (input.threads.total > 0) {
      const state = input.threads.open ? `${input.threads.open} still open` : "all resolved";
      rows.push({
        label: "Your review",
        value: [`${plural(input.threads.total, "thread")}, ${state}`],
        action: { id: "comments", label: "Comments" },
      });
    }
    return {
      glyph: "emptyCheck",
      title: "Merged",
      lead: `#${pr.number} is merged into ${held.branch}, so it no longer adds anything.`,
      panels: [{ rows }],
      actions: [...provider, OTHER],
    };
  }

  const rows: Row[] = [];
  if (held.mergedBy) {
    rows.push({ label: "Merged by", value: [mono(held.mergedBy.shortId), ` ${held.mergedBy.subject}`] });
    rows.push({
      label: "When",
      value: [`${held.mergedBy.author} · ${shortTime(held.mergedBy.time, input.now, "detailed")}`],
    });
  }
  if (head) {
    rows.push({
      label: isPr ? "Head" : "Branch tip",
      value: [mono(head)],
      meta: held.commits === undefined ? undefined : `${plural(held.commits, "commit")}, all in ${held.branch}`,
    });
  }
  if (held.since !== undefined) {
    rows.push({
      label: `${held.branch} since`,
      value: [`${plural(held.since, "commit")} ahead of the merge`],
    });
  }
  return {
    glyph: "emptyMerge",
    title: `Already in ${held.branch}`,
    lead: `Every commit of ${name} is part of ${held.branch}, so there is nothing left to review.`,
    panels: rows.length ? [{ rows }] : [],
    actions: [...provider, ...(isPr ? [OTHER] : [BROWSE])],
  };
}

/** The version a click on a pushes row of the timeline shows: what `chooseRow` says for that version alone. */
export const pickRange = (versions: readonly VersionInfo[], n: number) => chooseRow(versions, undefined, n);
