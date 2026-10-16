// The requests about the open project that belong to no opened changeset: the lists, the filter, the
// trust and the repository's folders. Each says why it failed, or answers `undefined`, as the caller needs.

import type { ChangesetKey } from "../generated/ChangesetKey";
import type { ReviewFilter } from "../generated/ReviewFilter";
import { errorText } from "../lib/errorText";
import { notices } from "./notices.svelte";
import { session } from "./session.svelte";

/** The changesets on offer and the one on view; `undefined` after showing why they could not be listed. */
export async function listChangesets() {
  try {
    return await session.connected().listChangesets();
  } catch (error) {
    notices.report(error);
    return undefined;
  }
}

/** The open pull requests on offer; `undefined` after showing why they could not be listed. */
export async function listPullRequests() {
  try {
    return await session.connected().listPullRequests();
  } catch (error) {
    notices.report(error);
    return undefined;
  }
}

/** The lines each of `keys` changes, in order; `undefined` where counting failed. */
export async function changesetLines(keys: ChangesetKey[]) {
  try {
    return await session.connected().changesetLines(keys);
  } catch {
    return undefined;
  }
}

/** Fetches the remotes. The footer shows the task; the lists follow with `changesetsChanged`. */
export async function fetchRemotes() {
  try {
    await session.connected().fetch();
  } catch (error) {
    notices.report(error);
  }
}

/**
 * Replaces the repository's review filter. The backend pushes `snapshotSettled`, which fetches the new
 * manifest. Resolves with the backend's message if it refused the filter (a bad glob).
 */
export async function setFilter(filter: ReviewFilter): Promise<string | undefined> {
  try {
    await session.connected().setProjectFilter(filter);
    return undefined;
  } catch (error) {
    return errorText(error);
  }
}

/** What each glob of the draft `filter` matches in the changeset on view; `undefined` if it could not be asked. */
export async function patternMatches(filter: ReviewFilter) {
  try {
    return await session.connected().patternMatches(filter);
  } catch {
    return undefined;
  }
}

/** The names of the folders directly in the repository's folder `path` (`""` is the root); `undefined` if there is no such folder. */
export async function folders(path: string) {
  try {
    const { entries } = await session.connected().listDirectory(path);
    return entries.filter((e) => e.kind === "dir").map((e) => e.name);
  } catch {
    return undefined;
  }
}

/** Marks the repository trusted or not. The backend then pushes `targetChanged`, which fetches the manifest that says so. */
export async function setProjectTrusted(trusted: boolean) {
  try {
    await session.connected().setProjectTrusted(trusted);
  } catch (error) {
    notices.report(error);
  }
}

/** Trusts the pull request on view at its current head; announced like `setProjectTrusted`. */
export async function trustPullRequest() {
  try {
    await session.connected().trustPullRequest();
  } catch (error) {
    notices.report(error);
  }
}

/** Excludes `path` (a file, or a folder with `dir`) from the review. As with `setFilter`, the backend then pushes `snapshotSettled`. */
export async function excludePath(path: string, dir: boolean) {
  try {
    await session.connected().excludePath(path, dir);
  } catch (error) {
    notices.report(error);
  }
}

/** The children of a directory of the target snapshot; `undefined` after showing why it failed. */
export async function listDirectory(path: string) {
  try {
    return await session.connected().listDirectory(path);
  } catch (error) {
    notices.report(error);
    return undefined;
  }
}
