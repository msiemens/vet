# Vet — Core Spec

## 1. Product definition

Vet is a **review-first IDE** for the agentic coding age. The unit of work is the *changeset*, not the file. The human's role is *verifier*, not author. Agents run elsewhere; Vet is the checkpoint between agent output and merged code.

**Primary activities, in priority order:**

1. Review changesets (dirty trees, local commits, stacks, PRs)
2. Verify semantics (navigation, impact, reachability, base-vs-target queries)
3. Close the loop (annotations → platform comments or agent task files)

**Permanent non-goals:**

- Agent authoring/orchestration integration
- Harness-specific integrations or provenance inference (observation-only)
- Being a general-purpose authoring IDE
- Multi-user/team collaboration (personal tool)

**Threat model:** agents are non-adversarial but fallible. Vet guards against *accidents*, not against attacks. The one exception is contributor PRs, which Vet does not analyze with code-executing backends until you choose to trust them (§7.5).

**Platforms:** macOS, Windows, and WSL2, including the split where the shell runs on Windows and the backend runs in WSL (§4.1).

---

## 2. Core invariants — frozen

| # | Invariant | Test |
|---|---|---|
| **I1** | **The semantic layer never knows git.** Acquisition maps git state into immutable snapshots; provenance lives in the acquisition/review layer. | No semantic-layer API accepts commit hashes, ref names, or origin flags. |
| **I2** | **A semantic answer is reused only under a key that names everything it depends on.** Semantic answers come from language servers running against the live worktree or a complete, materialized snapshot. Every answer Vet keeps, in memory or on disk, is stored under a key built from the complete inputs of the query that produced it: the exact text it was asked about (by blob id) for answers that depend on one document; the side's snapshot, the analysis policy and the restore of its dependencies for answers that resolve across files; and always the identity of the server (binary, version, settings, toolchain, and the user-level configuration outside the repository it reads). An answer is reused if and only if a recomputed key is equal; validity is never judged by time, events or heuristics. Answers from the live worktree are reused only when they depend on the exact text Vet sent. Every cache can be deleted at any time without changing anything but speed. The server identity and the standing of a restore are observed once per generation (a launch, a settings change, an install, a server restart or clearing the caches starts a new one), as a running server already is, and are part of every key observed within it. | For every cached query kind, changing any one key input makes a fresh server query, and keeping all of them answers from the cache without one. The same scenario run with every cache disabled and with caching on gives identical results. An answer the server gave while it was not ready is never stored. Deleting the cache file and the materialized directories leaves review state and all results unchanged. |
| **I3** | **Annotations and review state are content-anchored, never line-anchored. Outcomes distinguish `Exact` / `Inferred` / `Stale` / `Ambiguous`. `Seen` applies only to content identical to the content that was reviewed — new or modified content is never silently marked seen.** | Annotations survive amend, rebase, force-push, renames, moves, agent edits — repositioned with recorded confidence, never silently orphaned. A seen region split by new edits yields seen parts for identical content and unseen parts for the rest. |
| **I4** | **All writers are opaque.** Everything is learned from observing the git object graph and filesystem. Never detect, parse, or infer harness behavior. | Identical behavior whether the writer is Claude Code, Aider, a human, or `sed`. |
| **I5** | **The shell is a thin client.** All product logic sits behind the orchestrator's IPC boundary. | A shell port touches rendering only. No backend, store, or acquisition decision may reference the shell. |
| **I6** | **Semantic *primitives* — types, references, diagnostics, call edges — come exclusively from authoritative language backends; there is no heuristic tier. Vet may build *derived analyses* over those primitives, but derived results are labeled as such and their derivation is inspectable.** | Every primitive traceable to a vendor backend. Derived claims display their inputs. Missing server capabilities fail loudly → language unsupported, never silently degraded. |

---

## 3. Object model

```
Snapshot          immutable, content-hashed view of a worktree
  └─ entries:     BTreeMap<Path, FileEntry{ blob, mode, kind }>  // kind: file | symlink | submodule
  └─ id:          hash over the sorted entry set
LogicalTip        an evolving tip (the dirty tree, a branch, a PR) whose value is a
                  time-ordered sequence of settled Snapshots. This sequence IS the
                  edit timeline.
Changeset         (base: SnapshotId, target: SnapshotId)
ReviewFilter      review roots + include/exclude patterns, set per repository. A
                  VIEW over a changeset; never part of snapshot identity.
Pin               recorded {head, tree_hash} when a review starts (version 1); the
                  dirty content of that snapshot copied into the content store
Anchor            content anchor for one range on one side (I3); used by threads
Thread            a comment thread on a line range, a file, or the whole changeset;
                  optionally linked to a platform thread
ReviewedSpan      the durable unit of review coverage: a range of content the user
                  marked seen, on one or both sides. Hunks are a display concept;
                  "hunk seen" is derived from spans. Flags and concerns are comments.
Session           a working tree on view against a base: browsing until a review
                  starts (§5.3), then an open review of one Changeset that owns
                  its pin, versions and review state
```

**Provenance separation (I1):** snapshots are pure content. Origin, `base_head`, branch/PR metadata, and timeline order live in the acquisition layer, keyed by `SnapshotId`.

---

## 4. Architecture

```
┌─────────────────────────────────────────────────┐
│ Shell (thin client — I5)                        │
│   Web UI (Svelte + CodeMirror 6) in Tauri       │
├──────── IPC boundary: WebSocket (§4.1) ─────────┤
│ Orchestrator (Rust)                             │
│  ┌──────────────┬───────────────┬─────────────┐ │
│  │ Acquisition  │ Semantic      │ Review      │ │
│  │ snapshots,   │ server pool,  │ anchors,    │ │
│  │ watcher,     │ queries       │ remap,      │ │
│  │ reconcile,   │               │ spans       │ │
│  │ pins, refs,  │               │             │ │
│  │ materializer │               │             │ │
│  └──────────────┴───────────────┴─────────────┘ │
│  ┌────────────────────────────────────────────┐ │
│  │ Platform adapters: GitHub, ADO (sync out)  │ │
│  ├────────────────────────────────────────────┤ │
│  │ Storage: content store + SQLite (§8)       │ │
│  └────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────┘
```

### 4.1 Topologies and IPC

The orchestrator, language servers, stores, and **the worktree under review** always live on the same side, the *backend side*. Only the shell may be elsewhere.

| Topology | Shell | Backend side | Status |
|---|---|---|---|
| macOS native | Tauri | macOS | supported |
| Windows native | Tauri | Windows | supported |
| WSL split | Tauri on Windows | WSL2, worktree on the Linux filesystem (ext4) | supported |
| WSL, worktree on `/mnt/*` | Tauri on Windows | WSL2 | **degraded**: watcher unreliable across 9P, polling reconciliation, performance targets void, visible banner |

**Transport:** one protocol over one transport, a WebSocket bound to loopback, in every topology. The shell spawns the backend (directly, or via `wsl.exe` in the split) and reads a per-launch random token from its stdout. Every connection must present the token. There is no unauthenticated endpoint. In the WSL split, Windows reaches the socket through WSL2 localhost forwarding.

**Why Tauri and not a plain browser:** browsers reserve shortcuts the JetBrains keymap needs (Ctrl+W closes the tab). Because the UI is a web bundle, it still loads in a browser for development, but that is not a supported target.

**Agents** may run on either side. Writes from Windows into the WSL filesystem (`\\wsl$`) are observed like any other write (I4).

---

## 5. Review model

### 5.1 Anchors (I3)

```rust
struct Anchor {
    side: Side,                    // Base | Target of the changeset it was created in
    snapshot: SnapshotId,
    file: Path,
    range: Range,
    content_hash_raw: Hash,        // exact bytes of the annotated range
    content_hash_norm: Hash,       // LF line endings, trailing whitespace stripped
    context_hashes: Vec<Hash>,     // surrounding lines, normalized
}

enum AnchorState {
    Exact { whitespace_changed: bool },
    Inferred { strategy: Strategy, confidence: Confidence },
    Stale { reason: StaleReason },           // context changed / file deleted
    Ambiguous { candidates: Vec<Location> }, // duplicate matches — distinct from Stale
}

enum Strategy { ContextWindow, CrossFileMove, Deleted }

enum CommentTarget {
    Range(Anchor),          // lines on one side; goes through anchor resolution
    File { side: Side, path: Path },
    Changeset,              // the changeset as a whole
}

struct Thread {
    id: ThreadId,
    target: CommentTarget,
    messages: Vec<Message>,          // author, body, time; first is the opener
    status: Open | Resolved,         // local status
    platform: Option<PlatformThread>,
}

struct PlatformThread {
    remote_id: String,
    remote_status: Option<String>,   // shown as-is, e.g. ADO "Won't fix"
    resolvable: bool,                // false for a flat conversation comment
    blocks_merge: bool,              // as reported by the platform (§11)
}
```

Only `Range` targets are resolved by the algorithm below and can become `Stale` or `Ambiguous`. `File` targets follow git-detected renames and become `Stale` only when the file is deleted; `Changeset` targets never detach.

**Resolution algorithm.** An anchor resolves into the same `side` of the changeset it is displayed against, directly from its recorded snapshot to the target snapshot (never through intermediate snapshots).

- **Exact strategies, ordered, first success wins:**
  1. Identical blob at the same path, raw hash verified over the range.
  2. Diff-mapped range (old → new file at the same or a git-detected renamed path) with raw-hash verification.
  3. Diff-mapped range matching only on the normalized hash → `Exact { whitespace_changed: true }`.
- **Inferred strategies, evidence-based:** collect candidates from both a context-window search within the file and a cross-file content match across the changeset's changed files (catches moves). One dominant candidate → `Inferred` with strategy and confidence. Competing plausible candidates → `Ambiguous`.
- **Following the hunk.** When the anchored lines changed and no better candidate exists, using the old → new line alignment, the anchor attaches to the lines of the new text between the nearest unchanged line before it and the one after it (the hunk that replaced them), however large: `Inferred { ContextWindow, Low }`. When nothing replaced the lines, they were deleted: the anchor sits on the line where they were, `Inferred { Deleted, Low }`, and the UI says "line deleted". Both show the "moved" badge and can be confirmed; they are never silent.
- **None → `Stale`** only when the file was deleted or no text around the anchor survived.
- Inferred candidates are scored together, never first-success-wins.

### 5.2 Review state

```rust
struct ReviewedSpan {
    base:   Option<Anchor>,   // reviewed base-side lines; None for pure additions
    target: Option<Anchor>,   // reviewed target-side lines; None for pure deletions
    state:  Seen | SeenAgainstOldBase | Flagged,
}
```

Marking a hunk (or a selection) seen records one `ReviewedSpan` covering its lines on both sides. `Seen` and `SeenAgainstOldBase` spans record coverage; flagging a concern is a comment (§5.1). A hunk shows as seen when every changed line in it is covered by a `Seen` span.

**Flags.** A hunk can be flagged for later review: a `Flagged` span over its lines on both sides, which covers nothing. A flag is independent of seen (a hunk can be both), is only removed by flagging again, and is never cleared by marking seen or unseen. Flags are stored with the review and remapped exactly as seen spans are, except that a flag never degrades: a hunk is flagged while any of its changed lines is covered by a `Flagged` span, so a flagged hunk that is split or edited by a push stays flagged. Finishing the review drops them with the other marks.

**Remapping is a partition (constitutional, per I3).** When the changeset changes, each span is remapped side by side using the line-level diff between its recorded file and the current file (following git-detected renames). Only Exact strategies apply. Inferred moves never carry review state. Each reviewed line whose content is identical and maps into the current changeset stays covered. Every other line is uncovered, so one span can become several spans, or none. If a reviewed line's mapping is not unique (for example, repeated identical lines whose surrounding context changed), it is uncovered rather than left to the diff algorithm's tie-breaking. A seen 20-line region that gets a 3-line edit becomes up to two seen spans around 3 unseen lines.

**Base changes.** If a span's target lines are still identical but any of its base lines changed, its descendants are marked `SeenAgainstOldBase`. They are rendered distinctly, count as **unseen** toward completion, and can be re-confirmed in one action.

This catches base changes **within the reviewed region only**. Semantic changes elsewhere surface through impact queries (§7.3), not review state. Losing review progress is acceptable; manufacturing false "already reviewed" confidence is not.

**Non-text changes.** A file-level change with no reviewable text (mode, symlink target, submodule commit, binary or oversized content, a pure rename, an added or deleted empty file) is one review item, acknowledged like a hunk and stored the same way, keyed by the before and after entries.

**Completion** is relative to the active `ReviewFilter`: a changeset is complete when every changed line that passes the filter is covered by a `Seen` span and every non-text change that passes it is acknowledged. The UI always shows how many changes the filter excludes, so "complete" never implies every change was seen.

### 5.3 Session lifecycle

- **Browsing:** opening a repository shows the live working tree against the base, and nothing is recorded: no versions, no retained content, no session refs. The base is a view setting.
- **Open:** a review starts implicitly with the first review action: marking something seen (span, hunk or file), acknowledging a non-text change, or creating a thread. Un-marking and reading do not start one. The snapshot current at that moment is **version 1**, the review's start snapshot; the pin records its `HEAD` and tree, and the chosen base carries over. Each later settled snapshot is the next version (v2, v3, …). Content retention and session refs (§6.1) apply from the start. A complete review (§5.2) stays open and keeps tracking.
- **Closed:** "Finish review" is an explicit action, allowed with changes still unseen; it records whether the review was complete. It releases the session refs and stops versioning. Further edits are browsing again until the next review action starts a new review. Closed sessions follow the retention rule (§8).
- **Resume:** opening a repository with an open review resumes it (the tree settles as the next version and review state remaps, §5.2); with none, it opens browsing.

---

## 6. Acquisition layer

### 6.1 Snapshots and scope

- **Snapshot contents:** a file is in a snapshot iff it is under the repository root **and** (tracked by git **or** untracked and not git-ignored). Files over the capture size cap (default 50 MB) are still hashed in full, so snapshot identity stays exact, but their content is not retained and they appear as `content unavailable`. Ignored paths can be opted in explicitly. The untracked contents of folders a tool owns by name (`.pnpm-store/`) are never part of a snapshot; this list is explicit and fixed. No relevance inference of any kind (I4). Executable-bit and symlink changes are first-class `(mode, kind)` entries. Submodules are recorded as pinned commits; descent is opt-in.
- **Review filter:** review roots, binary hiding, and user patterns are a `ReviewFilter` applied when *displaying* a changeset. It is a project setting, stored per repository and applied to every changeset of it. Changing it never changes snapshot identity, so it never disturbs anchors or review state.
  - **Generated files** are excluded like a matching exclude pattern, unless the filter's "review generated files" setting is on: lockfiles and tool output by file name (`Cargo.lock`, `yarn.lock`, `*.pb.go`, `*.min.js`, …, at any depth), files with `linguist-generated` set in `.gitattributes` (those of the side the file is on; `-linguist-generated` keeps a file in), and files whose first five lines (within the first KiB) contain `<auto-generated`, `@generated` or Go's `// Code generated … DO NOT EDIT.`.
- **Pin at review start (§5.3):** record `{head, tree_hash}` of version 1 and copy its dirty and untracked content into Vet's content store. Dirty content is **never** written into the git object database, so untracked secrets never become pushable objects. The changeset is the base to the latest settled snapshot; version 1 is the one current when the review started.
- **Content retention:** when a snapshot settles while a review is open, every blob of it that git cannot reconstruct (dirty and untracked content) is copied into the content store, so every snapshot on the session's timeline stays reconstructible. In addition, every blob that appears on either side of a displayed diff, or under an anchor or span, is copied when first displayed, so review state and anchors never depend on git keeping old objects alive.
- **Session refs:** while a review is open, it holds `refs/vet/sessions/<session-id>/{base,target}` on the commits its snapshots are built from, so they stay materializable. The refs are deleted when the review is finished.
- **Availability:** a snapshot is *materializable* only while every one of its blobs is available (from git or the content store). After that, semantic queries on it are disabled with a visible reason. Vet never starts a server on a partial tree.

### 6.2 Watcher, settling, and reconciliation

- **Watcher = latency optimization, never authority.** Filesystem events drive incremental re-hashing.
- **Settling:** after a write burst, a snapshot is *settled* once the touched files' size and mtime stay stable across a debounce window (about 1–2 s). Each settled snapshot is appended to its `LogicalTip`; this sequence is the edit timeline. Same-file concurrent edits within one window are unresolvable by design.
- **Authoritative reconciliation** runs at open, on watcher overflow or rescan signals (§6.4), on resume or restart, and before declaring a snapshot settled. It must *establish correct snapshot identity*, metadata-first (filesystem stat, git index, cached hashes), hashing only uncertain entries. Dropped or coalesced events can never corrupt review state, only delay it.

### 6.3 Platform-faithful entries

- **Git access goes through an in-process library (gitoxide).** Vet spawns no git subprocesses for acquisition and never runs hooks, `fsmonitor` commands, or filter or diff drivers.
- **Paths:** repository-relative, `/`-separated, keyed by their exact bytes (as git records them for tracked files, as the filesystem reports them for untracked ones). Unicode normalization is used only for display and for matching git's `core.precomposeUnicode` behavior on macOS. Paths that differ only in case or only in normalization are flagged `PathCollision`; a snapshot containing one is not materialized on a filesystem that would merge them, and the UI says so.
- **Content:** an entry's blob is what git would store, applying **built-in** conversions only (`eol`, `text`, `core.autocrlf`, `ident`); otherwise every file under `autocrlf` would appear dirty. For paths with a custom filter driver (for example Git LFS): if the stat matches the index, the entry is the index blob; otherwise it is the raw worktree bytes, marked `unfiltered`.
- **Mode and kind:** where the filesystem cannot represent them (`core.filemode=false`, `core.symlinks=false`, typical on Windows), they come from the index for tracked files. Untracked files get mode `100644`, kind `file`.

### 6.4 Watcher backends

| Platform | Mechanism | Forces reconciliation |
|---|---|---|
| macOS | FSEvents | `MustScanSubDirs`, `UserDropped`/`KernelDropped`, `RootChanged` |
| Windows | `ReadDirectoryChangesW` | buffer overflow (zero-byte result), handle invalidation |
| Linux / WSL (ext4) | inotify | `IN_Q_OVERFLOW`, watch-limit exhaustion (falls back to polling, with a visible notice) |
| WSL, worktree on `/mnt/*` | polling only | every poll interval (degraded topology) |

### 6.5 History and base resolution

- **Audit trail:** Vet's own timeline of settled snapshots is the durable record of what happened. The reflog is supplementary provenance (which commit a snapshot came from, interdiffs across amends and rebases) and may be expired by git.
- **Base resolution:** every changeset is what it adds on top of a target. The working tree's default base is the tip of the current branch (its uncommitted changes); a branch's or a PR's is where it branched off the default branch (its target branch for a PR). Choosing a branch as the base means comparing against it as the target: the base is the merge base of that branch and the changeset's tip, as a PR shows it, never the branch's tip (which would show what the target gained since as reverse changes). A changeset whose tip the target already contains is empty, and says so. An explicit commit or reflog entry can be chosen too, and is the base as it is. The target moving on does not move the base, it only shows how far behind it is, and choosing it again catches up. Comparing two arbitrary refs is not a review and not offered. The base is a view setting while browsing and part of the session once a review is open (§5.3). For display, the backend names a base commit as the branch if it is a tip, else `branch~N` on a local branch's first-parent history (the chosen branch, the current one, the default one, the rest), else its short hash.

---

## 7. Semantic layer

### 7.1 Backends — LSP, uniformly (I6)

| Language | Backend | 3rd-party hover docs |
|---|---|---|
| Rust | rust-analyzer (pinned) | cargo registry sources |
| TypeScript | `typescript-language-server` (LSP adapter over tsserver) → TS7 native server (speaks LSP) | JSDoc from `.d.ts` |
| Svelte | svelte-language-server (→ tsserver) | as TS + component docs |
| Angular | Angular Language Server | as TS + template symbols |
| Python | Pyright (→ `ty` if/when stable) | docstrings + typeshed |
| C# | `roslyn-language-server` (pinned) | XML docs in NuGet packages |

```rust
enum Backend {
    GenericLsp(ServerConfig),
    // Quirk arms added reluctantly, per observed incompatibility,
    // narrowly scoped. No registry, no plugin surface, no trait hierarchy.
}

struct ServerConfig {
    binary, filetypes, root_markers,
    executes_project_code_during_analysis: bool,   // default true (conservative)
    // ...
}
```

Adding a language means adding a config entry plus a one-time manual check that its server supports the required operations. The orchestrator asserts `ServerCapabilities` at startup and **fails loudly** if a required operation is missing.

**Required operations:** `documentSymbol`, `definition`, `references`, `hover`, `callHierarchy` (incoming and outgoing), `workspace/symbol`, and diagnostics (push or pull).

### 7.2 Server pool and reuse (I2)

- **The live target** (the dirty tree) is served by a server running on the worktree itself. Its answers are asked fresh for every request, except answers that depend only on a document's text (its symbols): those are asked with the text of the revision's blob and reused by that blob (§7.6).
- **Any other snapshot** (a pinned base, an older timeline entry, a PR's base or head) is served by a server on a directory obtained from Acquisition via `materialize(SnapshotId) → root`. The semantic layer never sees how the directory was produced (I1). Acquisition builds it from the snapshot's commit plus the content store, and refuses if the snapshot is not materializable (§6.1).
- **Pool:** servers are keyed by `(SnapshotId or live worktree, backend, policy)`, kept warm while in use, and evicted LRU. Answers outlive their server: they are kept by query key (§7.6), not per server. Changing backend settings restarts the affected servers; their answers stop matching because the server identity in the key changed, and nothing is deleted.
- **Materialized snapshots are kept between launches** under Vet's data directory, one tree per snapshot and analysis policy shared by all languages, with a record of each language's restore. Reopening a review with the same base starts its servers without copying or restoring again. A restore record is checked against the current tools before reuse. Kept trees are garbage-collected under a disk budget (§8).
- **Restore:** a materialized snapshot needs installed dependencies for its server to work. Fetch-without-execution is the default and covers documentation needs (cargo registry, `.d.ts`, typeshed, NuGet XML docs).
### 7.3 Queries

Primitive rows pass backend answers through. Derived rows combine primitives with the diff; per I6 they are labeled derived in the UI and show their inputs.

| Question | Kind | How |
|---|---|---|
| Hover, go-to-definition, references, diagnostics | primitive | the server for the side being viewed |
| Base-side navigation, side by side with target | primitive | base server and target server queried in parallel |
| Search Everywhere (double-shift) | primitive | `workspace/symbol` on the target server |
| Which symbols changed | derived | per changed file, `documentSymbol` of both sides' texts; symbols whose ranges intersect diff hunks |
| Usages of a symbol | derived | `references` of the symbol shown, each with the symbol it is in, on the target server, async |
| Impact of a removal | derived | `references` for each removed symbol on the base server; each call site is then classified in the target as **gone**, **modified**, **likely broken** (identical and unresolved), **binding changed** (identical but resolves elsewhere), or **unknown** |
| Semantic-graph diff | derived | outgoing calls and references of each changed symbol on both sides, compared by container-qualified name. A renamed symbol appears as removed + added next to the textual diff; Vet does not infer renames or moves. |
| Is a diagnostic new or pre-existing | derived | diagnostics of each changed file on both sides. A target diagnostic is pre-existing if the base has one with equal severity, source, code and message (whitespace-collapsed) on a line that maps to it; otherwise it is new. It is *unknown*, never new, while the base side's diagnostics are unavailable; the target's diagnostics show at once as origin pending. |
| Changed callers and callees of a changed symbol | derived | incoming and outgoing calls on the target, followed through changed symbols only. Unchanged direct callers are listed as possibly affected. |

### 7.4 Reachability — a derived analysis (I6)

The reviewer's question is "is this new or changed code actually reached?" Vet answers it with a **bounded incoming-call search** from each changed or added symbol, stopping at the first configured entry point, bounded by depth and node count. Results are labeled **derived**:

- `reached` → shows the path to the entry point;
- `unreached within bound` → shows the bound and the entry-point config;
- `unknown` → backend gaps (DI, reflection, and serialization are known false-positive sources, worst in C#).

Whole-repo dead code is not a Vet analysis; it appears only when a language server itself reports it as a diagnostic (for example rustc's `dead_code` via rust-analyzer).

### 7.5 Trust

What analysis may execute project code. Not a security boundary.

- **Contributor PRs** (head on a fork or a remote not marked trusted): fetch-only restores, and only servers with `executes_project_code_during_analysis = false` start; other languages show a visible "analysis limited" marker. One action ("Trust this PR") lifts this for the PR's **current head commit**. Any new push to the PR voids the trust and restores the restrictions.
- **Analysis on trusted code** runs normally: language servers may execute project code (build scripts, proc macros, MSBuild design-time builds) without asking.

### 7.6 Reuse of answers (I2)

| Query | Depends on | Key | Kept |
|---|---|---|---|
| Document symbols (either side, any tree) | the document text, the language, the server | `(server identity, file extension, BlobId of the text sent)` | memory and disk |
| Hover, definition, references, call hierarchy, diagnostics on a materialized snapshot | the whole snapshot, its restored dependencies, the server, the policy | `(server identity, SnapshotId, policy, restore fingerprint, query)` | memory and disk |
| Workspace symbols on a materialized snapshot | as above | as above | memory |
| Any project-wide query on the live worktree | the live tree, which changes under the server | — | never reused |
| Derived analyses | their primitives and the diff | recomputed from (cached) primitives; changed symbols per file pair `(old BlobId, new BlobId, server identity)` | memory |

An answer the server gave while it reported work in progress or had not finished starting is never stored. A cached primitive keeps the identity of the server that produced it, and a derived result shows it among its inputs (I6). A new revision cancels the semantic work still running for the old one.

---

## 8. Storage and retention

- **Location:** the backend side's native filesystem (in WSL, the Linux home directory, never `/mnt/*`), in standard per-platform data directories.
- **Content store:** content-addressed files written temp → fsync → rename, referenced from metadata only once durable.
- **SQLite (WAL mode):** snapshots and their timeline order, sessions, reviewed spans, comments, and PR trust.
- **Retention:** content is kept while any open session, unresolved or unsynced comment, or reviewed span references it; closed sessions are kept 30 days (configurable). Unreferenced content is garbage-collected.
- **Semantic cache:** one SQLite file (`semantic-cache.sqlite`, WAL), apart from the review database. It holds reusable answers (§7.6) under a size budget (default 256 MB, adjustable in Settings), evicted least recently used. It is disposable: deleting it, or a format Vet does not recognise, only costs speed.
- **Materialized snapshots:** one tree per snapshot and analysis policy, with completion and restore records. A tree in use by a running Vet is locked; others are kept while an open session or a repository's remembered base refers to them, then removed least recently used to stay under a disk budget (default 20 GB, adjustable in Settings) and after 14 days unused. Settings shows the current use next to a "Clear semantic caches" action.

---

## 10. Review UX

- **Home = queue.** Changesets ordered by blocking status, then recency. Each shows completion against the active filter plus the count of changes the filter excludes (§5.2). Review state survives rebases and force-pushes.
- **Visual design:** light and dark themes from the design tokens (`ui/src/lib/theme/tokens.css`), with desktop-style controls (hover, pressed and toggled states, no text selection outside content).
- **Editor tabs** (titled with their `+N −M` line counts): a fixed "Changes" tab streams the whole changeset as hunks with a few lines of context, unchanged regions collapsed and expandable; a file has one tab (closable), whatever it shows of it, and the tab has a view mode: Unified, Split (the file's full diff) or File (the file's text with its changes drawn in, read-only). The Unified/Split/File control, Alt+Shift+1/2/3 and the View menu change the active tab's mode and never open or close a tab; each mode keeps its own caret and scroll, and the caret's line carries over. A file the changeset does not change (reached by Go to Definition, or from the tree) has a tab in File mode only. Jump to Source switches the file's tab to File mode at the line, opening it in that mode if it is not open: double-click or Enter in the tree or the Symbol list, double-click on a file header or a diff, a hunk header's button, or F4 at the caret (same line). Go to thread shows the thread in the file's tab, in the mode that tab is in, when the file has a tab open, else in the Changes tab; the file's own threads sit above the first line in File mode, as above the first hunk in a diff. Hunk navigation (Ctrl+Alt+Down/Up), marking seen (Space) and commenting work in every mode. Go to definition is Cmd+click or Cmd+B (Ctrl off macOS), on either side; a new comment is Ctrl+Alt+C (or the editor's context menu) on the selection, else on the caret's line, in every view, using the last editor the reviewer was in even after focus moved to a toolbar or the tree; a hunk's Comment button comments the whole hunk, or the selection when there is one inside that hunk. Settings open with Ctrl+Alt+S (⌘, on macOS).
- **Header:** `project  ⎇ base → target · vN`. The base chip names the base commit as in §6.5, with its hash, subject, author and age in the tooltip, and offers a one-click move to the latest when its branch has moved on; `vN` shows only while a review is open (§5.3).
- **Tool windows** on left and right rails, resizable; clicking the active tool's icon hides its window and keeps the tool selected.
- **Hunk view** with decorations: review state from spans (including `SeenAgainstOldBase`), coverage of changed lines, changed-symbol reachability (labeled derived), whitespace-only anchor notes, diagnostics as gutter markers with a count in the status footer. Marking seen is a toggle: a seen hunk can be un-marked.
- **Non-text changes** (mode, symlink target, submodule commit, binary, content unavailable) render as a change card with a hunk-style header: same "mark seen" action and key, included in hunk navigation, images with a before/after preview. A file with both kinds shows its card above the first hunk.
- **Inferred anchors** show a neutral "moved" badge with the confidence; the tooltip names the strategy and the original location. "Confirm" re-anchors at the current position (→ `Exact`). Low-confidence ones are also grouped under "Moved" in the comments tool window.
- **Review filter** is edited in the project's settings (review roots, hide binary, include/exclude globs, live excluded count) and from the file tree ("Exclude from review", "Review only this folder" change the project's filter). Excluded files collapse into one group in the file tree, which shows the excluded count.
- **Timeline:** the `LogicalTip`'s settled snapshots, with interdiffs between any two.
- **Semantic diff is a projection, never the canonical representation.** The byte-level diff is the source of truth; normalization, move detection, and formatting-churn suppression are views over it, and every normalized-away change is one click away.
- **Annotations** sync to platforms or export as agent task files. A full editor is retained for small fixes (fixup commits / suggestion blocks).
- **JetBrains-feel package:** parse the JB keymap XML; double-shift Search Everywhere; Alt+Enter intentions; Ctrl+W via tree-sitter; tool windows Alt+1..9; JetBrains Mono.

---

## 11. Platform adapters

GitHub + ADO: list PRs, fetch their heads and targets into Vet's own refs (read-only), read and post comments, poll CI per commit. Local review state (spans) is never synced. Threads are synced both ways: remote threads and replies are pulled in, local drafts are posted on request. Because anchored content is retained (§6.1), comment positions remap correctly across rebases where GitHub orphans them.

Each adapter describes its platform with a fixed set of properties. Vet core and the UI branch on these properties, never on the platform's name:

```rust
struct PlatformTraits {
    changeset_threads: Resolvable | Flat | Either, // a comment on the whole PR
    conversation_feed: bool,         // a flat feed beside threads (comments + events)
    posting: PerComment | BatchedReview,
    verdicts: Vec<Verdict>,          // the platform's own choices, in its order
    verdict_separate: bool,          // vote set independently of posting
    blocking: Vec<BlockingSource>,   // UnresolvedThreads | OpenTasks | Labels
    revision_scoped: bool,           // comments recorded on a patch set
}
```

| | GitHub | Azure DevOps | GitLab | Bitbucket | Gerrit |
|---|---|---|---|---|---|
| `changeset_threads` | Flat | Resolvable | Either | Resolvable | Resolvable |
| `conversation_feed` | yes | no | yes | no | no |
| `posting` | BatchedReview | PerComment | BatchedReview | PerComment | BatchedReview |
| `verdicts` | comment, approve, request changes | approve, approve with suggestions, wait for author, reject | approve | approve, request changes | label votes (e.g. Code-Review −2…+2) |
| `verdict_separate` | no | yes | yes | yes | no |
| `blocking` | UnresolvedThreads (if required) | UnresolvedThreads (if policy) | UnresolvedThreads (if setting) | OpenTasks | Labels, UnresolvedThreads |
| `revision_scoped` | no | no | no | no | yes |

GitHub and ADO ship in M1; the other columns show the model covers them, and are adapter work only. Platform details (especially merge checks) are verified against each platform's API when its adapter is built.

How the UI follows the traits:

- **Review and Conversation tabs** in the comments tool window, on every platform with a pull request: Review holds the threads on files and lines; Conversation holds what concerns the pull request as a whole — with `conversation_feed`, the platform's flat feed (comments and review summaries with their verdict), own comments editable there and answered by a new comment rather than a reply; without it, the changeset threads with replies and Resolve.
- **The comments tool window** shows "Submit review" with the verdict menu for `BatchedReview`, or "Post" for `PerComment`, next to the drafts it posts; a separate vote control appears beside it when `verdict_separate` is true.
- **Blocking marker** on a thread, and the queue's blocking status, mean whatever `blocking` says. A Bitbucket task is a flag on its thread.
- **Revision-scoped platforms**: Vet still anchors to its own snapshots; a remote comment's patch set maps to the snapshot built from that patch set.

`blocks_merge` feeds the queue's blocking status (§10).

---

## 12. Phasing

### 12.0 Shell spike

Historical: the spike's benchmark and mock backend were removed on 2026-10-09.

**Decided:** a Svelte web UI with CodeMirror 6 (plus `@codemirror/merge`), hosted in Tauri. The spike confirms this choice rather than comparing candidates.

**Run:** Tauri on WebView2 (Windows, WSL split) and on WKWebView (macOS), against a mock orchestrator over the real WebSocket transport.

**Corpus:** one real changeset from your own repos with at least 300 files and 3,000 hunks, including one file over 10k lines; review-state decorations on every changed line and at least 3 inline decorations per hunk; unified and side-by-side views.

**Pass criteria** (p95 on each webview):

| Metric | Threshold |
|---|---|
| Open changeset → first hunk painted | < 300 ms |
| Keystroke → paint | < 16 ms |
| Hunk-to-hunk navigation → painted | < 50 ms |
| Sustained scroll through the full changeset | ≤ 5 % dropped frames |
| Toggle `Seen` on a 1,000-line file | < 50 ms |
| Hover round-trip over the WSL-split transport | < 100 ms |
| Renderer memory, full changeset open | < 1 GB |

**Input correctness** (pass/fail): German layout on Windows and macOS. Dead keys compose correctly. AltGr characters (`{ [ ] } \ @ ~ |`) type correctly and do not trigger Ctrl+Alt bindings. IME composition doesn't corrupt decorations.

**Fallbacks:** if CM6 fails a threshold, try Monaco. If something fails only on WKWebView, rerun in Electron; if it passes there, Electron becomes the host on all platforms. **Timebox:** 5 working days.

### POC — vertical slice (dirty tree, Rust + TS + Python + C#, no hosts)

**POC platforms:** macOS native, Windows native, and the WSL split.

**Acceptance criteria:**

1. **Anchor robustness (I3):** rename, edited anchor range, duplicate matching content (→ `Ambiguous`), cross-file move (→ `Inferred`), amend, rebase, and file deletion, exercising all four `AnchorState` outcomes. Plus:
   - **split:** mark a 20-line region seen, then edit 3 lines in its middle → two seen spans around 3 unseen lines;
   - **revert:** edit a seen line, then restore it byte for byte → seen again;
   - **base change:** rebase so the base side of a seen region changes while its target side doesn't → `SeenAgainstOldBase`;
   - **deletion:** a seen pure-deletion region survives an unrelated rebase;
   - **line endings:** a CRLF-only change → `Exact { whitespace_changed: true }` for comments;
   - **filter change:** changing the review filter leaves all review state intact, and completion shows the excluded count;
   - **repeated lines:** a seen `return 0` that appears twice, with its surrounding context rewritten → unseen;
   - **mode only:** a changeset whose only change is `100644 → 100755` is not complete until that change is acknowledged;
   - **Windows filesystem:** with `core.filemode=false` / `core.symlinks=false`, mode and kind come from the index (§6.3); paths differing only in case are flagged `PathCollision` and not materialized.
2. **Semantics:** after an agent edit settles, hover, references and diagnostics reflect it. Base-side go-to-definition works side by side with target for a dirty-tree session. Impact and reachability are shown for changed symbols; a removed function's remaining call sites are found and classified. An older timeline snapshot from an open session can be materialized and queried.
4. **Reconciliation:** simulate dropped watcher events (kill, inotify overflow, FSEvents `MustScanSubDirs`, `ReadDirectoryChangesW` buffer overflow) → the rescan produces the correct snapshot, and review state is never silently wrong. A Windows-side write into the WSL worktree is observed or reconciled.
5. **Shell and topology:** the POC meets the spike's thresholds with real orchestrator data on every POC platform; the WSL split works end to end over the authenticated transport.

Also included: fetch-only restore with shared caches.

### M1 — daily driver

Grouped by area; areas and the items within each are in priority order.

#### Reviewing changes

- Sticky headers in every diff view: while scrolling, the current file's header and the current hunk's bar stay pinned at the top until the next one pushes them out
- The file tree marks the file the viewer is in (the one pinned at the top of the Changes tab, or the file tab's), and the Changes tab reopens the review at the scroll position it was left at
- The base chosen while browsing is kept per repository: reopening the repository without an open review shows the tree against it again (§5.3)
- Double-clicking a line in the Changes tab opens the file's tab at that line, highlighted
- The file icon of a file in the Changes tab opens the file's tab, like the hunk bar's; a changed file's tab switches between file and diff view
- A changed file's file view marks its added, changed and removed lines (removed ones where they were), coloured by review state; a mark opens the hunk's old lines with the hunk bar's actions (toggle seen, comment, open in the diff), so a change can be marked seen in context. Next/previous hunk and Space work there as in the diff (design to be iterated)
- Hunks can be flagged for later review (§5.2): a flag button beside the seen toggle in the hunk bars and in the File view's hunk toolbar (active while flagged), Toggle Flag in the Review menu, Next and Previous Flagged Hunk in the Navigate menu (across files, in All Changes and in file tabs), and Finish Review… warns with the number of flagged hunks left and offers to go to the first one; finishing anyway is allowed
- Edit and remove your own comments; removing the first comment removes its thread
- Analysis results (diagnostics and other hints) are not shown in collapsed unchanged regions
- Line highlight in the editor: the clicked line stays highlighted
- Long lines are reachable in every view (the Changes tab, diff and file tabs, both panes of a split view): horizontal scrolling when soft wrap is off, with hunk bars, cards and thread cards staying in view; soft wrap for prose (Markdown, reStructuredText, plain text), with a setting for it in the redesigned settings; a changed line that runs past the visible width is marked whether or not it wraps

#### Changesets and history

- GitHub + ADO adapters (§11): pull requests as changesets, their threads synced both ways, CI per commit
  - Login reuses a signed-in CLI (`gh auth token`; `az account get-access-token` for Azure DevOps), else a personal access token (also for GitHub Enterprise hosts); no OAuth or device-code login. Tokens live in the OS keychain, never in Vet's store; in the WSL split the shell keeps them and hands them to the backend
  - The pull-request list is per project (the repository's remote): "Waiting for me" by default (review requested from the viewer or their team, not reviewed since the last push), then Mine, Assigned, All open, with filters for author, label, draft and CI state. Each row shows title, author, branch → target, CI state, unresolved threads, the viewer's review state ("new pushes since your review") and when it was updated
  - Thread sync: the viewer's status change wins over the platform's if made since the last sync; an edit to a posted comment is pushed at once, and if the comment also changed on the platform since the last sync both texts are shown for the viewer to choose; a thread deleted on the platform is removed from Vet (a local draft reply in it is dropped with a notice); drafts are posted on Submit as one review (GitHub) or one by one (ADO), never as a pending review on the platform; a comment the platform refused is marked failed with the reason and is posted again with the next Submit, or discarded on its own (a comment being posted cannot be discarded)
- Committed changesets (branches and pull requests), shown as all changes at once (the squash, the default and what completion counts) or by commit. By commit, a clean merge of the target branch is a collapsed marker with nothing to review, and a merge with conflict resolutions shows only the resolution (as `git show --remerge-diff`)
- One changeset on view per project; switching to another changeset and back restores its open tabs, scroll positions and current file
- Interdiffs; snapshot timeline, as a bottom panel (not a sidebar tool window). A changeset has versions (a local session's snapshots, a pull request's pushes); an interdiff compares any two, chosen in the timeline, and shows as a mode of the review view with a way back. "Changes since my last review" is the default action on a changeset and carries the seen marks over. When the base moved (a rebase), the interdiff leaves out what the base move explains, as `git range-diff` does per commit
- Git fetch (in-process, read-only): on demand and periodically, so remote branches and how far a base is behind are current; Vet never pulls, merges or rebases
- Changeset queue home, reached through a Queue / Review toggle in the app header (not a sidebar tool window): every open pull request of the project's repository and local branches, with more metadata and filtering than the title bar's changeset switcher, of which it is the full-page view

#### Semantics and navigation

- Diagnostics are marked new or pre-existing by comparing the target's with the base's; new ones, those the change introduced, are counted separately and shown first
- Problems panel: the diagnostics of the whole changeset in one list, new ones first
- For a changed symbol: its changed callers (recursively, through changed symbols only) and changed callees, and its unchanged direct callers flagged as possibly affected; the list says what the server's call hierarchy cannot see (dynamic dispatch, reflection)
- Changed symbols are listed in the Changes tree under their files, which are collapsed by default and show their symbol count; one button in the tree's header expands or collapses all (a file that appears later follows the last choice). Only named containers nest: an unchanged container that holds a changed symbol is shown muted, without a change letter, and clicking it only opens or closes it. A removed symbol (base side only) is struck through in place under its container. The change letter M, A or D stands in the status column of the symbol's row. Symbols carry no review state: seen marks stay on hunks and files
- Each file shows a spinner in the tree while its symbols are being found, and the tree's header a count of the files done and a progress bar; symbols appear under each file as its analysis ends. The Symbol panel has no loading state of its own
- The Symbol panel shows one symbol, changed or not: the one under the cursor of an editor, or the one last clicked in the tree, whichever came last. With the cursor in no function, type or module it is empty. Its sections open and close, and are remembered: name, container and file; signature; changed callers; changed callees; possibly affected; reached from; usages. For a removed symbol it shows the remaining references (base side) instead
- Moving the cursor into a changed symbol in any editor (Changes, a diff or file tab) selects it in the tree: its file opens, its row scrolls into view, and no focus moves
- Double click, Enter or Jump to Source (F4) on a changed symbol in the tree opens its definition (the base side's for a removed symbol); ← and → open, close and move through the rows; speed search in the tree also matches symbol names; a click on a changed caller or callee in the Symbol panel selects it in the tree
- Find usages: a list of a symbol's references, any symbol, not only changed ones, in the bottom panel
- Go to definition shows its target: the symbol under the pointer is underlined while Cmd/Ctrl is held
- Hover popups styled as part of the app: the design tokens' sheet surface, border, radius and shadow, a readable maximum width and height with scrolling, consistent spacing between signature, docs and diagnostics, in both themes
- Syntax highlighting in hover popups (the server's Markdown rendered, code blocks highlighted)
- File structure view: the symbols of the current file as a tree

#### Language servers and performance

- Language-server status per server (starting, indexing with the server's progress, ready, failed, not installed), in the footer with details on click
- Language-server answers are reused across restarts under keys naming everything they depend on (§7.6): symbols by content on any tree, and hover, definition, references, call hierarchy and diagnostics on materialized snapshots
- Materialized base snapshots and their dependency restore are kept between launches, keyed by snapshot, so reopening a review with the same base starts its servers without copying or restoring again
- Semantic results track what they depend on (file contents, the project, server version), so a new revision recomputes only what its changes affect instead of discarding everything
- Svelte/Angular backends (config entries + capability assert); C# is in the POC
- Requests whose answer is no longer wanted (a hover the pointer left, a query for an outdated revision) are cancelled from the UI down to the language server (`$/cancelRequest`)

#### Search and keyboard

- Search in the open file (find with next/previous, matches marked) and search in the project (text across the repository's files, results grouped by file, opening a result at its line)
- Keyboard shortcuts for every action, shown in menus and tooltips, with a Keymap page in Settings (beyond the JetBrains-feel package's keymap import)
- Speed search: typing in any menu, dropdown, list or tree filters it (camel-hump matching, matches highlighted, Escape clears; trees keep the folders on the path to a match), as in IntelliJ
- JetBrains-feel package

#### App shell

- Background work is visible, as in JetBrains IDEs: a progress indicator in the footer while a project opens and while work runs (scanning, diffing, servers starting and indexing, restores), naming the step, with the details on click; the review shows as early as it can instead of behind a loading screen
- Project switcher and changeset switcher in the title bar, redesigned (design to follow): the project switcher lists recent projects plus "Open…"; the changeset switcher picks what is under review
- File-type icons for files and folders in the file tree and on tabs, by extension and well-known names (a folder's icon open or closed)
- The main menu is the map of what Vet does and where its shortcuts are learned: File, View, Navigate, Changeset, Review and Help, with an app menu, Edit and Window around them in the macOS menu bar (the host's native one); on Windows and Linux the title bar's ☰ draws it, also on the welcome screen, and Alt pressed and released alone opens it, then a first letter opens a menu. An item that does not apply is disabled, with the reason as its tooltip in the drawn menu, never hidden. Tool windows are Alt+1…6, Close Tab is Ctrl/⌘+W, Previous/Next Tab are Alt+←/→ (⇧⌘[ and ⇧⌘] on macOS). The View menu's view mode is one radio group, Unified / Side-by-Side / File, on Alt+Shift+1/2/3 (matched by the physical key, so every layout works); Post Comments… (Submit Review… where the platform submits a review) is Ctrl/⌘+Shift+K; Ctrl+Alt+V opens a picker of the platform's votes at the toolbar's outgoing button, where the platform keeps the vote apart from the comments; Ctrl+Alt+L switches between Changes Since My Last Review and the Whole Changeset; Alt+F2 reveals the file in the tree. Alt with Shift held never opens the menu bar (on Windows Alt+Shift switches the keyboard layout)
- Native context menus (the OS's, through the host's menu API) everywhere the app has one: the file tree, the editors and diff views, tabs; they replace the webview's default menu and show each item's shortcut. Without a host (plain browser) the app draws the same items
- The split between the right sidebar's panels can be dragged, keeping its position
- Resizable dialogs (Settings first), keeping their size

### M2

- Stacked changesets: branch stacks (each branch building on the one before) reviewed as a stack
- Themes and colour schemes as two settings, as in IntelliJ: an app theme (light, dark, Darcula, following the system) for the chrome, and an editor colour scheme (Default, Darcula, Solarized light and dark, high contrast) for syntax, diff and review-state colours. Syntax highlighting grows to the categories a scheme needs (comment, constant, variable, parameter, property, operator, punctuation, attribute, namespace, tag, …). Custom schemes by importing IntelliJ colour schemes and VS Code themes into an editable scheme file; no in-app colour editor
- Several projects open at once, each in its own window, with the welcome screen as a window of its own (until then one window switches between the welcome screen and a project, keeping a position and size for each)
- Contributor-PR trust (§7.5): limited analysis until trusted; a new push voids trust
- Semantic-graph diff
- Review-questions assistant (read-only; answers cite hunks)
- Worktree groups
- Export annotations as agent task files
- Coding agents integrated into the UI through the Agent Client Protocol (ACP)
- Minimap
- The language-server client's JSON-RPC transport uses `async-lsp` instead of its own framing, once upstream supports cancelling outgoing requests and our `lsp-types` version
- CI annotations on lines: the platform's check annotations (path, lines, severity, message) shown in the hunk view like diagnostics, anchored to the commit they were produced for
- Narrowing the project's review filter for one review from the file tree ("Review only this folder", "Exclude from review"), shown in the tree with a reset

### Later

Spec/test-diff-centric views; more languages via config; `ty` adoption when stable.

---

## 13. Performance (falsifiable)

Targets apply to supported topologies; the `/mnt/*` WSL topology is exempt.

| Milestone | Target |
|---|---|
| Open changeset → UI interactive | < 300 ms |
| Diff available | < 1 s |
| Live target server: hover/definition | backend-bound; Vet adds < 20 ms |
| Materialize a snapshot (worktree + overlay) | < 2 s for a 10k-file repo; server start is async with visible progress |
| Impact queries (`references` + incoming calls), 50-symbol changeset | < 10 s p50, async; slow symbols shown as pending |
| Reconciliation establishes correct snapshot identity | < 1 s, excluding time to hash changed bytes |
| Hashing changed bytes (read + hash, files in OS cache) | ≥ 1 GB/s |
| A review action (mark, comment) or a manifest, whatever the semantic load | < 100 ms; the load test holds every semantic resource stuck and requires them all to answer |

**Benchmark corpus:** your own repos, your own hardware, on each POC topology.

---

## 14. Open questions

1. **Reachability tuning:** per-language entry-point configs and search bounds are ongoing craft.
2. **Comment sync conflicts:** the M1 policy (Changesets and history); confirm against real PR flow.
3. **Base-resolution UX:** current-branch tip vs. merge-base defaults, tuned against the actual harness mix.
4. **Risk ordering:** whether the queue should rank changesets by a risk model, and what signals it would use.
5. **Base-server cost:** if materializing and starting a second server per session proves too slow for some backend (likely Roslyn on large solutions), consider loading base-side files as overlays into the target server for that backend only.
