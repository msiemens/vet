# Vet — Core Spec

## 1. Product definition

Vet is a **review-first IDE** for the agentic coding age. The unit of work is the *changeset*, not the file. The human's role is *verifier*, not author. Agents run elsewhere; Vet is the checkpoint between agent output and merged code.

**Primary activities, in priority order:**

1. Review changesets (dirty trees, local commits, stacks, PRs)
2. Verify semantics (navigation, impact, reachability, base-vs-target queries)
3. Verify behavior (run/debug the target, one keystroke)
4. Close the loop (annotations → platform comments or agent task files)

**Permanent non-goals:**

- Agent authoring/orchestration integration
- Harness-specific integrations or provenance inference (observation-only)
- Being a general-purpose authoring IDE
- Multi-user/team collaboration (personal tool)

**Threat model:** agents are non-adversarial but fallible. Vet guards against *accidents* (running code you haven't looked at), not against attacks. The one exception is contributor PRs, which Vet neither runs nor analyzes with code-executing backends until you choose to trust them (§7.5).

**Platforms:** macOS, Windows, and WSL2, including the split where the shell runs on Windows and the backend runs in WSL (§4.1).

---

## 2. Core invariants — frozen

| # | Invariant | Test |
|---|---|---|
| **I1** | **The semantic layer never knows git.** Acquisition maps git state into immutable snapshots; provenance lives in the acquisition/review layer. | No semantic-layer API accepts commit hashes, ref names, or origin flags. |
| **I2** | **Vet never invalidates semantic data.** Semantic answers come from language servers running against the live worktree or a complete, materialized snapshot. Nothing semantic is persisted: answers from a server on an immutable snapshot are cached in memory only for that server's lifetime, and answers from the live worktree are not cached at all. | No code path in Vet decides whether a semantic fact is still valid. Restarting a server or Vet discards every semantic answer. |
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
ReviewFilter      review roots + include/exclude patterns. A VIEW over a changeset;
                  never part of snapshot identity.
Pin               recorded {head, tree_hash} at review open; dirty-at-open content
                  copied into the content store
Anchor            content anchor for one range on one side (I3); used by threads
Thread            a comment thread on a line range, a file, or the whole changeset;
                  optionally linked to a platform thread
ReviewedSpan      the durable unit of review coverage: a range of content the user
                  marked seen, on one or both sides. Hunks are a display concept;
                  "hunk seen" is derived from spans. Flags and concerns are comments.
Session           an open review of one Changeset; owns its pin and review state
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
│  │ Execution: run/debug, approval, worktrees  │ │
│  ├────────────────────────────────────────────┤ │
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

enum Strategy { ContextWindow, CrossFileMove }

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
- **Inferred strategies, evidence-based:** collect candidates from both a context-window search within the file and a cross-file content match across the changeset's changed files (catches moves). One dominant candidate → `Inferred` with strategy and confidence. Competing plausible candidates → `Ambiguous`. None → `Stale`.
- Inferred candidates are scored together, never first-success-wins.

### 5.2 Review state

```rust
struct ReviewedSpan {
    base:   Option<Anchor>,   // reviewed base-side lines; None for pure additions
    target: Option<Anchor>,   // reviewed target-side lines; None for pure deletions
    state:  Seen | SeenAgainstOldBase,
}
```

Marking a hunk (or a selection) seen records one `ReviewedSpan` covering its lines on both sides. Spans record coverage only; flagging a concern is a comment (§5.1). A hunk shows as seen when every changed line in it is covered by a `Seen` span.

**Remapping is a partition (constitutional, per I3).** When the changeset changes, each span is remapped side by side using the line-level diff between its recorded file and the current file (following git-detected renames). Only Exact strategies apply. Inferred moves never carry review state. Each reviewed line whose content is identical and maps into the current changeset stays covered. Every other line is uncovered, so one span can become several spans, or none. If a reviewed line's mapping is not unique (for example, repeated identical lines whose surrounding context changed), it is uncovered rather than left to the diff algorithm's tie-breaking. A seen 20-line region that gets a 3-line edit becomes up to two seen spans around 3 unseen lines.

**Base changes.** If a span's target lines are still identical but any of its base lines changed, its descendants are marked `SeenAgainstOldBase`. They are rendered distinctly, count as **unseen** toward completion, and can be re-confirmed in one action.

This catches base changes **within the reviewed region only**. Semantic changes elsewhere surface through impact queries (§7.3), not review state. Losing review progress is acceptable; manufacturing false "already reviewed" confidence is not.

**Non-text changes.** A file-level change with no reviewable text (mode, symlink target, submodule commit, binary or oversized content, a pure rename, an added or deleted empty file) is one review item, acknowledged like a hunk and stored the same way, keyed by the before and after entries.

**Completion** is relative to the active `ReviewFilter`: a changeset is complete when every changed line that passes the filter is covered by a `Seen` span and every non-text change that passes it is acknowledged. The UI always shows how many changes the filter excludes, so "complete" never implies every change was seen.

---

## 6. Acquisition layer

### 6.1 Snapshots and scope

- **Snapshot contents:** a file is in a snapshot iff it is under the repository root **and** (tracked by git **or** untracked and not git-ignored). Files over the capture size cap (default 50 MB) are still hashed in full, so snapshot identity stays exact, but their content is not retained and they appear as `content unavailable`. Ignored paths can be opted in explicitly. The untracked contents of folders a tool owns by name (`.pnpm-store/`) are never part of a snapshot; this list is explicit and fixed. No relevance inference of any kind (I4). Executable-bit and symlink changes are first-class `(mode, kind)` entries. Submodules are recorded as pinned commits; descent is opt-in.
- **Review filter:** review roots, binary hiding, and user patterns are a `ReviewFilter` applied when *displaying* a changeset. Changing it never changes snapshot identity, so it never disturbs anchors or review state.
- **Pin on review open:** record `{head, tree_hash}` and copy dirty and untracked content into Vet's content store. Dirty content is **never** written into the git object database, so untracked secrets never become pushable objects. The initial changeset is `HEAD → dirty-at-open`; the live changeset is `dirty-at-open → now`.
- **Content retention:** when a snapshot settles while a session is open, every blob of it that git cannot reconstruct (dirty and untracked content) is copied into the content store, so every snapshot on the session's timeline stays reconstructible. In addition, every blob that appears on either side of a displayed diff, or under an anchor or span, is copied when first displayed, so review state and anchors never depend on git keeping old objects alive.
- **Session refs:** while a session is open, it holds `refs/vet/sessions/<session-id>/{base,target}` on the commits its snapshots are built from, so they stay materializable. The refs are deleted when the session closes.
- **Availability:** a snapshot is *materializable* only while every one of its blobs is available (from git or the content store). After that, semantic queries on it are disabled with a visible reason. Vet never starts a server on a partial tree.

### 6.2 Watcher, settling, and reconciliation

- **Watcher = latency optimization, never authority.** Filesystem events drive incremental re-hashing.
- **Settling:** after a write burst, a snapshot is *settled* once the touched files' size and mtime stay stable across a debounce window (about 1–2 s). Each settled snapshot is appended to its `LogicalTip`; this sequence is the edit timeline. Same-file concurrent edits within one window are unresolvable by design.
- **Authoritative reconciliation** runs at review open, on watcher overflow or rescan signals (§6.4), on resume or restart, before declaring a snapshot settled, and before every launch (§9). It must *establish correct snapshot identity*, metadata-first (filesystem stat, git index, cached hashes), hashing only uncertain entries. Dropped or coalesced events can never corrupt review state, only delay it.

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
- **Base resolution:** session pin (default), merge-base, or an explicit commit or reflog entry. This is pure UI over stored pins.

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

### 7.2 Server pool (I2)

- **The live target** (the dirty tree) is served by a server running on the worktree itself. Its answers are never cached.
- **Any other snapshot** (a pinned base, an older timeline entry, a PR's base or head) is served by a server on a directory obtained from Acquisition via `materialize(SnapshotId) → root`. The semantic layer never sees how the directory was produced (I1). Acquisition builds it from the snapshot's commit plus the content store, and refuses if the snapshot is not materializable (§6.1).
- **Pool:** servers are keyed by `(SnapshotId or live worktree, backend)`, kept warm while in use, and evicted LRU. A server on an immutable snapshot caches its answers in memory; the cache dies with the server. Changing backend settings restarts the affected servers. Materialized directories are deleted with their server.
- **Restore:** a materialized snapshot needs installed dependencies for its server to work. Fetch-without-execution is the default and covers documentation needs (cargo registry, `.d.ts`, typeshed, NuGet XML docs). Executing restores count as runs and go through the run check (§7.5).

### 7.3 Queries

Primitive rows pass backend answers through. Derived rows combine primitives with the diff; per I6 they are labeled derived in the UI and show their inputs.

| Question | Kind | How |
|---|---|---|
| Hover, go-to-definition, references, diagnostics | primitive | the server for the side being viewed |
| Base-side navigation, side by side with target | primitive | base server and target server queried in parallel |
| Search Everywhere (double-shift) | primitive | `workspace/symbol` on the target server |
| Which symbols changed | derived | `documentSymbol` on both sides of each changed file; symbols whose ranges intersect diff hunks |
| Impact of a change | derived | `references` + incoming `callHierarchy` for each changed or added symbol, on the target server, async |
| Impact of a removal | derived | `references` for each removed symbol on the base server; each call site is then classified in the target as **gone**, **modified**, **likely broken** (identical and unresolved), **binding changed** (identical but resolves elsewhere), or **unknown** |
| Semantic-graph diff | derived | outgoing calls and references of each changed symbol on both sides, compared by container-qualified name. A renamed symbol appears as removed + added next to the textual diff; Vet does not infer renames or moves. |

### 7.4 Reachability — a derived analysis (I6)

The reviewer's question is "is this new or changed code actually reached?" Vet answers it with a **bounded incoming-call search** from each changed or added symbol, stopping at the first configured entry point, bounded by depth and node count. Results are labeled **derived**:

- `reached` → shows the path to the entry point;
- `unreached within bound` → shows the bound and the entry-point config;
- `unknown` → backend gaps (DI, reflection, and serialization are known false-positive sources, worst in C#).

Whole-repo dead code is not a Vet analysis; it appears only when a language server itself reports it as a diagnostic (for example rustc's `dead_code` via rust-analyzer).

### 7.5 Run check

A guard against accidentally running build changes you haven't looked at, not a security boundary.

- **Rule:** before a run, debug, test, or executing restore, Vet checks whether any executor-relevant file changed since the last run you approved in this repo (a hash over those files' snapshot entries). If so, it shows those changes and asks "run anyway?" Approving records the new hash.
- **Executor-relevant files** (a fixed list in the POC): Rust `build.rs`, `.cargo/config.toml`, `Cargo.toml`, `Cargo.lock`, `rust-toolchain(.toml)`; TS/JS `package.json`, `.npmrc`, lockfiles, tool configs that are code (`*.config.{js,ts,…}`); Python `setup.py`, `pyproject.toml`, `conftest.py`, `noxfile.py`, `tox.ini`; C# `.csproj`/`.targets`/`.props`, `Directory.Build.*`; CI configs; build-tool files (`Makefile`, `justfile`, `Dockerfile`); toolchain and environment files (`.envrc`, `shell.nix`, `flake.nix`, `.tool-versions`).
- **Contributor PRs** (head on a fork or a remote not marked trusted): no run button, fetch-only restores, and only servers with `executes_project_code_during_analysis = false` start; other languages show a visible "analysis limited" marker. One action ("Trust this PR") lifts this for the PR's **current head commit**. Any new push to the PR voids the trust and restores the restrictions.
- **Analysis on trusted code** runs normally: language servers may execute project code (build scripts, proc macros, MSBuild design-time builds) without asking.

---

## 8. Storage and retention

- **Location:** the backend side's native filesystem (in WSL, the Linux home directory, never `/mnt/*`), in standard per-platform data directories.
- **Content store:** content-addressed files written temp → fsync → rename, referenced from metadata only once durable.
- **SQLite (WAL mode):** snapshots and their timeline order, sessions, reviewed spans, comments, run approvals, and PR trust.
- **Retention:** content is kept while any open session, unresolved or unsynced comment, or reviewed span references it; closed sessions are kept 30 days (configurable). Unreferenced content is garbage-collected.

---

## 9. Execution

One keystroke from review to run/debug.

- A dirty tree runs in place.
- A committed changeset or PR runs in a throwaway worktree (the materialized snapshot from §7.2, when one exists).

**Launch:** reconcile (§6.2), apply the run check (§7.5), start the process. If the snapshot at launch differs from the last reviewed target, the run panel says "running snapshot X — N files changed since review."

---

## 10. Review UX

- **Home = queue.** Changesets ordered by blocking status, then recency, with executor-relevant changes flagged. Each shows completion against the active filter plus the count of changes the filter excludes (§5.2). Review state survives rebases and force-pushes.
- **Visual design** follows `Vet UI Proposal.html` (light and dark themes), with desktop-style controls (hover, pressed and toggled states, no text selection outside content).
- **Editor tabs** (titled with their `+N −M` line counts): a fixed "Changes" tab streams the whole changeset as hunks with a few lines of context, unchanged regions collapsed and expandable; file tabs (closable) show one file's full diff. A file opens in a tab from the tree (double-click or Enter), from its name in the Changes stream, from a hunk header, or with F4 at the caret (same line). Hunk navigation (Ctrl+Alt+Down/Up) and marking seen (Space) work in both. Settings open with Ctrl+Alt+S (⌘, on macOS).
- **Tool windows** on left and right rails, resizable; clicking the active tool's icon hides its window and keeps the tool selected.
- **Hunk view** with decorations: review state from spans (including `SeenAgainstOldBase`), coverage of changed lines, changed-symbol reachability (labeled derived), executor-relevant changes highlighted, whitespace-only anchor notes, diagnostics as gutter markers with a count in the status footer. Marking seen is a toggle: a seen hunk can be un-marked.
- **Non-text changes** (mode, symlink target, submodule commit, binary, content unavailable) render as a change card with a hunk-style header: same "mark seen" action and key, included in hunk navigation, images with a before/after preview. A file with both kinds shows its card above the first hunk.
- **Inferred anchors** show a neutral "moved" badge with the confidence; the tooltip names the strategy and the original location. "Confirm" re-anchors at the current position (→ `Exact`). Low-confidence ones are also grouped under "Moved" in the comments tool window.
- **Review filter** is edited in a popover (review roots, hide binary, include/exclude globs, live excluded count) and from the file tree ("Exclude from review", "Review only this folder"). It is stored per session; excluded files collapse into one group.
- **Run targets:** the Run control is a split button. Targets are discovered by reading manifests (Cargo bins, `package.json` scripts, `pyproject` scripts), never by executing tools, and can be edited (command, working directory, env); stored per repo.
- **Timeline:** the `LogicalTip`'s settled snapshots, with interdiffs between any two.
- **Semantic diff is a projection, never the canonical representation.** The byte-level diff is the source of truth; normalization, move detection, and formatting-churn suppression are views over it, and every normalized-away change is one click away.
- **Annotations** sync to platforms or export as agent task files. A full editor is retained for small fixes (fixup commits / suggestion blocks).
- **JetBrains-feel package:** parse the JB keymap XML; double-shift Search Everywhere; Alt+Enter intentions; Ctrl+W via tree-sitter; tool windows Alt+1..9; JetBrains Mono.

---

## 11. Platform adapters

GitHub + ADO: list PRs, pull branches, read and post comments, poll CI per commit. Local review state (spans) is never synced. Threads are synced both ways: remote threads and replies are pulled in, local drafts are posted on request. Because anchored content is retained (§6.1), comment positions remap correctly across rebases where GitHub orphans them.

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

GitHub and ADO ship in M2; the other columns show the model covers them, and are adapter work only. Platform details (especially merge checks) are verified against each platform's API when its adapter is built.

How the UI follows the traits:

- **Conversation tab** in the comments tool window exists only when `conversation_feed` is true.
- **Toolbar** shows "Submit review" with the verdict menu for `BatchedReview`, or "Post" for `PerComment`; a separate vote control appears when `verdict_separate` is true.
- **Blocking marker** on a thread, and the queue's blocking status, mean whatever `blocking` says. A Bitbucket task is a flag on its thread.
- **Revision-scoped platforms**: Vet still anchors to its own snapshots; a remote comment's patch set maps to the snapshot built from that patch set.

`blocks_merge` feeds the queue's blocking status (§10).

---

## 12. Phasing

### 12.0 Shell spike

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

### POC — vertical slice (dirty tree, Rust + TS + Python, no hosts)

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
3. **Run check:** an agent edit to `package.json` scripts → prompt before running; no build-file changes → runs without asking.
4. **Reconciliation:** simulate dropped watcher events (kill, inotify overflow, FSEvents `MustScanSubDirs`, `ReadDirectoryChangesW` buffer overflow) → the rescan produces the correct snapshot, and review state is never silently wrong. A Windows-side write into the WSL worktree is observed or reconciled.
5. **Shell and topology:** the POC meets the spike's thresholds with real orchestrator data on every POC platform; the WSL split works end to end over the authenticated transport.

Also included: run-target-in-place; fetch-only restore with shared caches.

### M1 — daily driver

- Committed and stacked changesets (per-commit / pseudo-commit / squash lenses)
- Changeset queue home
- Change decomposition (mechanical vs. logic clusters)
- Interdiffs; snapshot timeline
- C#/Svelte/Angular backends (config entries + capability assert)
- Throwaway-worktree execution
- JetBrains-feel package
- Diagnostics on the changeset

### M2

- GitHub + ADO adapters
- Contributor-PR trust (§7.5): no run and limited analysis until trusted; a new push voids trust
- Semantic-graph diff
- Review-questions assistant (read-only; answers cite hunks)
- Worktree groups
- Export annotations as agent task files

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
| Launch (reconcile + run check) | < 1 s, excluding time to hash changed bytes |

**Benchmark corpus:** your own repos, your own hardware, on each POC topology.

---

## 14. Open questions

1. **Reachability tuning:** per-language entry-point configs and search bounds are ongoing craft.
2. **Comment sync conflicts:** local-wins policy; confirm against real PR flow.
3. **Base-resolution UX:** pin vs. merge-base defaults, tuned against the actual harness mix.
4. **Risk ordering:** whether the queue should rank changesets by a risk model, and what signals it would use.
5. **Base-server cost:** if materializing and starting a second server per session proves too slow for some backend (likely Roslyn on large solutions), consider loading base-side files as overlays into the target server for that backend only.
