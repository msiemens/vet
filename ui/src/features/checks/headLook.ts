// What the toolbar shows for the head commit's checks: the button's title and whether what it shows
// may be old, and the Pipelines rail's count. The data is the checks store's and the platform's.

import type { CommitChecks } from "../../generated/CommitChecks";
import type { PlatformStatus } from "../../generated/PlatformStatus";
import { ciIcon, headerTitle } from "../../lib/ciStatus.ts";
import { clock } from "../../lib/format.ts";
import { platformName } from "../../lib/platformName.ts";
import { badgeCount } from "./checkRows.ts";

export interface HeadChecksLook {
  head: CommitChecks;
  /** What the checks say, for the button's tooltip. */
  title: string;
  /** The platform limits requests, so what is shown may be old. */
  stale: boolean;
}

/**
 * The head commit's checks as the toolbar's button shows them, or nothing while the commit has none.
 * When the platform limits requests, the title says until when and as of when; signed out, it says
 * the token is not valid.
 */
export function headChecksLook(
  head: CommitChecks | undefined,
  status: PlatformStatus | undefined,
  signedOut: boolean,
  limitedUntil: number | undefined,
  loadedAt: number | undefined,
): HeadChecksLook | undefined {
  if (!head) return undefined;
  const limited =
    limitedUntil === undefined || !status
      ? undefined
      : {
          until: clock(limitedUntil),
          asOf: clock(loadedAt ?? limitedUntil),
          platform: platformName(status.kind),
        };
  const title = signedOut
    ? `${status && platformName(status.kind)}: the token for ${status?.host} is not valid. Checks are not loaded.`
    : headerTitle(head, limited);
  return { head, title, stale: limited !== undefined };
}

/** The Pipelines rail's count: what the head's checks are about, while they are not all passed. */
export function pipelineBadge(
  head: CommitChecks | undefined,
  signedOut: boolean,
): { count: number; text: string; bad?: boolean } {
  if (!head || signedOut || head.state === "success" || head.state === undefined) return { count: 0, text: "" };
  const count = badgeCount(head);
  return { count, text: `${count} ${ciIcon(head.state)?.word ?? "waiting"}`, bad: head.state === "failure" };
}
