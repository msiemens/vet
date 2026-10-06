import type { CommitInfo } from "../../generated/CommitInfo";

/** The commits where every whitespace-separated term of `query` occurs in the id, subject or author. */
export function searchCommits(commits: readonly CommitInfo[], query: string): CommitInfo[] {
  const terms = query.toLowerCase().split(/\s+/).filter(Boolean);
  if (!terms.length) return [...commits];
  return commits.filter((c) => {
    const text = `${c.id} ${c.subject} ${c.author}`.toLowerCase();
    return terms.every((t) => text.includes(t));
  });
}
