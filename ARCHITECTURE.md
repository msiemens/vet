# Architecture

What Vet does is in `SPEC.md`; the UI is built as `ui/ARCHITECTURE.md` says. This file is the backend: the Rust crates under `crates/`, how they depend on each other and where each concept lives. The design notes in `notes/` give the reasons behind the larger pieces (`design-changesets.md`, `design-semantics.md`).

## Crates

The dependency direction is enforced by `ci/check-deps.sh` (an allowlist per crate; the pure crates may not pull in IO, git, a database or an async runtime).

| Crate           | Holds                                                                                                                                                                                                                                  | May depend on                                      |
| --------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------- |
| `vet-core`      | Pure domain model (I1, I3): content-addressed snapshots, diffs and hunks, the review model (spans, anchors, threads), remapping, completion, interdiff and version diffs, the review filter. Knows nothing of git refs, files or time. | nothing                                            |
| `vet-protocol`  | The shell-backend protocol: requests, responses, events, DTOs. `ts-rs` generates `ui/src/generated` from it (`cargo test -p vet-protocol`).                                                                                            | nothing                                            |
| `vet-transport` | The authenticated loopback WebSocket: a per-launch token and the protocol version in the first frame; requests are handled concurrently.                                                                                               | protocol                                           |
| `vet-highlight` | Syntax highlighting on tree-sitter: text and language in, kinds with ranges out.                                                                                                                                                       | core                                               |
| `vet-semantic`  | The semantic layer (SPEC §7): the server pool, primitive queries, derived analyses, the reuse layer. Owns the traits it consumes (`Materializer`, `ServerLauncher`, `LanguageServer`). No async runtime.                               | core, highlight                                    |
| `vet-app`       | Use cases over ports: acquisition, the live session, bases, versions and interdiffs, settling, fetch, pull requests, platform accounts, progress, the semantic wiring. Modules call each other directly, so ordering is plain awaits.  | core, semantic                                     |
| `vet-infra`     | Adapters, one port each: git (gix), the SQLite store (sea-orm), the file watcher, language-server processes and their restore, kept materialized trees, the GitHub and Azure DevOps adapters, credential validation.                   | core, semantic, app                                |
| `vet-mapping`   | `vet-app` views to protocol DTOs, and the request enums back.                                                                                                                                                                          | core, protocol, semantic, app                      |
| `vet-backend`   | The composition root: opens projects (`open.rs`), wires the adapters, serves each request (`lib.rs` and one module per area), turns session changes into events.                                                                       | all of the above but `vet-mock`                    |
| `vet-mock`      | A mock backend over a real git changeset, for the shell spike (SPEC §12.0).                                                                                                                                                            | core, protocol, transport, highlight, app, mapping |

## Snapshots and the store

A snapshot is a content-addressed tree (`SnapshotId`, a Merkle id over its entries). Where it came from (a commit, the live tree) lives in `Acquisition` and nowhere else (I1): the review model and the semantic layer see snapshots only. `Acquisition` reads blobs through git or, for content git does not hold, from the store.

The SQLite store (`vet-infra/src/store`, migrated at open) keeps:

- `snapshot`, `snapshot_path`, `tree_node`, `tree_child`: retained snapshots and their trees.
- `changeset`: one row per reviewed thing, unique by `(repo, kind, key)`: kind working tree, branch (key is the name) or pull request (key is `host/path#number`).
- `version`: a changeset's states, numbered per changeset: base and target snapshot, base commit, tip commit, time, origin (`Settled`, `BaseMoved`, `Observed`).
- `version_file`: for each version the files it changed against the version before it, with lines added and removed. They are written with the version, and the version list serves them, so the timeline needs no diff per row.
- `review`: how the base was chosen (`MergeBase` of a reference, or a commit), when it opened and closed, `last_reviewed` (the version something was last marked in), whether it finished complete.
- `span`, `anchor`, `ack`: the marks of a review; `thread`, `message`: threads, which belong to the changeset and outlive a review.
- `project_filter`, `project_filter_item`: the repository's review filter; `server_path`, `recent_project`: settings.

Spans are recorded as marked and never remapped in place; remapping starts from them whenever the target changes, and its result lives in memory. `refs/vet/changesets/<changeset>/<n>/…` keep each version's commits alive in git; `vet-app/src/gc.rs` decides what the store keeps alive (SPEC §8).

## The live session

`vet-app/src/session.rs` documents the lifecycle. A project is _browsing_ until the first review action (a mark, an acknowledgement, a thread) starts a _review_ at version 1; from then on each settled snapshot and each base change is the next version. Finishing returns to browsing and keeps the review's marks in the store.

- **Revision:** a counter that increases with every settled snapshot, base or filter change, changeset switch and finished review. Requests about positions carry the revision they were made at and fail as stale otherwise. `Revisions` (a `watch` of the newest revision) lets semantic work for an older revision be dropped.
- **One changeset on view per project** (`Source`): the working tree, a local or remote-tracking branch, or a pull request. `OpenChangeset` parks the state of the one on view and takes up the other's; `ChangesetOpened` tells every subscriber, which fetches the manifest again.
- **Settling** (`settle.rs`): the watcher (`vet-infra/src/watch.rs`) reports `Dirty` paths, `MustRescan` and `RefsMoved`; a quiet period turns them into a settled scan of the worktree. `.git/HEAD` and `.git/index` force a rescan. A change under `.git/refs` or `packed-refs` (a commit on any branch, a fetch) is `RefsMoved`: the session looks at the branch on view again (`observe`), and if its tip or merge base differs from the last seen, the target and base follow and, with a review open, the change is the next version (origin `Observed`). A settled tree is announced only while the working tree is on view.
- **Targets:** the working tree's target is the live tree. A branch's or pull request's target is its tip commit, built from git as a snapshot and never settled; the semantic layer serves it from a materialized snapshot.

### Bases

`BaseChoice` is either `MergeBase { reference }` or `Commit { id }`; resolved against the changeset's tip it gives the base commit (`base.rs`). A chosen branch is a target: the base is its merge base with the changeset's tip, never its tip, so the review shows what the changeset adds. When the merge base is the tip itself, the target already contains the changeset: the manifest's `Base::contained_in` names it and the changeset is empty. A merge-base choice reports how many commits its branch has gained since (`behind`), from the branch or its remote-tracking ref as of the last fetch. The default is the current branch's tip for the working tree, the merge base with the repository's default branch for a branch, and the merge base with the fetched target branch for a pull request. `ListBaseCandidates` offers local and remote-tracking branches and commits.

### Versions and views

`Versions` lists the open review's versions with their `version_file` deltas. Every request that reads or marks the changeset's files carries a `View`: `Changeset` (the changeset as it is) or `Interdiff { from, to }` (`session/interdiff.rs`, on `vet_core::version_diff` and `interdiff`). An interdiff compares version `from`'s target, rebased onto the base of version `to`, with `to`'s target, so what only a base move explains drops out. It is cached per `(revision, view)`, has no review state of its own, and a mark in it is recorded in the changeset of the newer version (which must be the latest), so seen state carries over. Threads shown in a range are those of its newer version's side.

### Changeset listing

`ListChangesets` is local: the working tree first, every local branch, then the remote-tracking branches that have commits the default branch lacks and were committed to recently (always the one on view and any with a review open), each group by last commit. A row carries the branch the working tree is on, ahead and behind counts, open threads, and what Vet has of the review (`RowReview`: none, open with version and `last_reviewed`, finished). `ListPullRequests` asks the platforms (below) and answers separately, so the local list never waits for the network.

## Pull requests and platforms

A pull request is a changeset (`ChangesetKey::PullRequest { remote, number }`). `vet-app/src/pull_request.rs` names its refs: the head is fetched from `refs/pull/<n>/head` into `refs/vet/pr/<repository>/<n>/head` and the target branch into `.../<n>/target`, apart from every branch and remote-tracking ref of the user's. The base is the merge base of the head with the fetched target, as of each push; a push that a later fetch brings is the next version, announced as `SnapshotSettled`. Opening one fetches first, as a task, and fails with a message fit to show. Vet never writes `refs/heads/*`, the index, the worktree or `HEAD`.

The platform port (`vet-app/src/platform/port.rs`: `traits`, `viewer`, `list_pull_requests`, `pull_request`) is the only platform knowledge outside the adapters. `vet-infra/src/platform/github` (on `octocrab`: GraphQL for lists, conditional REST for the rest) and `azure_devops` (on `azure_devops_rust_api`; iterations are pushes) implement it. `host.rs` and `remote.rs` decide which platform a host is and which repository a remote URL names; `queue.rs` decides which pull requests belong on the list and what a row shows, purely.

Credentials (`platform/accounts.rs`, per host and not per session) are held in memory only, tried in order: the token of a signed-in `gh` or `az` (`cli.rs`), then the token the shell hands over from its keychain (`SetCredentials`; the backend asks with the `CredentialsNeeded` event), validated against the platform when entered by the viewer. `vet-infra/src/credentials.rs` asks the platform whom a token belongs to. A token goes to its host's API and nowhere else, and nothing logs a request.

`fetch.rs` keeps remote-tracking refs current: every remote, as one background task, on demand (`Fetch`) and every five minutes while a project is open; afterwards `BaseChanged` carries the new `behind`.

## The semantic layer

`vet-semantic` is pure; `vet-infra` implements its ports over language-server processes. Servers are pooled by `(target, backend, policy)`, kept warm and evicted least recently used. The live tree is served in place; every other snapshot is served from a directory the `Materializer` produced. A policy that forbids executing project code refuses the backends that would.

**Reuse (I2).** An answer is reused only under a key that names everything it depends on (`reuse.rs`). `Key::Text` is for answers that depend on one text, the symbols a document declares: the server's identity, the file extension and the text's blob id, valid on any tree and either side. `Key::Project` is for answers that resolve across files: the server's identity, a `Basis` (snapshot, analysis policy, restore fingerprint, or the numbered server instance when nothing was recorded of the restore) and the query. The live tree has no project key, so its answers are asked fresh. The server's identity (`ServerLauncher::identify`) covers binary, version, settings and toolchain, so a changed server misses on every key and nothing needs deleting. `Answers` is a bounded in-memory LRU with a single-flight slot per key; an answer is admitted only if the server was idle when asked and its readiness epoch did not change before it answered, and an error is never kept. Every answer carries its provenance (the server, and whether it came from the cache).

**Changed symbols** are derived from keyed primitives alone. `derived::changed_symbols` takes the base and target text of each changed file through the `Texts` port (`vet-app/src/semantics.rs` implements it over the acquisition's blobs) and asks `text_symbols` for each, so a revision that changed one file asks a server for that file, and one that changed none (a filter, a finished review) asks for none. Progress is reported per file as a task. Impact, reachability and removal impact are derived from the changed symbols and project-keyed primitives.

**Revisions.** `Semantics` runs a request's work through `Revisions::until_superseded`; once a newer revision is known the work is dropped, with the server requests it waits on (a dropped request is cancelled with `$/cancelRequest`), and the request fails as stale.

**Kept trees.** Materialized trees live under Vet's data directory, one per `(snapshot, policy)`, built in staging and renamed so a directory under a key is complete (`vet-infra/src/materialize/kept.rs`). Locks outside the tree are shared by users and exclusive for the gc, which removes trees beyond a disk budget or idle for too long. Each language's restore is recorded beside the tree with a fingerprint, so a later launch starts its servers without copying or restoring again.

## Events and background work

The backend pushes `SnapshotSettled`, `ChangesetOpened`, `ReviewChanged`, `ReviewStarted`, `ThreadsChanged`, `BaseChanged`, `RestoreFailed`, `ServersChanged`, `ServerInstallOutput` and, to every connection, `CredentialsNeeded`; a client refetches what a revision change invalidates. Long work (scan, diff, restore, server start and indexing, the changed-symbols analysis, fetch, comparing versions) reports as `Task`s through `vet-app/src/progress.rs`, throttled, and `Tasks` answers a connection that joined late with the running ones.

## Where things go

- A rule about what a review is: `vet-core`, with tests on plain values.
- A use case: a module of `vet-app` over ports it declares; the adapter for a new port goes in `vet-infra`.
- A new request or event: `vet-protocol`, then `vet-mapping` for its views and `vet-backend` for the handler, and the typed call in `ui/src/state/backend.ts`.
- A semantic query: a primitive on `LanguageServer`, a key for it in `reuse.rs`, a derived analysis in `vet-semantic/src/derived`.
- A new hosting platform: an adapter of the port in `vet-infra/src/platform`, its host and remote parsing in `vet-app/src/platform`.
