// What the "Accounts" page shows per credential, derived from `Accounts`.

import type { Account } from "../../generated/Account";
import type { PlatformKind } from "../../generated/PlatformKind";

export type AccountStatus = "signedIn" | "rejected" | "unverified";

export interface AccountRow {
  /** One row per host and source. */
  key: string;
  host: string;
  platform: string;
  source: string;
  user?: string;
  status: AccountStatus;
  statusLabel: string;
  /** What the platform said when it did not accept the credential or could not be asked. */
  problem?: string;
  /** Why this credential matters or does not: a CLI's token is never stored, a token behind a CLI is not used. */
  note?: string;
  /** Only a personal access token is Vet's to remove. */
  removable: boolean;
}

const PLATFORMS: Record<PlatformKind, string> = { github: "GitHub", azureDevOps: "Azure DevOps" };
const CLI_NAMES: Record<PlatformKind, string> = { github: "GitHub CLI (gh)", azureDevOps: "Azure CLI (az)" };

const STATUS_LABELS: Record<AccountStatus, string> = {
  signedIn: "Signed in",
  rejected: "Rejected",
  unverified: "Not verified",
};

export function accountRow(account: Account): AccountRow {
  const { state } = account;
  const cli = account.source === "cli";
  let note: string | undefined;
  if (!account.inUse) note = "Not used: the signed-in CLI for this host comes first.";
  else if (cli) note = "Vet asks the CLI for a token when it needs one and never stores it.";
  return {
    key: `${account.host}/${account.source}`,
    host: account.host,
    platform: PLATFORMS[account.kind],
    source: cli ? CLI_NAMES[account.kind] : "Personal access token",
    user: account.user ?? undefined,
    status: state.type,
    statusLabel: STATUS_LABELS[state.type],
    problem: state.type === "signedIn" ? undefined : state.message,
    note,
    removable: !cli,
  };
}

/** Whether a signed-in `gh` or `az` supplies the credential of some host. */
export const usesCli = (accounts: Account[]): boolean => accounts.some((a) => a.source === "cli" && a.inUse);
