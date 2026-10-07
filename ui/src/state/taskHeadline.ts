// What the footer says about the running tasks: the newest one, and how many others there are.

import type { Task } from "../generated/Task";

export interface Headline {
  /** The newest task's title. */
  title: string;
  /** The newest task's progress in 0..1; none when it cannot say. */
  fraction?: number;
  /** The other running tasks. */
  more: number;
}

/** How far a task is in 0..1; `undefined` when it reports no determinate progress. */
export function fractionOf(task: Task): number | undefined {
  const { done, total } = task;
  if (done === null || total === null || total <= 0) return undefined;
  return Math.min(1, Math.max(0, done / total));
}

/** `tasks` oldest first; nothing when none runs. */
export function headline(tasks: readonly Task[]): Headline | undefined {
  const newest = tasks.at(-1);
  if (!newest) return undefined;
  return { title: newest.title, fraction: fractionOf(newest), more: tasks.length - 1 };
}
