// Typed requests over the connection. The only module besides `connection.ts` that
// knows the wire protocol, so a protocol change lands here.

import type { Account } from "../generated/Account";
import type { BackendSettings } from "../generated/BackendSettings";
import type { Base } from "../generated/Base";
import type { BaseChoice } from "../generated/BaseChoice";
import type { BaseDiff } from "../generated/BaseDiff";
import type { BlobContent } from "../generated/BlobContent";
import type { BranchInfo } from "../generated/BranchInfo";
import type { ChangedSymbols } from "../generated/ChangedSymbols";
import type { ChangesetKey } from "../generated/ChangesetKey";
import type { ChangesetRow } from "../generated/ChangesetRow";
import type { CheckLog } from "../generated/CheckLog";
import type { CommitChecks } from "../generated/CommitChecks";
import type { CommentAt } from "../generated/CommentAt";
import type { CommitInfo } from "../generated/CommitInfo";
import type { CommitUnit } from "../generated/CommitUnit";
import type { Diagnostic } from "../generated/Diagnostic";
import type { DirEntry } from "../generated/DirEntry";
import type { EditChoice } from "../generated/EditChoice";
import type { Event as ServerEvent } from "../generated/Event";
import type { Impact } from "../generated/Impact";
import type { Lens } from "../generated/Lens";
import type { LineDelta } from "../generated/LineDelta";
import type { Manifest } from "../generated/Manifest";
import type { NonTextKind } from "../generated/NonTextKind";
import type { PullRequestInfo } from "../generated/PullRequestInfo";
import type { PullRequestRow } from "../generated/PullRequestRow";
import type { PullRequestSource } from "../generated/PullRequestSource";
import type { Reachability } from "../generated/Reachability";
import type { RecentProject } from "../generated/RecentProject";
import type { RemovalImpact } from "../generated/RemovalImpact";
import type { Request } from "../generated/Request";
import type { Response } from "../generated/Response";
import type { ReviewFilter } from "../generated/ReviewFilter";
import type { ReviewSubmitted } from "../generated/ReviewSubmitted";
import type { ReviewUpdate } from "../generated/ReviewUpdate";
import type { SemanticLocation } from "../generated/SemanticLocation";
import type { ServerInfo } from "../generated/ServerInfo";
import type { Side } from "../generated/Side";
import type { SymbolId } from "../generated/SymbolId";
import type { SymbolSignature } from "../generated/SymbolSignature";
import type { Task } from "../generated/Task";
import type { TextContent } from "../generated/TextContent";
import type { Thread } from "../generated/Thread";
import type { ThreadStatus } from "../generated/ThreadStatus";
import type { ThreadTarget } from "../generated/ThreadTarget";
import type { Unavailable } from "../generated/Unavailable";
import type { VersionInfo } from "../generated/VersionInfo";
import type { View } from "../generated/View";
import { type Address, Connection, type Dialer, type LinkState } from "./connection";
import { reportUncaught } from "./errorLog";
import { hashParams, inTauri, invoke } from "./host";
import type { ServerLine } from "./serverStatus";
import { StaleRequests } from "./staleRequests";

export type { LinkState };

/**
 * In Tauri the host spawned the backend, read its Ready line and took the repository from its command
 * line; after the backend exited the host starts another and answers with its line. In a plain browser
 * (development only, not a supported target) pass both in the URL hash: `#port=1234&token=...&repo=/path`
 * (without `repo`, the start screen asks for one).
 */
const hostDialer: Dialer = {
  address: async (): Promise<Address> => {
    if (inTauri) return invoke<Address>("backend_connection");
    const params = hashParams();
    const port = Number(params.get("port"));
    const token = params.get("token");
    if (!port || !token) throw new Error("open with #port=<port>&token=<token> outside Tauri");
    return { port, token, repo: params.get("repo") };
  },
  socket: (url) => new WebSocket(url),
};

export type FileContents = Extract<Response, { type: "fileContents" }>;

/** The events that belong to the backend, not to a session. */
type ProcessEvent = Extract<ServerEvent, { type: "credentialsNeeded" | "accountsChanged" | "recentProjectsChanged" }>;

/** The events of a session. */
export type SessionEvent = Exclude<ServerEvent, ProcessEvent>;

const isProcessEvent = (event: ServerEvent): event is ProcessEvent =>
  event.type === "credentialsNeeded" || event.type === "accountsChanged" || event.type === "recentProjectsChanged";

/** A language server's Markdown, with the highlights of each of its fenced code blocks, in order. */
export interface HoverText {
  text: string;
  code: number[][];
  /** Why there is no text, when the backend knows. */
  unavailable?: Unavailable;
}

/** A hover's text with what the popup says around it: the file's server, and the key that goes to a definition. */
export interface HoverAnswer extends HoverText {
  server?: ServerLine;
  /** The label of the go to definition shortcut, if it has one. */
  definitionKey?: string;
}

export interface PullRequestList {
  /** Most recently updated first. */
  rows: PullRequestRow[];
  /** One per GitHub repository of the project, with the reason its pull requests could not be listed. */
  sources: PullRequestSource[];
}

export interface BaseCandidates {
  branches: BranchInfo[];
  /** Newest first. */
  commits: CommitInfo[];
}

function expect<T extends Response["type"]>(response: Response, type: T): Extract<Response, { type: T }> {
  if (response.type !== type) throw new Error("unexpected response");
  return response as Extract<Response, { type: T }>;
}

/** The `checkLog` response carries the log's fields beside its tag. */
const asCheckLog = (response: Extract<Response, { type: "checkLog" }>): CheckLog => response;

/** The `thread` response carries the thread's fields beside its tag. */
const asThread = (response: Extract<Response, { type: "thread" }>): Thread => response;

/** Likewise the `account` response. */
const asAccount = (response: Extract<Response, { type: "account" }>): Account => response;

export class Backend {
  /** The open project, if any; one at a time. */
  project?: number;

  /** The semantic requests underway, which the changeset moving on aborts. */
  private stale = new StaleRequests();

  private constructor(
    private conn: Connection,
    /** The repository the host (or the URL hash) named, if any. */
    readonly initialRepo: string | null,
  ) {
    conn.onEvent((event) => {
      if ((event.type === "snapshotSettled" || event.type === "changesetOpened") && event.project === this.project) {
        this.stale.moveTo(event.revision);
      }
    });
  }

  static async connect(): Promise<Backend> {
    const conn = await Connection.open(hostDialer);
    reportUncaught((level, message, context) => conn.log(level, message, context));
    return new Backend(conn, conn.repo);
  }

  /** Calls `listener` whenever the connection drops, stalls or is up again. */
  onLinkState(listener: (state: LinkState) => void): () => void {
    return this.conn.onState(listener);
  }

  /** Calls `listener` when the connection is up again after it dropped; the project is to be opened again. */
  onReconnect(listener: () => void): () => void {
    return this.conn.onReconnect(listener);
  }

  /** Tries to connect again at once, for after the backend was restarted. */
  retryConnection() {
    this.conn.retryNow();
  }

  /**
   * The recently opened repositories, most recent first. They belong to the backend, not to a project. Asking also
   * subscribes this connection to the backend's events.
   */
  async recentProjects(): Promise<RecentProject[]> {
    return expect(await this.conn.request({ type: "recentProjects" }), "recentProjects").projects;
  }

  /** Puts `path` at the top of the recent list; resolves with the list. */
  async recordRecentProject(path: string): Promise<RecentProject[]> {
    return expect(await this.conn.request({ type: "recordRecentProject", path }), "recentProjects").projects;
  }

  /** Resolves with the recent list without `path`. */
  async forgetRecentProject(path: string): Promise<RecentProject[]> {
    return expect(await this.conn.request({ type: "forgetRecentProject", path }), "recentProjects").projects;
  }

  /** Opens a project on `repo`; resolves with the repository root the backend settled on. */
  async open(repo: string): Promise<string> {
    const opened = expect(await this.conn.request({ type: "openProject", repo }), "project");
    this.project = opened.project;
    // The backend resolves a subfolder to its repository root and reports it.
    return opened.root;
  }

  async close(): Promise<void> {
    const project = this.project;
    this.project = undefined;
    if (project !== undefined) await this.conn.request({ type: "closeProject", project });
  }

  private get pid(): number {
    if (this.project === undefined) throw new Error("no repository is open");
    return this.project;
  }

  /** The events of the open repository's session. */
  onEvent(listener: (event: SessionEvent) => void): () => void {
    return this.conn.onEvent((event) => {
      if (!isProcessEvent(event)) listener(event);
    });
  }

  /**
   * Calls `listener` with a host the backend needs a credential for and has none; heard, like the other events of
   * the backend, after `accounts()` or `recentProjects()`.
   */
  onCredentialsNeeded(listener: (host: string) => void): () => void {
    return this.conn.onEvent((event) => {
      if (event.type === "credentialsNeeded") listener(event.host);
    });
  }

  /** Calls `listener` when a credential was given or removed, by this window or another. */
  onAccountsChanged(listener: () => void): () => void {
    return this.conn.onEvent((event) => {
      if (event.type === "accountsChanged") listener();
    });
  }

  /** Calls `listener` when the recent list changed, by this window or another. */
  onRecentProjectsChanged(listener: () => void): () => void {
    return this.conn.onEvent((event) => {
      if (event.type === "recentProjectsChanged") listener();
    });
  }

  /** The credentials the backend holds or can get, per host; asking also subscribes this connection to the backend's events. */
  async accounts(): Promise<Account[]> {
    return expect(await this.conn.request({ type: "accounts" }), "accounts").accounts;
  }

  /**
   * Gives the backend a personal access token, which it keeps in memory only. With `validate` the platform
   * is asked first and a refused token rejects with the backend's message. Resolves with the account, whose
   * host is the normalized one.
   */
  async setCredentials(host: string, token: string, validate: boolean): Promise<Account> {
    const request = { type: "setCredentials", host, token, validate } as const;
    return asAccount(expect(await this.conn.request(request), "account"));
  }

  /** Forgets the personal access token of a host; resolves with the accounts left. */
  async removeCredentials(host: string): Promise<Account[]> {
    return expect(await this.conn.request({ type: "removeCredentials", host }), "accounts").accounts;
  }

  /**
   * Removes the answers of the language servers that the backend keeps, and the kept copies of snapshots
   * nothing uses. Only speed is lost. Resolves with the bytes the semantic cache takes on disk afterwards.
   */
  async clearSemanticCaches(): Promise<number> {
    return expect(await this.conn.request({ type: "clearSemanticCaches" }), "semanticCacheUsage").bytes;
  }

  /** The bytes the answers of the language servers kept between launches take on disk. */
  async semanticCacheUsage(): Promise<number> {
    return expect(await this.conn.request({ type: "semanticCacheUsage" }), "semanticCacheUsage").bytes;
  }

  /** The backend's own settings, app-wide. */
  async settings(): Promise<BackendSettings> {
    return expect(await this.conn.request({ type: "settings" }), "settings").settings;
  }

  /** Replaces the backend's own settings; resolves with them as kept, rejects with the backend's message. */
  async setSettings(settings: BackendSettings): Promise<BackendSettings> {
    return expect(await this.conn.request({ type: "setSettings", settings }), "settings").settings;
  }

  /** The folder of the backend's log files, as a path of the machine it runs on. */
  async logDirectory(): Promise<string> {
    return expect(await this.conn.request({ type: "logDirectory" }), "logDirectory").path;
  }

  /** The background tasks of the open project that are running now. */
  async tasks(): Promise<Task[]> {
    return expect(await this.conn.request({ type: "tasks", project: this.pid }), "tasks").tasks;
  }

  /** The versions of the open review, oldest first; none while browsing. */
  async versions(): Promise<VersionInfo[]> {
    return expect(await this.conn.request({ type: "versions", project: this.pid }), "versions").versions;
  }

  /** The commits of the changeset on view with their own progress, oldest first; none for the working tree. */
  async commits(): Promise<CommitUnit[]> {
    return expect(await this.conn.request({ type: "commits", project: this.pid }), "commits").commits;
  }

  /** Remembers the lens the changeset on view is reviewed in; every subscriber, this one included, then gets `lensChanged`. */
  async setLens(lens: Lens): Promise<void> {
    expect(await this.conn.request({ type: "setLens", project: this.pid, lens }), "done");
  }

  /** The changeset, or with another `view` an interdiff of two of its versions or one commit. */
  async manifest(view: View): Promise<Manifest> {
    return expect(await this.conn.request({ type: "manifest", project: this.pid, view }), "manifest");
  }

  fileContents(path: string, view: View): Promise<FileContents> {
    return this.conn
      .request({ type: "fileContents", project: this.pid, view, path })
      .then((r) => expect(r, "fileContents"));
  }

  /** The direct children of a directory of the target snapshot (`""` is the root), as of `revision`. */
  async listDirectory(path: string): Promise<{ revision: number; entries: DirEntry[] }> {
    const { revision, entries } = expect(
      await this.conn.request({ type: "listDirectory", project: this.pid, path }),
      "directory",
    );
    return { revision, entries };
  }

  /** The target-side text of any path of the target snapshot, changed or not. */
  async fileText(path: string): Promise<{ revision: number; content: TextContent }> {
    const { revision, content } = expect(
      await this.conn.request({ type: "fileText", project: this.pid, path }),
      "fileText",
    );
    return { revision, content };
  }

  /** One side of a non-text item's file, as of `revision`. */
  async itemBlob(revision: number, path: string, kind: NonTextKind, side: Side, view: View): Promise<BlobContent> {
    const request = { type: "itemBlob", project: this.pid, view, revision, path, kind, side } as const;
    return expect(await this.conn.request(request), "itemBlob").content;
  }

  /**
   * Flips one hunk: seen becomes unseen, anything else seen. `revision` is the manifest's; the backend
   * rejects a stale one. In an interdiff the answer is in the terms of the interdiff.
   */
  async toggleHunk(revision: number, path: string, hunk: number, view: View): Promise<ReviewUpdate> {
    const request = { type: "toggleHunk", project: this.pid, view, revision, path, hunk } as const;
    return expect(await this.conn.request(request), "review");
  }

  /** Flips the acknowledgement of one non-text item. */
  async toggleItem(revision: number, path: string, kind: NonTextKind, view: View): Promise<ReviewUpdate> {
    const request = { type: "toggleItem", project: this.pid, view, revision, path, kind } as const;
    return expect(await this.conn.request(request), "review");
  }

  /** Marks everything of a file seen, or takes every mark back if it all is seen already. */
  async toggleFileSeen(revision: number, path: string, view: View): Promise<ReviewUpdate> {
    const request = { type: "toggleFileSeen", project: this.pid, view, revision, path } as const;
    return expect(await this.conn.request(request), "review");
  }

  /** Marks the hunks of a file seen against an old base seen again, against the base now. The changeset's file only. */
  async reconfirmFile(revision: number, path: string): Promise<ReviewUpdate> {
    const request = { type: "reconfirmFile", project: this.pid, revision, path } as const;
    return expect(await this.conn.request(request), "review");
  }

  /** The base-side lines of a hunk seen against an old base as they were when it was marked, as a diff to the base now. */
  async baseDiff(revision: number, path: string, hunk: number): Promise<BaseDiff> {
    const request = { type: "baseDiff", project: this.pid, revision, path, hunk } as const;
    return expect(await this.conn.request(request), "baseDiff");
  }

  /** The changesets on offer, the working tree first, and the one on view. */
  async listChangesets(): Promise<{ rows: ChangesetRow[]; onView: ChangesetKey }> {
    const { rows, onView } = expect(
      await this.conn.request({ type: "listChangesets", project: this.pid }),
      "changesets",
    );
    return { rows, onView };
  }

  /**
   * The open pull requests of the project's GitHub and Azure DevOps repositories,
   * and how asking each repository went. Asks the platform, so it takes a moment.
   */
  async listPullRequests(): Promise<PullRequestList> {
    const { rows, sources } = expect(
      await this.conn.request({ type: "listPullRequests", project: this.pid }),
      "pullRequests",
    );
    return { rows, sources };
  }

  /**
   * The lines a changeset changes against its base, for a row of the list; none for the working tree and for a
   * pull request that was not fetched. Takes a moment for a large changeset, so ask per row.
   */
  async changesetLines(changeset: ChangesetKey): Promise<LineDelta | undefined> {
    const { lines } = expect(
      await this.conn.request({ type: "changesetLines", project: this.pid, changeset }),
      "changesetLines",
    );
    return lines ?? undefined;
  }

  /**
   * The checks of the commits on the platform of the pull request on view: none for another changeset.
   * `error` says why the commits left out of `checks` could not be answered for.
   */
  async checks(commits: string[]): Promise<{ checks: CommitChecks[]; error?: string }> {
    const { checks, error } = expect(await this.conn.request({ type: "checks", project: this.pid, commits }), "checks");
    return { checks, error: error ?? undefined };
  }

  /** The log of a run of `checks` that has one. Rejects with the reason it could not be had. */
  async checkLog(run: string): Promise<CheckLog> {
    return asCheckLog(expect(await this.conn.request({ type: "checkLog", project: this.pid, run }), "checkLog"));
  }

  /**
   * Puts a changeset on view; every subscriber, this one included, then gets `changesetOpened`. A pull
   * request is fetched first, so this waits for the network and rejects with the reason it could not.
   */
  async openChangeset(changeset: ChangesetKey): Promise<void> {
    expect(await this.conn.request({ type: "openChangeset", project: this.pid, changeset }), "done");
  }

  /** Compares against another base; every subscriber, this one included, then gets `snapshotSettled`. */
  async setBase(base: BaseChoice): Promise<Base> {
    return expect(await this.conn.request({ type: "setBase", project: this.pid, base }), "base");
  }

  /** Replaces the open repository's review filter; every subscriber, this one included, then gets `snapshotSettled`. Rejects with the backend's message for an invalid glob. */
  async setProjectFilter(filter: ReviewFilter): Promise<void> {
    expect(await this.conn.request({ type: "setProjectFilter", project: this.pid, filter }), "done");
  }

  /** Excludes a file, or with `directory` everything below a folder, from the review; every subscriber, this one included, then gets `snapshotSettled`. */
  async excludePath(path: string, directory: boolean): Promise<void> {
    expect(await this.conn.request({ type: "excludePath", project: this.pid, path, directory }), "done");
  }

  /** Ends the open review; every subscriber, this one included, then gets `snapshotSettled` and `changesetsChanged`. Resolves with whether it was complete. */
  async finishReview(): Promise<boolean> {
    return expect(await this.conn.request({ type: "finishReview", project: this.pid }), "reviewFinished").complete;
  }

  /**
   * Fetches the repository's remotes and resolves when that ended; a remote that failed shows as a failed task, not
   * here. If any remote was fetched, every subscriber then gets `baseChanged` and `changesetsChanged`.
   */
  async fetch(): Promise<void> {
    expect(await this.conn.request({ type: "fetch", project: this.pid }), "done");
  }

  /** The branches and recent commits a base can be picked from. */
  async baseCandidates(): Promise<BaseCandidates> {
    const { branches, commits } = expect(
      await this.conn.request({ type: "listBaseCandidates", project: this.pid }),
      "baseCandidates",
    );
    return { branches, commits };
  }

  /**
   * A request about the changeset at `revision`. Aborting `signal` cancels it in the backend, down to the
   * language server, and so does the changeset moving to a newer revision; either rejects with an abort.
   */
  private async semantic(revision: number, request: Request, signal?: AbortSignal): Promise<Response> {
    const tracked = this.stale.track(revision, signal);
    try {
      return await this.conn.request(request, tracked.signal);
    } finally {
      tracked.release();
    }
  }

  /**
   * Hover at a 0-based line and UTF-16 column of `side`'s text at `revision`, the manifest's (the
   * backend rejects a stale one). `text` is empty if the server has nothing to say there.
   */
  async hover(
    revision: number,
    side: Side,
    path: string,
    line: number,
    col: number,
    signal?: AbortSignal,
  ): Promise<HoverText> {
    const request = { type: "hover", project: this.pid, revision, side, path, line, col } as const;
    const r = expect(await this.semantic(revision, request, signal), "hover");
    return { text: r.text ?? "", code: r.codeHighlights, unavailable: r.unavailable ?? undefined };
  }

  /** Where the symbol at the position is defined, in `side`'s snapshot; otherwise like `hover`. */
  async definition(
    revision: number,
    side: Side,
    path: string,
    line: number,
    col: number,
    signal?: AbortSignal,
  ): Promise<{ locations: SemanticLocation[]; unavailable?: Unavailable }> {
    const request = { type: "definition", project: this.pid, revision, side, path, line, col } as const;
    const r = expect(await this.semantic(revision, request, signal), "definition");
    return { locations: r.locations, unavailable: r.unavailable ?? undefined };
  }

  /**
   * The diagnostics of a file of the target side, and how many of them to tell the viewer about;
   * the language server may take seconds on a cold start.
   */
  async diagnostics(
    revision: number,
    path: string,
    signal?: AbortSignal,
  ): Promise<{ diagnostics: Diagnostic[]; count: number; unavailable?: Unavailable }> {
    const request = { type: "diagnostics", project: this.pid, revision, path } as const;
    const r = expect(await this.semantic(revision, request, signal), "diagnostics");
    return { diagnostics: r.diagnostics, count: r.count, unavailable: r.unavailable ?? undefined };
  }

  /** Each language's server as it resolves for the open repository. */
  async listServers(): Promise<ServerInfo[]> {
    return expect(await this.conn.request({ type: "listServers", project: this.pid }), "servers").servers;
  }

  /** Installs the pinned server; output arrives as `serverInstallOutput` events. Rejects with the tool's output tail. */
  async installServer(language: string): Promise<ServerInfo[]> {
    return expect(await this.conn.request({ type: "installServer", project: this.pid, language }), "servers").servers;
  }

  /** `null` removes the custom path: automatic resolution again. The path is not checked; `listServers` says whether it works. */
  async setServerPath(language: string, path: string | null): Promise<ServerInfo[]> {
    return expect(await this.conn.request({ type: "setServerPath", project: this.pid, language, path }), "servers")
      .servers;
  }

  /** The symbols the changeset touches, as of `revision` (the manifest's; a stale one is rejected). */
  async changedSymbols(revision: number, signal?: AbortSignal): Promise<ChangedSymbols> {
    const request = { type: "changedSymbols", project: this.pid, revision } as const;
    return expect(await this.semantic(revision, request, signal), "changedSymbols");
  }

  /** The declaration of a changed symbol; `signature` is unset if the server has nothing, `unavailable` says why. */
  async signature(
    revision: number,
    symbol: SymbolId,
    signal?: AbortSignal,
  ): Promise<{ signature?: SymbolSignature; unavailable?: Unavailable }> {
    const request = { type: "signature", project: this.pid, revision, symbol } as const;
    const r = expect(await this.semantic(revision, request, signal), "signature");
    return { signature: r.signature ?? undefined, unavailable: r.unavailable ?? undefined };
  }

  /** References and callers of an added or modified changed symbol; may take a while on a cold server. */
  async impact(revision: number, symbol: SymbolId, signal?: AbortSignal): Promise<Impact> {
    const request = { type: "impact", project: this.pid, revision, symbol } as const;
    return expect(await this.semantic(revision, request, signal), "impact");
  }

  /** What became of the references to a removed changed symbol. */
  async removalImpact(revision: number, symbol: SymbolId, signal?: AbortSignal): Promise<RemovalImpact> {
    const request = { type: "removalImpact", project: this.pid, revision, symbol } as const;
    return expect(await this.semantic(revision, request, signal), "removalImpact");
  }

  /** Whether an added or modified changed symbol is reached from an entry point. */
  async reachability(revision: number, symbol: SymbolId, signal?: AbortSignal): Promise<Reachability> {
    const request = { type: "reachability", project: this.pid, revision, symbol } as const;
    return expect(await this.semantic(revision, request, signal), "reachability");
  }

  /** Every thread with its anchor resolved at `revision`, the current one, in creation order. */
  async threads(view: View): Promise<{ revision: number; threads: Thread[] }> {
    const request = { type: "threads", project: this.pid, view } as const;
    const { revision, threads } = expect(await this.conn.request(request), "threads");
    return { revision, threads };
  }

  /**
   * The line of the target text that a unified diff shows `line` of `side` of the file at `path`
   * (its old path on the base side) at, as of `revision`, the manifest's (a stale one is rejected).
   */
  async targetLine(revision: number, side: Side, path: string, line: number, view: View): Promise<number> {
    const request = { type: "targetLine", project: this.pid, view, revision, side, path, line } as const;
    return expect(await this.conn.request(request), "targetLine").line;
  }

  /**
   * Where a comment on a part of the changeset file at `path` attaches, as of `revision`, the manifest's
   * (a stale one is rejected); `line` is the target line the form of a range target goes below.
   */
  async commentTarget(
    revision: number,
    path: string,
    at: CommentAt,
    view: View,
  ): Promise<{ target: ThreadTarget; line: number | null }> {
    const request = { type: "commentTarget", project: this.pid, view, revision, path, at } as const;
    const { target, line } = expect(await this.conn.request(request), "commentTarget");
    return { target, line };
  }

  /** `revision` is the manifest's; lines of a range target are in its text. */
  async createThread(revision: number, target: ThreadTarget, body: string, view: View): Promise<Thread> {
    const request = { type: "createThread", project: this.pid, view, revision, target, body } as const;
    return asThread(expect(await this.conn.request(request), "thread"));
  }

  async reply(thread: number, body: string): Promise<Thread> {
    const request = { type: "reply", project: this.pid, thread, body } as const;
    return asThread(expect(await this.conn.request(request), "thread"));
  }

  async setThreadStatus(thread: number, status: ThreadStatus): Promise<Thread> {
    const request = { type: "setThreadStatus", project: this.pid, thread, status } as const;
    return asThread(expect(await this.conn.request(request), "thread"));
  }

  async deleteThread(thread: number): Promise<void> {
    expect(await this.conn.request({ type: "deleteThread", project: this.pid, thread }), "done");
  }

  /** Edits one of the viewer's own comments; a posted one is edited on the platform too. */
  async editMessage(message: number, body: string): Promise<Thread> {
    const request = { type: "editMessage", project: this.pid, message, body } as const;
    return asThread(expect(await this.conn.request(request), "thread"));
  }

  /** Removes one of the viewer's own comments; removing a thread's first removes the thread. */
  async removeMessage(message: number): Promise<void> {
    expect(await this.conn.request({ type: "removeMessage", project: this.pid, message }), "done");
  }

  /** Settles a comment that was edited here and on the platform by keeping one of the texts. */
  async keepEdit(message: number, keep: EditChoice): Promise<Thread> {
    const request = { type: "keepEdit", project: this.pid, message, keep } as const;
    return asThread(expect(await this.conn.request(request), "thread"));
  }

  /** Edits one of the viewer's comments in the conversation of the pull request, on the platform. */
  async editConversationComment(comment: string, body: string): Promise<void> {
    const request = { type: "editConversationComment", project: this.pid, comment, body } as const;
    expect(await this.conn.request(request), "done");
  }

  /** Deletes one of the viewer's comments in the conversation of the pull request, on the platform. */
  async deleteConversationComment(comment: string): Promise<void> {
    const request = { type: "deleteConversationComment", project: this.pid, comment } as const;
    expect(await this.conn.request(request), "done");
  }

  /** Syncs the threads of the pull request on view with its platform now. */
  async syncThreads(): Promise<void> {
    expect(await this.conn.request({ type: "syncThreads", project: this.pid }), "done");
  }

  /** Posts the drafts as one review with the verdict (an id of `PullRequestInfo.traits.verdicts`). */
  async submitReview(verdict: string, body: string | null): Promise<ReviewSubmitted> {
    const request = { type: "submitReview", project: this.pid, verdict, body } as const;
    const { posted, skipped, voteCast, problem } = expect(await this.conn.request(request), "reviewSubmitted");
    return { posted, skipped, voteCast, problem };
  }

  /** Posts the drafts one after the other, then the staged vote, on a platform that takes comments that way. */
  async postDrafts(): Promise<ReviewSubmitted> {
    const request = { type: "postDrafts", project: this.pid } as const;
    const { posted, skipped, voteCast, problem } = expect(await this.conn.request(request), "reviewSubmitted");
    return { posted, skipped, voteCast, problem };
  }

  /**
   * Stages the viewer's vote (an id of `PullRequestInfo.traits.verdicts`) for the next post, on a platform that keeps
   * it apart from posting. The viewer's current vote clears the staged one instead.
   */
  async stageVote(verdict: string): Promise<void> {
    expect(await this.conn.request({ type: "stageVote", project: this.pid, verdict }), "done");
  }

  /** Drops the staged vote. */
  async discardVote(): Promise<void> {
    expect(await this.conn.request({ type: "discardVote", project: this.pid }), "done");
  }

  /** What the platform said of the pull request on view at the last sync; fails before the first. */
  async pullRequestInfo(): Promise<PullRequestInfo> {
    const request = { type: "pullRequestInfo", project: this.pid } as const;
    return expect(await this.conn.request(request), "pullRequestInfo");
  }

  /** Re-anchors an inferred thread where it is now. */
  async confirmAnchor(revision: number, thread: number): Promise<Thread> {
    const request = { type: "confirmAnchor", project: this.pid, revision, thread } as const;
    return asThread(expect(await this.conn.request(request), "thread"));
  }

  /** Re-anchors an ambiguous thread at one of its candidates (an index into its `ambiguous` anchor). */
  async attachAnchor(revision: number, thread: number, candidate: number): Promise<Thread> {
    const request = { type: "attachAnchor", project: this.pid, revision, thread, candidate } as const;
    return asThread(expect(await this.conn.request(request), "thread"));
  }
}
