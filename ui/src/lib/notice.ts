// The words of the notice a failed request leaves with the viewer.

import { isAbort } from "./abort.ts";
import { errorText } from "./errorText.ts";
import { isKind } from "./requestError.ts";

/** Said when a request is refused or rejected because the views it came from are about to be replaced. */
export const STALE_NOTICE = "Changes arrived — try again";

/**
 * What to tell the viewer of a request that failed while `what` (the start of a sentence, without
 * its ending) was being done; none for a request that was given up on. A request made against a
 * revision that is gone is the same notice whatever it was doing: the views are being replaced and
 * the action is to be repeated.
 */
export function failureNotice(what: string | undefined, error: unknown): string | undefined {
  if (isAbort(error)) return undefined;
  if (isKind(error, "outdated")) return STALE_NOTICE;
  return what === undefined ? errorText(error) : `${what}: ${errorText(error)}`;
}
