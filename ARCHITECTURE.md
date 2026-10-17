# Architecture

What Vet does is in `SPEC.md`; the UI is built as `ui/ARCHITECTURE.md` says. This file is the backend: the crates under `crates/`, what each owns, and how a project runs through them.

## What must always hold

A rule that carries out an invariant of `SPEC.md` §2 names it; the others are architecture rules.

- **Git is read-only.** Vet reads the object graph and the filesystem in-process (gix, no git subprocesses or hooks) and fetches into `refs/vet/*` only; it never writes `refs/heads/*`, the index, the worktree or `HEAD`.
- **Review state is content-anchored.** Spans and anchors are recorded as marked and are not remapped in place; remapping starts from the recorded spans whenever the target changes, uses exact mapping only, and its result lives in memory. Content a remap cannot place exactly is left unseen (I2).
- **Trust gates code execution.** Whether servers and restores may execute project code is the analysis policy (`SPEC.md` §6.5), part of the key of every kept tree, restore record and cached answer, so a fetch-only answer is never reused as an executing one.
- **The semantic layer knows no git.** It sees content-addressed snapshots (`SnapshotId`, a Merkle id over its entries) and materialized directories; a snapshot's provenance lives in `Acquisition`. No function of the semantic layer takes a commit hash, a ref name or an origin.
- **Semantic primitives come only from language servers** (I3): derived analyses are labelled and recomputed from their inputs, and an operation a server does not offer is answered as not offered, never as empty. Cache keys cover every input (I1).
- **The window is a thin client.** All product logic sits behind the transport; the shell port touches rendering only, and no decision in the backend depends on the window.
- **Writers are opaque.** Vet learns everything from git and the filesystem and never detects which tool or person changed a file.
- **The dependency direction is enforced**: `ci/check-deps.sh` holds an allowlist per crate, and the pure crates (`vet-core`, `vet-protocol`, `vet-engine`, `vet-semantic`) pull in no IO, git, database or async runtime.

## Building blocks

The backend is layered: `vet-core` holds the domain, `vet-protocol` the protocol, `vet-app` the use cases over ports it declares, `vet-infra` the adapters, and `vet-backend` the composition root. Modules call each other directly, so ordering is plain awaits, and a use case does not reach around a port.

| Crate           | Owns                                                                                                                                                                                                 | May depend on                               |
| --------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------- |
| `vet-core`      | Pure domain model: snapshots, diffs and hunks, the review model (spans, anchors, threads), remapping, completion, interdiff and version diffs, filters.                                              | nothing                                     |
| `vet-protocol`  | The shell–backend protocol: requests, responses, events, DTOs; `ts-rs` generates `ui/src/generated` (`cargo test -p vet-protocol`).                                                                  | nothing                                     |
| `vet-log`       | The log subscriber: a daily rolling file, and stderr in debug builds.                                                                                                                                | nothing                                     |
| `vet-engine`    | Derived work computed once per key: shared flights, retention classes. The semantic layer and the review's diffs run on it.                                                                          | nothing                                     |
| `vet-host`      | The Tauri host's window-free logic: the backend child and its respawn policy, WSL path translation, the browser allow-list, the credential-host index.                                               | protocol                                    |
| `vet-transport` | The authenticated loopback WebSocket: framing, request handling, the outbox.                                                                                                                         | protocol                                    |
| `vet-highlight` | Syntax highlighting on tree-sitter: text and language in, kinds with ranges out.                                                                                                                     | core                                        |
| `vet-semantic`  | The semantic layer: the server pool, primitive queries, derived analyses, reuse. Owns the traits it consumes.                                                                                        | core, highlight, engine                     |
| `vet-app`       | Use cases over ports: acquisition, the session, bases/versions/lenses, settling, listing, search, fetch, pull requests, accounts, sync, progress, semantics.                                         | core, semantic, engine                      |
| `vet-infra`     | Adapters: git (gix, including merge remerge), the SQLite store, the watcher, the regex matcher, the language-server processes and restores, kept trees, GitHub and Azure DevOps, the semantic cache. | core, semantic, app, engine, log            |
| `vet-mapping`   | `vet-app` views to protocol DTOs, and the request enums back.                                                                                                                                        | core, protocol, semantic, app               |
| `vet-backend`   | The composition root: opens projects, wires adapters, serves requests, turns session changes into events, hosts the platform client.                                                                 | every crate but `vet-engine` and `vet-host` |

### The review model

- A **snapshot** is a content-addressed tree. Its provenance lives in `Acquisition`; the review model and the semantic layer see snapshots only.
- A **changeset** is what one thing adds on top of a base: the working tree, a branch, or a pull request. One is on view per project.
- The **base** is fixed by the changeset's kind and never chosen, so a queue row and its view agree. A branch's is the merge base with the default branch (its own tip when there is none); a pull request's is the merge base with the fetched target. A base follows its branch when that moves, never its tip, so the review shows what the changeset adds.
- The working tree's base is the `HEAD` it was opened at, pinned while browsing (so it survives a restart) and cleared when a review finishes; commits on top do not move it. `Base::head_off_base` says when `HEAD` no longer builds on the base, because the changes then include work from elsewhere; `Base::contained_in` says when the base already holds the tip, and the changeset is empty.
- A **review** numbers its states from version 1 and discards the changeset's earlier versions; version 0 is the virtual base it started from, not stored. Each version stores the files it changed against the version before, so the timeline needs no diff per row.
- A **view** is what a request reads or marks: the changeset, an interdiff between two versions, or one commit. An interdiff compares `from`'s target, rebased onto `to`'s base, with `to`'s target, so what only a base move explains drops out; the engine keeps it under the four snapshots it compares, not per revision.
- A branch or pull request is read in a **lens**: `Squash` (All changes, the default) or `PerCommit`, one commit at a time. A merge is compared with the automatic merge of its parents, so it shows what the author resolved.
- A **span** records a seen or flagged range as marked and is never remapped in place; remapping from the recorded spans uses exact mapping only, so a seen region split by an edit becomes seen parts around unseen ones (I2). A span records the commit whose diff it was marked in; comments are always made in the changeset.
- An **anchor** records its side, snapshot, file and range, with hashes of the range's exact bytes, of its normalized text (LF line endings, trailing whitespace stripped) and of the normalized lines around it.
- A comment is remapped straight from its snapshot to the one shown. Exact first, the first that works: the file unchanged at its path with equal bytes; the range mapped through the diff, renames included, with equal bytes; the same with equal normalized text (`Exact { whitespace_changed }`). Otherwise candidates come from the context window within the file and from the content of the other changed files, scored together: one clear winner is `Inferred` with its `Strategy` (`ContextWindow`, `CrossFileMove`) and confidence, several close ones `Ambiguous`. Rewritten lines fall back to the new lines between the nearest unchanged neighbours (`Inferred { ContextWindow, Low }`), deleted ones to where they were (`Inferred { Deleted, Low }`).
- A span seen against an earlier base counts as unseen until re-confirmed; the base-side lines as they were when it was marked can be inspected. A flag is a span too, kept out of the seen coverage and not degrading.

## How it runs

### Opening a project, through to the first settle

1. `OpenProject` (`vet-backend/src/projects.rs`) resolves the repository root and registers the project with the connection's subscribers.
2. `open.rs` wires the project's adapters: the git repo and store on their lanes' budgets, the watcher, the platform clients, and the semantic lane.
3. Watching starts before the initial scan, so no edit falls between them; the scan runs in the background, and requests that need the root wait for it.
4. The project opens on the queue, browsing the live diff against its base, with nothing stored.
5. The first scan settles into the changeset's first snapshot: the revision moves on and the backend announces `SnapshotMoved`.

### An edit becoming a new version

1. The watcher (`vet-infra/src/watch.rs`) reports `Dirty` paths, `MustRescan` or `RefsMoved`; `.git/HEAD` and `.git/index` force a rescan, and a change under `.git/refs` or to `.git/HEAD` is `RefsMoved`.
2. `Settling` (`vet-app/src/settle.rs`) turns a burst of writes into one settled scan after the tree is quiet; reconciliation is metadata-first, hashing only uncertain entries.
3. The settled tree refreshes the session's `HEAD` and, with a review open, records it as the version's `tip` and makes the snapshot the next version.
4. Review state is remapped from the recorded spans onto the new revision, in memory.
5. The backend announces `SnapshotMoved`, with the open review's version; a settled tree is announced only while the working tree is on view, and `EditsPending` says whether edits wait to become a version.
6. A `RefsMoved` makes the session look at the changeset on view again: for a branch, if the tip or merge base differs, the target and base follow and the change is the next version; a pull request gains one version per push its platform lists.

What makes the watcher ask for a full reconciliation:

| Platform                    | Watcher                 | Reconciles on                                                        |
| --------------------------- | ----------------------- | -------------------------------------------------------------------- |
| macOS                       | FSEvents                | `MustScanSubDirs`, `UserDropped`/`KernelDropped`, `RootChanged`      |
| Windows                     | `ReadDirectoryChangesW` | buffer overflow (a zero-byte result), an invalid handle              |
| Linux / WSL (ext4)          | inotify                 | `IN_Q_OVERFLOW`; too many watches switches to polling, with a notice |
| WSL, repository on `/mnt/*` | polling only            | every poll                                                           |

### A review action reaching the store and every window

1. The request's handler calls a session command; the first action starts the review at version 1 (`ReviewStarted`), later ones change it (`ReviewChanged`, whose `reviewed_at` says when the mark moved where the review last was, so a window reads the versions again only then).
2. The command takes the write token (`LiveSession::hold`, FIFO) and gets a `Held`: the token and a copy-on-write state. Reading is free; the first change copies, cheaply, because snapshots and spans sit behind `Arc`s; publishing or dropping the hold makes the copy the state readers see.
3. Writes go only through `ReviewWrites` with the store's write connection, which the token guards; reads use a pool and never wait.
4. The command announces its own events. `vet-backend/src/contract.rs` classifies every request as a query, unannounced (with the reason), or a mutation with its events, exhaustively, and its test drives every mutation with two connections.
5. Every event reaches every subscriber of the project, the requester included: the requester's own store reads the answer, and the others learn from the event alone.
6. An answer or event is applied only if it is newer than what the client holds (`State.seq`); a manifest never overwrites a newer mark.
7. A reader takes the published pointer and gets what the state lacks from the engine without a lock, taking the token only to install it (a cold start, an evicted value).

### A pull request fetched into versions

1. A pull request is a changeset; opening one fetches into Vet's own refs first (`refs/vet/pr/…`), as a task, and fails with a message fit to show. GitHub's head is `refs/pull/<n>/head` (forks included); Azure DevOps's is the source branch, or the head commit id when the branch is in a fork.
2. The base is the merge base of the head with the fetched target; each push the platform lists is a version, the newest announced as `SnapshotMoved`. A push whose commits the platform dropped is a version without content. A background fetch keeps every remote current, on demand and every five minutes while a project is open, and announces `ChangesetsChanged`.
3. The title and draft are stored with the changeset when the platform tells of them, so a restored view is headed without asking again. What the platform last said besides its threads is answered from the store first; details whose `updatedAt` changed are fetched ahead, paced, and stopped while the account is limited.
4. The platform port (`vet-app/src/platform/port.rs`) is the only platform knowledge outside the adapters (`vet-infra/src/platform/github`, `azure_devops`); `hosting.rs` forwards to the adapter of the pull request on view, so no handler above the port names a platform. Each platform describes itself as `PlatformTraits` (`changeset_threads`, `conversation_feed`, `posting`, `verdicts`, `verdict_separate`, `blocking`, `revision_scoped`; `SPEC.md` §7.2), which the UI branches on. A linked thread keeps the platform's id and status, whether it is resolvable and whether it blocks the merge.
5. Sync reads the platform's threads in common terms, decides as data what each status and comment becomes, and applies it; the platform is asked with the session's lock released. A viewer's status change wins; an edit is sent at once; an edit made on both sides is kept as two texts until the viewer chooses; a deleted thread is removed with a notice for the drafts lost.
6. Each outgoing comment is stored as a draft, posting or failed, written as posting before the platform is asked, so a post Vet did not see through is reconciled at the next sync. A batched platform posts all drafts as one review; a per-comment platform posts one by one and stops at the first refusal. A vote kept apart from posting is staged and sent after the comments.
7. The backend keeps the checks of the commits on view fresh itself and pushes them; no check or log is stored. Credentials are per account, in memory only, tried in order (a signed-in `gh`/`az`, then the shell's keychain token), validated when entered; a token goes to its host's API and nowhere else. Only the shipped backend looks for `gh`/`az` on `PATH` (`Config::cli_dirs`); a backend built from `Config::default()`, as the tests do, finds none and uses only the credentials it is given.

### A semantic question answered

1. `vet-semantic` is pure; `vet-infra` implements its ports over language-server processes.
2. The request runs through `Revisions::until_superseded`: work for an older revision is dropped, its server requests cancelled (`$/cancelRequest`), and the request fails as stale.
3. The semantic lane reads a `RevisionView`, a clone of the acquisition and the published pointer. It cannot take the write token or reach the review database, so it never makes a command wait.
4. Servers are pooled by target, backend and policy, kept warm and evicted by cost; a start is one shared flight per pool key and generation, and each server has a request budget. The pool supervises what it started: a server that dies is started again once, and one that dies again stays stopped until a manual restart, which the window is told. A policy that forbids executing project code refuses the backends that would. A server that fails to start repeats its failure for a backoff that starts at 5 s and doubles per failure in a row, up to 5 min; an install, a new path or a restart clears it.
5. Cache keys (I1):
   - a file's symbols: the server fingerprint, the file extension and the text's blob id; kept in memory and on disk;
   - hover, definition, references, call hierarchy and diagnostics on a materialized snapshot: the server fingerprint, snapshot, policy, restore and query; kept in memory and on disk;
   - workspace symbols on a materialized snapshot: the same key, in memory only;
   - the live tree has no project key, so its project-wide answers are asked fresh;
   - derived results are not kept; they are recomputed from the cached primitives over the diff.

   Persisted answers live in a disposable `semantic-cache.sqlite` (WAL), apart from the review database.

6. An answer is admitted only if the server was idle and its readiness did not change; errors are never kept. An operation the server does not offer is answered as not offered before the server is asked (I3).
7. The live tree is served in place. Questions are asked at one revision's positions, so the server gets that revision's file texts, kept like a review's content from the texts the diff read; a file Vet has no text of is read from disk by the server. An answer naming such a file that changed on disk after the revision settled is dropped and asked again at the next revision, once per revision, while the caller waits; a file Vet read nothing of for the revision cannot be checked and stays in the answer.
8. Every other snapshot is served from a kept tree the `Materializer` built, with each language's restore recorded beside it and checked against the current tools and policy before reuse. A kept tree in use by a running Vet is locked.
9. Restores run nothing by default (`cargo fetch`, and the package managers with `--ignore-scripts`, `--ignore-pnpmfile`, Yarn's `--mode=skip-build` with scripts off, uv's `--no-build`); the same flags hold where the policy allows executing, and only `dotnet restore` (MSBuild) needs that policy. When `npm ci` refuses a lock file out of sync with `package.json`, `npm install --no-save` with the same flags stands in. A failed step reports the part of the output that explains it (npm's error without its usage text, otherwise the first and last lines).
10. The backend analyses the changeset's files itself while the review screen shows it, open files first, pushing symbols and diagnostics as they come; a file whose analysis fails for a passing reason is tried three times with a short pause. Derived analyses (changed symbols, calls, usages, reachability, removal impact) are recomputed from the cached primitives over the diff. Languages and their servers are in `vet-infra/src/language.rs` (`SPEC.md` §6.1). rust-analyzer and `roslyn-language-server` are pinned versions; the TypeScript and Svelte servers run over tsserver; the JSON server comes from `vscode-langservers-extracted`.

### Listing the queue

1. `ListChangesets` answers locally: the working tree, every local branch, then recent remote-tracking branches, each group by last commit. A row carries what Vet has of the review (none, open, finished).
2. `ChangesetLines` counts a row's changed lines from the changeset's own base, a few rows at a time, pushing each count as it is made.
3. Each open project has one pull request feed (`vet-app/src/pull_request_feed/`, wired in `vet-backend/src/feed.rs`). One cycle lists the platforms, prunes the refs of pull requests no longer listed and fetches the listed heads; the details are then read ahead. The cycles are runs of the project's supervisor, one at a time, and the head fetch and the details read ahead are child runs of it. The fetch timer, the `Fetch` request and an account change ask it to list again. The timer and the first read of an empty feed join a cycle under way. A `Fetch` or an account change during a cycle gets one more cycle after it. A changed list replaces the details read ahead; an unchanged one leaves them running. A timer or account-change cycle skips an account that is rate limited. A repository whose listing fails keeps its last rows beside the error.
4. `ListPullRequests` answers from the feed's list, adding each row's review state and the details read ahead; only an empty feed waits for a listing. A different list is announced as `PullRequestListChanged`. The pull request on view is in the list even when the platform leaves it out: opening it hands the feed the row.

### A text search answered

1. `Search` answers the lines of the files in a scope that a pattern matches, grouped by file in path order, each found line carrying the review state of its hunk.
2. The pure part (`vet-app/src/search.rs`) decides which lines are looked at and matches them through the `LineMatcher` port, which `vet-infra` implements with the `regex` crate; a rejected pattern fails with a message fit to show.
3. The scan stops at 1000 matches and yields between files, so a cancelled request stops reading and others get their turn. `Search`, `ListFiles` and `WorkspaceSymbols` are queries in the contract.

## Shared concerns

### Concurrency and budgets

- Three lanes own their resources and share only the engine: the review commands, the semantic lane, and the platform lane (serialized by the session's exchange lock, with the state released while the platform is asked).
- A command holding the token never awaits a language server, a platform or another lane's budget; settling works its content out before it takes the token, and derives diffs, interdiffs and commits without touching the state.
- Derived work runs on `vet-engine`: computed once per key however many callers ask, kept by count and bytes, one flight per key, dropped when no waiter is left. Long work reports as throttled `Task`s. The working tree's copy into the store is kept per base and target snapshot, since content leaves the store only in the collection at store open.
- Every way off the async threads draws on a bound (`vet-infra/src/budget.rs`): review I/O, semantic I/O, background I/O, probes, restores. A lane cannot take another's permits, so a saturated semantic lane leaves the review's reads alone; a fetch has its own budget.
- `crates/clippy.toml` bans a bare `spawn_blocking`, `thread::spawn`, `Command::new` and a raw mutex lock; blocking work and processes go through `budget` and `exec`, the two modules that allow them, and the narrow exceptions are local (the transport's stdin thread, the host's backend child, the fake platform, each crate's lock helper, tests that build fixtures). The lint covers the workspace crates, not `ui/src-tauri`.
- `vet-backend/tests/it/load.rs` checks that with every semantic request stuck, review requests answer.
- Each open project's background work runs on one supervisor (`vet-app/src/supervise.rs`), which counts the runs per kind, isolates a panic to its run, and at close ends everything. The policy (`vet-backend/src/supervisor.rs`) decides what runs; `reconcile.rs` carries it out.
- Keyed work follows the state:
  - the analysis (symbols, problems): revision, policy, servers, a rerun, while the review screen is shown;
  - server starts: queued, never two at once; the snapshots, policy, servers, while the review screen is shown;
  - retiring servers a policy no longer allows: the snapshots, policy, on any screen; a retire that can't read the trust stops every server that may execute project code;
  - the checks: the snapshots, an action on a run, an account change, for a pull request, on any screen;
  - the copy ahead: asked for by a quiet tree or the screen coming up, while the review screen is shown; the queries on the working tree ask through the same flight, and a stored pair costs nothing.

  "The snapshots" is a count of snapshot moves kept beside the revision and read with it (`LiveSession::position`). A filter change moves the revision but not the snapshots, so it restarts only the analysis, whose files the filter decides. A kind starts when its key changes and stops when none is wanted. An ended run is not restarted for the same key, so a failure is not retried in a loop.

- Reconciling happens in a loop on the revision and the inputs, and after every request for the projects it looked up, so a request's background work is decided before its reply.
- Triggered work merges its requests:
  - the sync: at most one runs and one waits, and a ticket is answered only by a run that began after its request;
  - the fetch: a request joins the fetch under way, and the stronger asker's follow-ups apply;
  - the pull request feed: as above.

  The sync, the fetch and the settle loop cooperate with closing. They are never aborted for a newer request, finish their step, and get `Config::shutdown_bound` (5 s) before they are aborted.

### Events and the protocol

- The transport is one authenticated WebSocket on loopback: a `Hello` carrying the per-launch token and protocol version is the first frame, requests are handled concurrently and can be cancelled, and replies and events share one ordered outbox per connection.
- The outbox holds 10 000 messages; a send that finds it full closes the connection rather than grow, and the window reconnects and resyncs. Nothing is replayed.
- The backend pushes project events (`SnapshotMoved` (the working tree settled, the branch on view moved or a fetch brought the pull request on view a push; with the open review's version), `FilterChanged` (the filter, an excluded path or a narrowing moved the revision, no snapshot), `ReviewFinished`, `ChangesetOpened`, `ReviewChanged`, `ReviewStarted`, `LensChanged`, `TargetChanged`, `ChangesetsChanged`, `PullRequestListChanged` (the feed published a different list), `PullRequestsChanged` (sent once the prefetch or a sync read details that the platform's list leaves out, carrying what they add to the rows, so the queue patches them), `ChangesetLinesChanged`, `ThreadsChanged`, `EditsPending`, `FileSymbols`, `FileDiagnostics`, `ChecksChanged`, `RestoreFailed`, `ServersChanged`, `ServerRuntimes` (with what each language's server offers), `ServerInstallOutput`, `PlatformStatus`, `Task`, `Notification`) to a project's subscribers, and process events (`CredentialsNeeded`, `AccountsChanged`, `RecentProjectsChanged`, `SettingsChanged`, `PreferenceChanged`) to every connection that asked to `Subscribe`.
- A **revision** increases with every settled snapshot, observed move, filter change, changeset switch and finished review. A request about positions carries the revision it was made at and fails as stale otherwise, and `Revisions` lets semantic work for an older one be dropped.
- `WaitForQuiet` answers once the project has no background work at all, ordered after the earlier events on the connection: it has the settle loop look at the tree, reconciles, and waits until no run but the settle loop is alive, no progress task runs and the settle loop has nothing pending.

### Storage and retention

- The store is SQLite in WAL mode (`vet-infra/src/store`), migrated at open. It keeps snapshots and their trees; changesets, their versions and the files each version changed; the review's spans, anchors and threads; trust; the last thing a platform said of each pull request (at most 256, least recently read dropped first); the review filter; and the backend's settings and the window's preferences.
- Content is content-addressed, written temp → fsync → rename and referenced only once durable. The schema changes only by migrations applied in order, so an older database is upgraded in place and a newer one is never opened or deleted.
- `vet-app/src/gc.rs` decides what is kept: a changeset expires once its last review closed longer ago than the retention (30 days by default, adjustable) unless a review, thread or pending vote is open; content and snapshots have an hour of grace. Session refs (`refs/vet/sessions/<session-id>/{base,target}`) keep each version's commits alive in git; a pull request no longer listed, whose refs were fetched, has them dropped unless it is on view or its review is open.
- One contract of the store ports (`vet-app/src/store_contract.rs`) runs against the in-memory fake and against `SqliteStore`; the settings and gc ports run against the real store alone.
- The semantic cache is one disposable SQLite file under a viewer-set budget; kept trees are one per snapshot and policy under a disk budget, the settings showing the use and a Clear action.

### Errors

- A request answers with a `Response` or a `Failure`: an `ErrorKind` (`Outdated`, `NotFound`, `Invalid`, `Platform`, `Unavailable`, `Internal`) and a message fit to show. The client acts on the kind and shows the message; it never reads the message to tell kinds apart.
- The backend keeps the layers' errors as they are and maps them to a kind (`vet-backend/src/error.rs`), with its own context of what it was doing. A failure the viewer should know about reaches the window as a `Notification`.
- No request or response is logged; a failure is, at `warn`, with the account, endpoint, status and the platform's reason, and never a token.

### Logging

- `vet-log` installs the log subscriber, in the backend and in the shell: a daily rolling file (five days kept) through a non-blocking writer, plus stderr in debug builds; `VET_LOG` sets the level in `RUST_LOG` syntax.
- The transport opens a span per request, so every line of a handler carries the request's context; timing is `debug` under the target `timing`.
- The UI sends uncaught errors and failed requests as `ClientMessage::Log` under the target `ui`; `LogDirectory` points at the files. Nothing at `info` or above carries a token, a credential or file content.

## Where new code goes

- A rule about what a review is: `vet-core`, with tests on plain values.
- A use case: a module of `vet-app` over ports it declares; the adapter for a new port goes in `vet-infra`.
- A new request or event: `vet-protocol`, then `vet-mapping` and `vet-backend` for the handler, its classification in `contract.rs`, and the typed call in `ui/src/state/backend.ts`.
- A change to review state: a session command that takes the write token and publishes; a derived value: a `vet-engine` kind awaited without the token. Blocking work and processes go through `budget` and `exec`.
- A new store table: an entity and a migration in `vet-infra/src/store`, behind the store ports in `vet-app/src/ports.rs`.
- A semantic query: a primitive on `LanguageServer`, a key in `reuse.rs`, a `Feature` if servers may lack it, a derived analysis in `vet-semantic/src/derived`.
- A new language server: a `Language` in `vet-infra/src/language.rs` (binary, file types, root markers, whether it executes project code (assumed yes unless known), its install and restore, its setup in `lsp/setup.rs`), then a check of which features the server offers. Special cases for a server are added one by one, for problems actually seen.
- A new hosting platform: an adapter of the port in `vet-infra/src/platform`, its host and remote parsing in `vet-app/src/platform`.
