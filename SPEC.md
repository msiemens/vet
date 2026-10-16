# Vet — Spec

This file describes what Vet does and the rules it must always follow. How Vet is built is in `ARCHITECTURE.md` (backend) and `ui/ARCHITECTURE.md` (UI).

In this file, the **current changeset** is the one changeset of a project that Vet has open at a time. The **review screen** is the screen that shows its changes; the **queue** is the screen that lists changesets.

## 1. Product

Vet is a desktop app for reviewing code changes, in particular changes made by coding agents. It shows changesets rather than files, and the reviewer reads and marks code rather than writing it. Agents run outside Vet.

What it does, in order of priority:

1. Review changesets: uncommitted changes, local commits, branches, pull requests.
2. Answer questions about the code: navigation, impact, reachability, the old version against the new one.
3. Comment, with comments synced to GitHub or Azure DevOps.

**Non-goals:**

- Writing code or running agents.
- Integrations with specific agent tools, or guessing which tool made a change.
- Being a general-purpose IDE.
- Collaboration between several users. Vet is a personal tool.

**Threat model:** agents are assumed to make mistakes, not to attack. Vet does not defend against malicious code, with one exception: code from outside your own project. That is pull requests from a fork and, in a project you have not marked trusted, all pull requests. Vet does not run anything that executes such code until you trust it (§6.5).

**Platforms.** The backend, the language servers, Vet's storage and the repository under review always run on the same machine, the _backend side_. Only the window (the shell) may run elsewhere.

| Setup     | Window           | Backend side                             | Status    |
| --------- | ---------------- | ---------------------------------------- | --------- |
| macOS     | Tauri            | macOS                                    | supported |
| Windows   | Tauri            | Windows                                  | supported |
| WSL split | Tauri on Windows | WSL2, repository on the Linux filesystem | supported |

## 2. Invariants

### I1 Cache keys cover every input

The key of a cached language-server answer holds:

- for an answer about one file: the blob id of that file's text;
- for an answer that looks across files: the snapshot, the analysis policy (§6.5) and the dependency restore;
- always: the server's fingerprint (binary, version, settings, toolchain, and any user configuration outside the repository that it reads).

Further rules:

- No time-based or event-based invalidation: a cached answer is valid exactly while its key matches.
- For the working tree, which changes under the server, only per-file answers are cached.
- Caches are disposable: deleting one only makes Vet slower.
- Vet computes the server's fingerprint and the restore's state once, and again after a launch, a settings change, a restore, a server restart or clearing the caches.

Tests:

- For each kind of cached answer, changing any one input is a cache miss, and changing none is a hit.
- The same scenario gives the same results with all caches off and with caching on.
- Partial answers (§6.3) are never stored.
- Deleting the cache file and the materialized snapshots (§6.2) changes no review state and no result.

### I2 Review state is anchored to content, not line numbers

When the code changes, comments and review marks are remapped, and each ends up `Exact`, `Inferred`, `Stale` or `Ambiguous`. "Seen" stays only on content that is identical to what was reviewed: new or changed content is never marked seen without the reviewer doing it.

- Test: comments survive amend, rebase, force-push, renames, moves and agent edits. They are remapped with the confidence recorded, and never silently lost.
- Test: when new edits split a seen region, the identical parts stay seen and the rest becomes unseen.

### I3 Facts about code come only from language servers

Types, references, diagnostics and calls come only from language servers; Vet does not guess them. Vet may compute further results from them (_derived_ results), but labels them as derived and shows what they were computed from.

- Test: every fact can be traced to a language server, and every derived result shows its inputs.
- Test: an operation a server does not support is shown as "not supported by this language's server", never as an empty result. A server that cannot receive file contents is refused with a visible error.

## 3. Concepts

- **Snapshot:** the contents of a working tree at one moment: every file with its content, mode and kind (file, symlink or submodule). It never changes.
- **Edit timeline:** the settled snapshots of something that changes over time (the working tree, a branch, a pull request), in order.
- **Changeset:** a base snapshot and a target snapshot.
- **Interdiff:** a diff of two diffs: how the changes of one version of a changeset differ from the changes of another. Since it compares changes rather than files, a rebase alone gives an empty interdiff.
- **Review filter:** which files to review (review roots, include and exclude patterns), set per repository. It is applied when a changeset is shown and is never part of a snapshot.
- **Pin:** the commit and tree recorded when a review starts (version 1), with the uncommitted content of that snapshot copied into the content store.
- **Anchor:** where a comment or mark is attached: one range of content on one side (I2).
- **Thread:** a comment thread on a line range, a file or the whole changeset, optionally linked to a thread on the platform.
- **Reviewed span:** a range of content the reviewer marked seen or flagged, on one or both sides. Vet stores spans; whether a hunk is seen is worked out from them. A concern is a comment, not a mark.
- **Session:** a working tree against a base: browsing until a review starts (§4.5), then an open review of one changeset with its pin, versions and marks.

**Provenance:** a snapshot holds file contents only. Its origin, its base commit, branch and pull request details, and its place in the timeline are stored separately, by snapshot id.

## 4. Changesets and versions

### 4.1 Snapshots

- **What is in a snapshot:** every file under the repository root that git tracks, plus every untracked file that git does not ignore.
  - Files larger than 50 MB (configurable) are hashed in full, so two snapshots never look equal by mistake, but Vet does not keep their content and shows them as `content unavailable`.
  - Ignored files can be included explicitly.
  - Untracked files in folders that belong to a tool by name (`.pnpm-store/`) are never included. This list is fixed.
  - Vet never guesses which files matter.
  - A change of the executable bit or of a symlink is a change like any other. A submodule is recorded as its commit; Vet does not look inside it unless asked.
- **When a review starts (§4.5)**, Vet records the commit and tree of version 1 and copies its uncommitted and untracked content into Vet's content-addressed store (the _content store_, §9). Vet never writes uncommitted content into git's object database, so untracked secrets never end up in something that could be pushed. The changeset is the base against the latest settled snapshot.
- **Keeping content:**
  - While a review is open, every new snapshot's content that git cannot rebuild (uncommitted and untracked files) is copied into the content store, so every version of the review can be shown again later.
  - Content shown on either side of a diff, or under a comment or a seen mark, is copied when first shown. Review state therefore never depends on git keeping old objects.
  - The working tree's uncommitted content is copied ahead of a review only while the review screen shows it.
- **Session refs:** while a review is open, Vet keeps refs on the commits its snapshots come from, so git's garbage collection never removes them. They are deleted when the review is finished.
- **Materializing a snapshot** (writing it to a directory to run a language server on it) works only while all of its file contents are available, from git or from the content store. If any are missing, the language features for that snapshot are disabled with a visible reason. Vet never starts a server on an incomplete copy.

### 4.2 Watching and settling

- **Watching:** Vet watches the working tree for file changes and re-hashes what changed. Watching only makes Vet faster; reconciliation (below) decides what the snapshot is.
- **Settling:** a burst of writes is debounced: a snapshot is _settled_ once the changed files' size and modification time have stayed the same for about 1–2 seconds. Each settled snapshot is added to the edit timeline. If two writers change the same file within that window, the result is one snapshot, and the two cannot be told apart.
- **Reconciliation:** Vet scans the whole working tree when it opens it, when the watcher reports that it lost events, after a resume or restart, and before a snapshot settles. Lost watcher events can delay review state but never make it wrong. Where the watcher cannot cover the tree, Vet polls instead and shows a notice.
- A working tree that is not the current changeset and has no open review is not scanned. Vet scans it once when it becomes the current changeset.
- A folder Vet cannot read does not stop the scan. Vet skips it with a warning: tracked files in it are taken from git's index and marked as content unavailable, and untracked files in it are left out. A folder that disappears during the scan counts as deleted.

### 4.3 Files as git sees them

- **Git:** Vet reads git inside its own process. It never starts git, and never runs hooks, `fsmonitor` commands, or filter or diff drivers.
- **Paths** are relative to the repository root, use `/`, and are compared by their exact bytes (as git stores them for tracked files, as the filesystem reports them for untracked ones). Unicode normalization is used only for display and to match git's `core.precomposeUnicode` on macOS. Two paths that differ only in case or only in Unicode normalization are flagged as a `PathCollision`; Vet does not materialize such a snapshot on a filesystem that would merge them, and says so.
- **Content:** a file's content is what git would store, after git's built-in conversions only (`eol`, `text`, `core.autocrlf`, `ident`). Otherwise every file under `autocrlf` would look changed. For files with a custom filter (for example Git LFS): if the file's size and date match git's index, Vet uses the index's content; otherwise it uses the bytes on disk and marks them `unfiltered`.
- **Mode and kind:** where the filesystem cannot store them (`core.filemode=false`, `core.symlinks=false`, usual on Windows), they come from git's index for tracked files. Untracked files get mode `100644` and kind `file`.
- **Fetch:** Vet fetches from the remotes inside its own process, read-only, on request and every few minutes, so remote branches and pull requests are current. It never pulls, merges or rebases.

### 4.4 Bases

- **History:** Vet's own timeline of settled snapshots is the record of what happened. The reflog is extra information (which commit a snapshot came from, comparisons across amends and rebases); git may delete it.
- **The base depends on the kind of changeset. The user does not choose it.**
  - A pull request's base is the point where it branched off its target branch, as the platform shows it (the merge base with the fetched target branch).
  - A local branch's base is its merge base with the default branch. A branch built on another branch is also compared with the default branch.
  - The working tree's base is `HEAD` at the moment it was opened. Vet keeps this base across restarts. Commits an agent makes on top do not move it, so the working tree shows everything done since, committed or not. The base stays until the review of the working tree is finished; the next time the working tree is opened, Vet takes the `HEAD` it finds then. Before a review starts, this base is the only thing Vet keeps, so closing and reopening Vet before the first mark does not move it.
- The base is never the tip of the target branch, because that would show what the target gained since as reverse changes. A changeset whose tip the target already contains is empty, and Vet says so (§8.5).
- The default branch is not offered as a changeset: the others are compared with it. Comparing two arbitrary commits is not a review and is not offered.
- When a branch's or pull request's merge base moves (a rebase, a merge of the target), the base moves with it, as a new version.
- The line counts in the queue use the same base as the review screen, so the two agree. Until Vet has counted them, a pull request's row shows the platform's own counts.
- **`HEAD` off the base:** when the working tree's `HEAD` no longer descends from its base (the agent rebased, reset or checked out something else), the changes shown include work from elsewhere. A banner says so for as long as this is true, and the header marks the base (§8.2).

### 4.5 Reviews and versions

- **Browsing:** opening a repository shows the working tree against its base. Vet records no versions, keeps no content and creates no refs.
- **A review starts** with the first review action: marking something seen (a range, a hunk or a file), acknowledging a non-text change, or adding a comment. Removing a mark and reading do not start a review.
  - The snapshot at that moment is version 1. Vet records its commit and tree, and the base (§4.4) becomes the review's base. Each later settled snapshot is the next version (v2, v3, …).
  - A pull request's versions are the pushes its platform recorded (GitHub's force-pushes, Azure DevOps' iterations), from the first push. A review that starts has one version per push made so far, the current head being the last. Each later push is a new version, however many pushes happened between two fetches. Vet fetches their commits from the platform.
  - A push whose commits the platform no longer has (dropped after a force-push) is still a version, but without content. It is listed and cannot be opened, and the pushes around it are compared with the nearest version that has content.
  - A local branch's versions are the tips Vet saw.
  - Keeping content and session refs (§4.1) apply from the start. A review stays open, and keeps recording versions, after it is complete (§5.3).
  - While a review of the working tree is open, every settled snapshot becomes a version, whichever screen is shown.
- **Finishing a review** is an explicit action, allowed with changes still unseen. Vet records whether the review was complete, deletes the session refs and stops recording versions. Later edits are browsing again until the next review action starts a new review. Finished reviews are kept as §9 says.
- **Reopening** a repository with an open review continues it: the working tree settles as the next version and the marks are remapped (§5.2). Without an open review, Vet opens in browsing.
- **Changes between versions:** a version, or a range of versions, can be shown on its own, as an interdiff (§3). The queue's count of files changed since the last review uses the same interdiff, so a change the author undid, and a change that only comes from the base move, are not counted.
- **By commit:** a branch or pull request is shown either as all its changes at once (the default, and what completion counts) or commit by commit. By commit, a clean merge of the target branch is a collapsed marker with nothing to review, and a merge with conflict resolutions shows only the resolutions (as `git show --remerge-diff`).

### 4.6 The review filter

- The review filter decides which files of a changeset are reviewed: review roots, whether binary files are hidden, and include and exclude patterns. It is a project setting, stored per repository and used for every changeset. Changing it never changes snapshots, so it never disturbs comments or marks.
- **Generated files** are excluded like files matching an exclude pattern, unless the filter's "review generated files" option is on. Vet treats these as generated:
  - lockfiles and tool output by file name (`Cargo.lock`, `yarn.lock`, `*.pb.go`, `*.min.js`, …, in any folder);
  - files marked `linguist-generated` in `.gitattributes` (on the side the file is on; `-linguist-generated` keeps a file in);
  - files whose first five lines (within the first KiB) contain `<auto-generated`, `@generated` or Go's `// Code generated … DO NOT EDIT.`.
- **Review only this folder** narrows the current review instead of changing the filter. The narrowing replaces the project's filter for this review only, is not saved, and ends when the project's filter changes or another changeset becomes current.
- §8.10 describes how the filter is edited.

## 5. Review state

### 5.1 Anchors and comments

- **What a comment is on:** a line range on one side (an anchor), a whole file on one side, or the whole changeset.
- **A thread** has its messages (author, text, time; the first opens the thread) and a status in Vet, open or resolved. A thread linked to the platform also shows the platform's own status as the platform names it (for example Azure DevOps' "Won't fix"), whether it can be resolved there (not a comment in a flat conversation), and whether it blocks the merge (§7.2).
- Only comments on a line range are remapped as below, and only they can become `Stale` or `Ambiguous`. A comment on a file follows the file through renames git detects, and becomes `Stale` only when the file is deleted. A comment on the whole changeset never moves.

**Remapping a comment.** A comment is remapped on the same side of the changeset it is shown in, directly from the snapshot it was made in to the snapshot shown, never through the versions in between. The result is one of:

- **`Exact`:** the commented bytes are where the diff maps the range, at the same path or a path git detected as a rename. If only whitespace differs, the comment is exact with "whitespace changed".
- **`Inferred`:** Vet found one clearly best place, by the surrounding lines or by the same content in another changed file (a move), and records how it found it and how sure it is. When the commented lines were rewritten, the comment moves to the lines that replaced them, with low confidence. When they were deleted, it stays where they were, and the UI says "line deleted".
- **`Ambiguous`:** several places are about equally good.
- **`Stale`:** the file was deleted, or none of the surrounding text survived.

Vet never moves a comment without showing it:

- In the UI, an `Inferred` comment shows a "moved" badge with Vet's confidence; its tooltip says how it was found and where it was. "Confirm" fixes it at its new place (→ `Exact`). Comments remapped with low confidence are also listed under "Moved" in the Comments tool window.

### 5.2 Seen, flags and remapping

- **A span** covers reviewed lines on the old side, the new side or both (a pure addition has no old side, a pure deletion no new side). It is `Seen`, `SeenAgainstOldBase` or `Flagged`.
- **Marking a hunk seen** (or a selection) stores one span covering its lines on both sides. A hunk shows as seen when every changed line in it is covered by a `Seen` span. Marking seen can be undone. A concern is a comment (§5.1), not a mark.
- **Flags.** A hunk can be flagged for later: a `Flagged` span over its lines, which does not count as seen. A flag is independent of seen (a hunk can be both). Only flagging again removes it; marking seen or unseen does not. Flags are stored and moved like seen marks, except that they never shrink: a hunk stays flagged while any of its changed lines is flagged, so a flagged hunk that a push splits or edits stays flagged. Finishing the review removes flags with the other marks.
- **When the code changes (I2),** each span is remapped side by side, using the line diff between the file it was made in and the current file, following renames git detects.
  - Only exact matches carry marks. A move Vet only inferred never carries a seen mark.
  - Each reviewed line whose content is unchanged and still in the changeset stays seen. Every other line becomes unseen. One span can therefore become several, or none.
  - If a reviewed line could map to more than one place (for example, repeated identical lines whose surroundings changed), it becomes unseen, rather than leaving the choice to the diff algorithm.
  - A seen region of 20 lines with 3 lines edited in the middle becomes up to two seen spans around 3 unseen lines.
- **Base changes.** If a span's new lines are unchanged but any of its old lines changed, its marks become `SeenAgainstOldBase`. They look different, count as unseen for completion, and can be confirmed again in one action. This only covers changes inside the reviewed lines; changes elsewhere show up in the impact questions (§6.3), not in the marks. When in doubt, Vet marks a line unseen rather than keeping it seen.
- **Non-text changes.** A change to a file with no text to review (mode, symlink target, submodule commit, binary or oversized content, a pure rename, an added or deleted empty file) is one item. It is acknowledged like a hunk and stored the same way, identified by the file's entries before and after.

### 5.3 Completion

Completion counts only what the review filter includes. A changeset is complete when every changed line the filter includes is covered by a `Seen` span, and every non-text change it includes is acknowledged. The UI always shows how many changes the filter excludes, so "complete" never suggests that everything was seen.

## 6. Language features

### 6.1 Language servers (I3)

| Language   | Server                            | Docs in hover for dependencies |
| ---------- | --------------------------------- | ------------------------------ |
| Rust       | rust-analyzer                     | cargo registry sources         |
| TypeScript | typescript-language-server        | JSDoc from `.d.ts`             |
| Svelte     | svelte-language-server            | as TypeScript, plus components |
| Angular    | Angular Language Server           | as TypeScript, plus templates  |
| Python     | Pyright                           | docstrings and typeshed        |
| C#         | roslyn-language-server            | XML docs in NuGet packages     |
| YAML       | yaml-language-server              | schema descriptions (§6.5)     |
| JSON       | vscode-json-languageserver        | schema descriptions (§6.5)     |
| Shell      | bash-language-server              | none                           |
| Dockerfile | dockerfile-language-server-nodejs | instruction docs               |

- The Svelte server loads the project's `svelte.config.js`.
- The Angular server handles only `.html` templates under an `angular.json` (the nearest one up from the file); without one, no server handles the HTML. TypeScript files, inline templates included, use the TypeScript server.
- The Shell server runs ShellCheck when it is installed.
- The Dockerfile server works offline (Docker's own server needs Buildx and calls Docker Scout).

- A file belongs to the language of its extension or, for files like `Dockerfile`, its name. No two languages claim the same one. The backend decides which language a file belongs to and tells the window; the window does not match file names itself. A kind of file several projects share, such as an Angular app's `.html`, is decided by the project's own files.
- The user can set the path of a known server. There are no plugins for other languages.
- **Required:** the server must accept file contents from Vet (text document sync), because Vet sends it the files it asks about. Vet refuses a server without it, with a visible error.
- **Optional, per feature:** `documentSymbol`, `definition`, `references`, `implementation`, `hover`, `callHierarchy` (incoming and outgoing), `workspace/symbol`. Vet always asks for diagnostics (pushed or pulled), because servers publish them without announcing it.
- Vet does not send a request for a feature the server does not support. Instead it answers "not supported by this language's server" (I3), and every place that would show the answer says so:
  - Go to Definition says it (a hover popup stays empty, as it does for a symbol the server knows nothing about);
  - the Symbol panel says call hierarchy is not available for its callers and callees;
  - a workspace symbol search lists the languages whose servers cannot be searched.

  A server that supports only some features (only diagnostics, or a YAML server without references) is still used for the rest.

- **Status:** for the old and the new side of the current changeset, the backend reports what each language's server is doing (starting, indexing with its progress, ready, failed, not installed), whenever it changes. The footer shows this, with details on click. Nothing waits for a server for a fixed time.
- **Cancelling:** a request whose answer is no longer needed (a hover the pointer left, a question about an outdated revision) is cancelled at the language server too.

### 6.2 Servers for each snapshot (I1)

- **When servers start.** Language servers, and the dependency restores they need, start for a changeset when the review screen shows it, not when the queue selects it. Moving through the queue's rows starts nothing, and no background work (problems, symbols, outlines, marks in the editors) asks a server while the queue is shown. Showing the review screen starts the servers of the current changeset; while it is shown, a new snapshot, a fetch that moves the current pull request, or a change of trust starts the servers they need. A search the user starts, such as a symbol search in Search Everywhere, starts what it needs on any screen.
- **The working tree** is served by a server running in the working tree itself. Its answers are about the revision on screen, even when the files on disk have moved on since.
- **Any other snapshot** (a pinned base, an older version, a pull request's base or head) is served by a server in a directory where Vet has materialized the snapshot. The language-server layer only gets the directory and never sees how it was made. Vet materializes it from the snapshot's commit and its content store, and refuses if content is missing (§4.1).
- **Running servers** are kept per snapshot (or working tree), server and analysis policy, and stay running while used.
  - When too many run, servers that are quick to start again, and those unused longest, stop first. Vet never stops a server the current changeset needs for its changed files.
  - Answers outlive their server (I1).
  - A server that fails to start is not tried again by every question. Installing it, changing its path or "Restart" tries again at once.
  - Changing a server's settings restarts it. Its cached answers no longer match because the server's fingerprint changed; nothing is deleted.
- **Materialized snapshots are kept between launches** in Vet's data directory: one directory per snapshot and analysis policy, shared by all languages, with a record of each language's dependency restore. Reopening a review with the same base starts its servers without materializing or restoring again. Before reusing a restore, Vet checks it against the current tools and the policy. Old directories are deleted to stay within a disk budget (§9).
- **Dependency restore.** A materialized snapshot needs its dependencies restored for its server to work.
  - By default Vet only downloads them and runs nothing. That is enough for documentation (cargo registry, `.d.ts`, typeshed, NuGet XML docs): no install script, `.pnpmfile.cjs`, build backend or MSBuild step runs.
  - Where the policy allows running project code (§6.5), install scripts and build steps still never run. Only a C# restore (which runs MSBuild) needs that policy.
  - When the npm lock file does not match `package.json`, Vet installs the locked versions anyway, without touching the lock file. A banner says "Dependencies installed with npm install: the lock file is out of sync", milder than the "analysis limited" banner. If something also failed, the "analysis limited" banner stays and lists this note in its details.
  - A failed step shows the part of the tool's output that explains it. The full output is in the debug log.

### 6.3 Questions Vet answers

- An answer the server gives while it is busy (indexing, or analysing after an edit) is a _partial answer_: it may be incomplete, so Vet never caches it. Diagnostics given that way are marked, and Vet asks again when the server is done.
- Some answers come straight from a server. Others Vet computes from server answers and the diff; these are labeled derived and show what they were computed from (I3).
- When a server points at a file outside the repository (a dependency, the toolchain's sources), Vet shows the line only if the file is of that server's language (§6.1; the Svelte and Angular servers may also point at TypeScript files). Other files are listed without their text.

**From a server:**

- **Hover, go to definition, references, diagnostics:** the server of the side being viewed.
- **Navigation on the old side, next to the new side:** both sides' servers, asked at the same time.
- **Workspace symbols** (Search Everywhere): `workspace/symbol` on the new side's server.

**Derived:**

- **Which symbols changed:** for each changed file, `documentSymbol` on both sides; a symbol changed if its range overlaps a hunk.
- **Usages of a symbol:** `references` on the new side, in the background, each with the symbol it is in.
- **What a removal breaks:** `references` of each removed symbol on the old side. Each place is then checked on the new side and classed as:
  - **gone**;
  - **modified**;
  - **likely broken:** unchanged, and no longer resolves;
  - **binding changed:** unchanged, but now resolves to something else;
  - **unknown**.
- **Is a diagnostic new:** diagnostics of each changed file on both sides.
  - A new-side diagnostic is old if the old side has one with the same severity, source, code and message (whitespace collapsed) on a line that maps to it. Otherwise it is new.
  - It is _unknown_, never new, while the old side's diagnostics are missing or partial.
  - New-side diagnostics show at once, marked "origin pending".
- **Changed callers and callees of a changed symbol:** incoming and outgoing calls on the new side, followed through changed symbols only. Unchanged direct callers are listed as possibly affected.
- **Is it a test:** by the file's path (`tests/`, `*_test.go`, `*.test.ts`, `*Tests.cs`, …) or an enclosing `tests` module. Shown as "test by file name".

### 6.4 Reachability (derived, I3)

Reachability says whether new or changed code is reached at all. From each changed or added symbol, Vet follows incoming calls backwards until it hits a configured entry point, within limits on depth and number of symbols. The result is labeled derived:

- `reached` shows the path to the entry point;
- `unreached within bound` shows the limits and the entry points used;
- `unknown` names what servers cannot see (dependency injection, reflection and serialization often hide real calls, worst in C#).

Finding dead code in the whole repository is not a Vet feature. Vet shows it only when a language server reports it as a diagnostic (for example rustc's `dead_code` through rust-analyzer).

### 6.5 Trust

Trust decides whether Vet may run project code while analysing. It is not a security boundary.

- The analysis policy has two parts: whether language servers may run project code (build scripts, proc macros, MSBuild design-time builds), and whether dependency restores may (§6.2). The policy is part of the key of materialized snapshots, restore records and cached answers, so a restore that ran nothing is never reused as one that ran code, and the other way round.
- When trust changes, Vet starts the servers the current changeset needs under the new policy, and stops the running servers that run project code under a policy the current changeset no longer has. A server in use stops as soon as its request ends.
- **Trusted project:** a per-repository setting ("Trust this project" on the Trust page of Settings), off by default. In a trusted project, the working tree, local branches, and pull requests whose head is in the repository itself (not a fork) are analysed with restores and servers that run project code, without asking.
- **Project not trusted:** the working tree and local branches still use servers that run project code, because they are the reviewer's own code, but their restores run nothing. A pull request is limited until trusted: restores run nothing, and only servers that do not run project code start. Other languages show an "analysis limited" marker, and a banner says why and offers "Trust this PR" and "Trust project…" (which opens Settings on the Trust page).
- **Pull requests from a fork** (the head is in another repository than the one the pull request targets, or Vet never stored the platform's data on it) are limited in a trusted project too, and their banner does not offer "Trust project…".
- **Trust this PR** lifts the limit, for restores and servers alike, for the pull request's current head commit. A new push removes the trust again. A pull request whose head is in a trusted project's own repository is trusted by the project, and a push does not change that. There is no action to remove trust from a pull request; unticking "Trust this project" removes the project's.
- A pull request's old side runs project code only if its project is trusted, or the pull request is trusted at its current head (the old side is an ancestor of that head). Trusting a pull request covers its head commit only.
- **TypeScript.** The TypeScript server runs a `tsserver.js` through node, and the project may bring its own `typescript`. Where servers may not run project code, Vet uses only the `typescript` it installed itself, or else one installed next to the server (the user's own), never the project's. Without either, the server is reported missing, and the language server settings offer to install it. The Angular server likewise loads `@angular/language-service`: where servers may not run project code, Vet points it only at its own copies (installed with the server), never at the project's `node_modules`, and without them reports it missing. The settings page shows each server as it would run for the current changeset.
- **Schemas.** The YAML and JSON servers check files against JSON schemas and would download them (SchemaStore, a `$schema` key, a modeline). Since the repository decides what is downloaded, this follows trust: downloads are allowed only where restores may run code (a trusted project, a trusted pull request). Everywhere else downloading is off, and the servers get a fixed set of schemas that ships with Vet (GitHub Actions workflows, Azure Pipelines, Kubernetes, docker-compose, `package.json`, `tsconfig.json`). Any other schema is refused with a visible diagnostic naming its address, never skipped silently. Where schemas come from is part of the server's fingerprint, and answers obtained with downloads are not reused in later sessions, so answers under different schema rules never mix.

### 6.6 Background analysis

While the review screen shows the current changeset, the backend analyses it in the background and sends each result as soon as it has it. The window does not ask file by file.

- **Changed symbols:** the changed symbols of each changed file, files with an open tab first.
- **Problems:** the diagnostics of each changed file, files with an open tab first. First the new side, marked "origin pending", then the comparison with the old side (§6.3).
- If a file's analysis fails for a reason that may pass (a storage read, a server that is starting, indexing or restarted, a request that failed), Vet tries again a few times before it reports the failure. A file that cannot be analysed for a lasting reason (no server for its language, an untrusted project, a binary or too-large file, a file the changeset does not change, a snapshot that cannot be materialized) is reported at once.
- The analysis follows the current revision. A filter change restarts it for the files the filter now includes. Retry in the Problems tool runs it again.

## 7. Platforms

### 7.1 Accounts

- GitHub (including Enterprise hosts) and Azure DevOps. Vet uses a signed-in command-line tool if there is one (`gh auth token`; `az account get-access-token` for Azure DevOps), otherwise a personal access token. There is no browser or device-code login.
- Tokens are kept in the operating system's keychain, never in Vet's storage. In the WSL split, the window keeps them and hands them to the backend. A token is sent only to its own host's API.
- Signing in or out for a host syncs the open pull requests of that host again and reads their checks again.

### 7.2 Platform traits

Each platform is described by a fixed set of properties. Vet and its UI decide by these properties, never by the platform's name:

|                                    | GitHub                            | Azure DevOps                                               |
| ---------------------------------- | --------------------------------- | ---------------------------------------------------------- |
| Comments on the whole pull request | a flat conversation               | resolvable threads                                         |
| A conversation feed                | yes                               | no                                                         |
| Posting                            | one review with all comments      | comment by comment                                         |
| Votes                              | comment, approve, request changes | approve, approve with suggestions, wait for author, reject |
| Vote set apart from posting        | no                                | yes                                                        |
| What blocks the merge              | unresolved threads, if required   | unresolved threads, if a policy says so                    |
| Comments belong to a patch set     | no                                | no                                                         |

How the UI uses them:

- **Review and Conversation tabs** in the Comments tool window, for every pull request. Review holds the comments on files and lines. Conversation holds what is about the pull request as a whole: with a conversation feed, the platform's flat list (comments and review summaries with their vote), where your own comments can be edited and are answered with a new comment instead of a reply; without one, the comments on the whole changeset, with replies and Resolve.
- **Posting:** the Comments tool window shows "Submit review" with a vote menu where comments are posted as one review, or "Post" where they are posted one by one, next to the drafts it will send. Where the vote is set apart, a separate vote control appears next to it.
- **Blocking:** the blocking marker on a thread, and the queue's blocking status, follow what blocks the merge on that platform.
- **Platforms that tie comments to a patch set:** Vet still attaches comments to its own snapshots; a platform comment's patch set maps to the snapshot built from that patch set.

### 7.3 Pull requests

- Pull requests are changesets. Vet fetches their heads and targets into its own refs, read-only.
- **The backend keeps the list.** It lists the pull requests again every fetch interval (whatever screen is shown), when the refresh button is pressed and when an account changes, but not just because the queue, the switcher or Search Everywhere opens. Once listed, reading the list never waits for the platform.
- A timed or account-change refresh skips a rate-limited account until its retry time; the refresh button always asks.
- A repository whose listing fails keeps its last rows and shows the error.
- The current pull request is always listed, even when the platform's list leaves it out, from what Vet read when it opened it.
- **Details:** Vet keeps each pull request's details as the platform last gave them (description, reviewers, labels, conversation, state). Selecting one shows them at once and refreshes them. After listing, Vet fetches in advance the details of the newest pull requests whose `updatedAt` changed since its copy, at a slow pace, and pauses while their account is rate limited or failing. A pull request that is gone is skipped. If the list changed, this starts again from the top; if not, it continues.

### 7.4 Comment sync

- Comments sync both ways: comments and replies on the platform are pulled into Vet, and drafts in Vet are posted when the user posts them. Seen marks are never synced. Because Vet keeps the commented content (§4.1), comments stay in the right place across rebases where the platform loses them.
- If the user changed a thread's status in Vet since the last sync, that status wins over the platform's.
- An edit to a posted comment is sent at once. If the comment also changed on the platform since the last sync, Vet shows both texts and the user picks one.
- A thread deleted on the platform is deleted in Vet; a draft reply in it is dropped with a notice.
- Drafts are posted on Submit, as one review (GitHub) or one by one (Azure DevOps), never as a pending review on the platform. A comment the platform refused is marked failed with the reason; it is posted again with the next Submit, or can be discarded on its own. A comment being posted cannot be discarded.
- Users can edit and delete their own comments. Deleting the first comment deletes the thread.
- A failed sync shows in the footer until a sync succeeds or another changeset becomes current.

### 7.5 Checks

- Vet reads CI results per commit. While the queue or the review screen shows the current pull request, the backend keeps its checks up to date and sends them: every 30 seconds while a run is unfinished, and shortly after an action on a run, because the platform shows the result with a delay.
- When the account is rate limited, Vet waits until the limit ends. A failed checks refresh shows in the footer until a refresh succeeds or another changeset becomes current.
- §8.7 describes how checks are shown (the Pipelines tool).

## 8. The window

### 8.1 Queue

- **The queue is the start screen**, reached through a Queue / Review switch in the header. It lists every open pull request of the project's repository and the local branches, with more details and filtering than the changeset switcher in the title bar. Its tabs are All, Blocking, Local and Pull requests, with a text search.
- Rows are sorted by blocking status, then by date. A row shows the title, author, branch, CI state, comments and when it was updated, with a status chip such as "waiting for you". It also shows how much is reviewed under the current filter, and how many changes the filter excludes (§5.3).
- The panel next to the list shows the selected changeset: for a pull request its description, reviewers and checks; the files changed since the last review; and its comments.
- Marks survive rebases and force-pushes.
- A project has one current changeset. Switching to another and back restores its open tabs, scroll positions and current file. Switching changesets, closing or opening a project, or quitting asks first while a comment box holds text that was not added ("Discard your comment?", with Keep editing as the default).

### 8.2 Header

- The header reads `project  ⎇ changeset  [base]  [scope]`, with the review's progress and Finish Review on the right, showing the progress of what is on screen. Project and changeset are separate controls, with no arrow or slash between them.
- The **base** is shown only for a pull request into a branch other than the default ("into release/2.1"), and while `HEAD` is off the working tree's base (§4.4, with a warning mark and the base's short hash). It is plain text, not a control. Its tooltip gives the base commit's hash, subject and age.
- The **scope chip** says what the review screen shows: "All changes", "Push v2", "Pushes v2–v3" or "Commit 5a6b7c8" (the commit's subject in the tooltip). It shows no version number. A cross on it, and Esc, go back to all changes (there is no cross on "All changes"); a click opens or focuses the timeline. For the working tree it ends in a dot while it shows the working tree, hollow while edits are not yet a version.
- The header has no base chooser and no "back" button. The footer does not repeat what is compared.
- The header's checks button shows the commit's checks state and opens or focuses the Pipelines tool. It never stays pressed.

### 8.3 Tabs and views

- **The Changes tab** is the fixed first tab. It is named like the scope chip ("All changes", "Push v2", "Pushes v2–v3"; the tooltip counts the changed files). It shows the changes as hunks with a few lines of context; unchanged regions are collapsed and can be expanded. While scrolling, the current file's header and the current hunk's bar stay at the top until the next one pushes them out. Tab titles show `+N −M` line counts.
- **File tabs.** A file has one tab (closable), whatever it shows. The tab has a view mode: Unified, Split (the file's whole diff) or File (the file's text with its changes marked, read-only).
  - The Unified/Split/File control, Alt+Shift+1/2/3 and the View menu change the active tab's mode, and never open or close a tab. Each mode keeps its own cursor and scroll position, and the cursor's line carries over.
  - A file the changeset does not change (opened by Go to Definition, or from the file tree) has a File tab only.
  - File mode marks added, changed and removed lines (removed ones where they were), coloured by review state. Clicking a mark shows the hunk's old lines with the hunk bar's actions (seen, flag, comment, open in the diff), so a change can be marked seen right there.
- **Tabs at a commit.** A file opened while a commit is selected (by commit) gets a tab that stays at that commit, whichever commit or All changes is selected later.
  - It shows the file as that commit leaves it, with that commit's hunks and marks; Jump to Source and flagged-hunk stepping from it stay at that commit. It shows no comments and allows no commenting.
  - The tab has a commit icon. Under the tab strip, one line shows the commit's short hash, subject, author and time ("merge" for a merge), with the tooltip "Comments are made in All changes".
  - A file opened at several commits (or also on the whole changeset) has a tab for each; their titles then show the short hash, and the tooltip names the full commit.
  - A file opened on the whole changeset stays there when a commit is selected. A tab whose commit is no longer in the changeset (after a rewrite) closes.
- **Opening a file.** Jump to Source switches the file's tab to File mode at the line, opening the tab in that mode if needed. It is reached by double-click or Enter in the tree or the Symbol list, double-click on a file header or a diff, a hunk header's button, or F4 at the cursor (same line). Double-clicking a line in the Changes tab opens the file's tab at that line, highlighted. The file icon in a file header opens the file's tab.
- **Go to comment** shows the comment in the file's tab, in that tab's mode, if the file has a tab open (not a tab at a commit, which shows no comments); otherwise in the Changes tab. A comment on the whole file sits above the first line in File mode, and above the first hunk in a diff.
- **Long lines** can be read in every view: horizontal scrolling when soft wrap is off, with hunk bars, cards and comment cards staying in view; soft wrap for prose (Markdown, reStructuredText, plain text) by default, with a setting. A changed line wider than the view is marked, wrapped or not.
- The clicked line stays highlighted. Diagnostics and other hints are not shown in collapsed unchanged regions.

### 8.4 Hunks and review actions

- **A hunk shows** its review state (including `SeenAgainstOldBase`), which changed lines are covered, whether changed symbols are reached (labeled derived), notes on whitespace-only changes, and diagnostics as gutter markers, counted in the footer.
- **Non-text changes** (mode, symlink target, submodule commit, binary, content unavailable) show as a card with a hunk-style header: the same "mark seen" action and key, included in hunk navigation, and a before/after preview for images. A file with both shows the card above the first hunk.
- **Keys:** hunk navigation (Ctrl+Alt+Down/Up), mark seen (Space) and commenting work in every mode. Go to Definition is Cmd+click or Cmd+B (Ctrl on Windows and Linux), on either side; the symbol under the pointer is underlined while Cmd/Ctrl is held. A new comment is Ctrl+Alt+C (or the editor's context menu), on the selection or else the cursor's line, in every view, in the last editor used even if focus moved to a toolbar or the tree since. A hunk's Comment button comments the whole hunk, or the selection if there is one inside the hunk.
- **Flags:** a flag button next to the seen button in the hunk bars and in File mode's hunk toolbar (highlighted while flagged); Toggle Flag in the Review menu; Next and Previous Flagged Hunk in the Navigate menu, across files, in All changes and in file tabs. Finish Review… warns with the number of flagged hunks left and offers to go to the first one; finishing anyway is allowed.
- **Hover popups** look like the rest of the app (surface, border, radius and shadow), with a maximum width and height and scrolling, and even spacing between signature, docs and diagnostics, in both themes. The server's Markdown is rendered, with code blocks highlighted.
- **Diff:** Vet shows and marks the byte-level diff. Hiding whitespace, detecting moves or hiding formatting changes are only ways of displaying it, and every hidden change is one click away.

### 8.5 Nothing changed

A Changes tab with no files is never blank. It says why, always with the same parts (an icon, a title, one sentence, at most one panel of rows, at most two buttons), and shows rows only for facts the backend has. The reasons:

- the working tree has no edits yet (it shows the watched folder, `HEAD`, and what it is compared with);
- the working tree's edits were undone (its versions, each can be shown);
- the target already contains the branch or pull request (the commit that merged it, or that it has no commits of its own yet);
- the pull request was merged (its source branch, the merge commit and your comments);
- the filter hides every file (the files by rule, each group with "Review these", which narrows the review to them);
- the narrowing leaves nothing (the folders with changes, each with "Review only this", and a way back);
- the commit changes no file;
- the push changed nothing (the two versions with their bases).

While the tab is empty, the stepping and Unified/Split controls are disabled, the header's progress reads "nothing to review" instead of 0 %, and Finish Review stays available.

### 8.6 Timeline

- A bottom panel (Alt+5) listing the settled snapshots, newest first, with a grey row for the base under v1 that cannot be selected. A version without content (a push whose commits are gone) is a dimmed row that cannot be clicked, with "unavailable" where the other rows name their files.
- Clicking a row shows that push's own changes, compared with the version before it (the base for v1). Shift-click extends the selection to a range of pushes. Clicking the only push shown, Esc, or the scope chip's cross goes back to all changes. Filled rows are the pushes shown; a ring marks the version they are compared with. Selecting a row leaves the by-commit view.
- "Since my review" shows the pushes after the version last reviewed. Ctrl+Alt+L switches between that and the whole changeset.

### 8.7 Tool windows

Tool windows sit on left and right rails and can be resized. Clicking the active tool's icon hides its window but keeps it selected. Bottom tools share one area, one at a time; its height is set with a splitter above it (mouse or arrow keys) and remembered. Shortcuts follow the order left, bottom, right: Alt+1…8 (Changes, All files, Commits, Comments, Timeline, Problems, Find, Pipelines), Alt+9 (Symbol) and Alt+0 (Structure).

- **Changes tree.** The changed files, with the file you are in marked (the one at the top of the Changes tab, or the active file tab's). Changed symbols are listed under their files, which are collapsed at first and show how many symbols they have. A button in the tree's header expands or collapses all (a file that appears later follows the last choice).
  - Only named containers nest. An unchanged container that holds a changed symbol is shown greyed, without a change letter, and clicking it only opens or closes it. A removed symbol (only on the old side) is struck through in place under its container. The letter M, A or D stands in the symbol's status column. Symbols have no seen marks of their own: marks stay on hunks and files.
  - Each file shows a spinner while its symbols are being found (until the last retry, §6.6), and the tree's header shows how many files are done, with a progress bar.
  - Moving the cursor into a changed symbol in any editor selects it in the tree: its file opens and its row scrolls into view, without moving focus. Double click, Enter or Jump to Source (F4) on a changed symbol opens its definition (the old side's for a removed symbol). ← and → open, close and move through the rows. Speed search also matches symbol names.
  - Excluded files are collapsed into one group that shows the excluded count and opens to the files, each with the reason (the project's rule, or "outside the narrowing").
  - "Exclude from review" changes the project's filter. "Review only this folder" narrows the current review (§4.6) and shows as an "Only src/api ×" chip in the Changes tool's header; the cross removes it.
- **Comments** (Alt+4): the Review and Conversation tabs (§7.2), the comments waiting to be posted and the vote, and Post Comments… / Submit Review… (Ctrl/⌘+Shift+K). Ctrl+Alt+V opens a vote picker at the toolbar's outgoing button, on platforms where the vote is separate from the comments.
- **Problems** (Alt+6): the diagnostics of the whole changeset in one list, new ones first, with its own icon in the left rail. The icon's badge and the footer count the changeset's new errors and warnings ("No new problems" when there are none, "N problems · comparing" while the comparison with the old side runs).
  - A file whose old side cannot be analysed has its problems listed separately as of unknown origin, never as new, and the footer says how many files that is.
  - Filters: scope (changed files, open files, this file), severity, new or all, and text. Grouping by origin, file or severity. Options for problems in hunks not yet seen, or on changed lines only.
  - A click shows the problem in the current tab without taking focus; Enter or double click is Jump to Source. F2 and Shift+F2 (Next and Previous Problem) step through the current file's problems from the cursor, wrapping within the file. A tab at a commit is never used. Retry analyses again.
- **Pipelines** (Alt+8, only for a pull request): the CI runs of the shown commit with their jobs and steps, with its own icon in the left rail, whose badge counts the commit's checks and turns red while one fails.
  - Runs fold. The selected job's steps show next to them, with the failing step marked, "Open log (at failure)" and, for any finished job, a log tab titled "<job> · log" (with the attempt after a re-run). Checks a service sets without a run are listed as "status from a service".
  - The toolbar re-runs the failed jobs, re-runs the whole run, and cancels an unfinished run, in the platform's own words ("Re-run failed jobs" on GitHub, "Retry failed stages" on Azure DevOps). A failed job has its own re-run on hover. An action that does not apply is disabled, with the reason as its tooltip.
  - If the token may not change runs, every action is off until the token changes. While the platform rate-limits Vet, the tool shows what it last loaded and when it will ask again.
- **Find** (Alt+7): result lists as tabs. Find Usages (Alt+F7; any symbol, not only changed ones), the Symbol panel's Usages button, and "Open in Find window" (⌘↵) in the Show Usages popup and in Find in Files each fill one.
  - A new result replaces the tab that is not pinned (its title in italics). A pinned tab stays, and running the same search again refreshes its tab.
  - Rows are grouped by file. Each shows the review state of the hunk the line is in, the symbol it is in, and the line with the match. A line from the old code is marked "base" and opens on the diff's old side.
  - "Changeset" and "Not seen" narrow the list. A filter field matches text, symbol and path (Esc clears it, then returns to the editor). A preview shows the code around the selected row. A list cut at 200 rows offers "Show all".
  - Rerun (⌘R) searches again. When the code changed since a list was made, its tab shows a dot and a banner says so, and a rerun reports how many rows are new and gone. Find Usages is not offered in a tab at a commit.
  - Result lists belong to the changeset: another changeset starts with none.
- **Symbol** (Alt+9): one symbol, changed or not: the one under the cursor in an editor, or the one last clicked in the tree, whichever was last. With the cursor outside any symbol (a function, type, module, field or constant), it is empty. For a language without a usable server, it says so.
  - The top names the symbol, its change, its container and namespace, and its file (a click opens it).
  - Below is one tree whose top-level groups open and close and are remembered, all open at first: signature; changed callers; changed callees; possibly affected; reached from; usages. A note at the callers explains, for any language, what a server's call hierarchy cannot see (calls through interfaces or virtual methods, callbacks, reflection, generated code).
  - Rows look like the Changes tree: the member's name in the code font, then its type, line and (if the panel is wide) file in the UI font, never cut off. Each group shows five rows, then "N more" shows the rest.
  - Callers nest by call, so a caller's callers are under it. Possibly affected and usages are grouped by type (by file for a free function); several lines in one member share its row. Tests (§6.3) are greyed and, in those two groups, collected in a Tests branch at the end.
  - For a removed symbol it shows the remaining references on the old side instead, in two branches: those that need attention (likely broken, now resolves elsewhere, unknown), open; and those already handled (deleted, edited), closed.
  - Clicking a changed caller or callee selects it in the tree.
- **Structure** (Alt+0, and as a popup on Ctrl/Cmd+F12 over the editor, both with the same list): every symbol of the current file as a tree.
  - Rows look like the Changes tree: change letter M, A or D, kind icon, removed symbols struck through in place, unchanged symbols greyed, fields and constants included.
  - Header toggles: changed only (the changed symbols and the containers around them) and A–Z sorting, both remembered; folds are per tab. A target button selects the symbol the editor's cursor is in, once; the list does not follow the cursor by itself.
  - Speed search filters by name; Esc returns focus to the editor. A click selects a row and shows it in the Symbol panel; double click, Enter or Jump to Source (F4) opens its definition (Enter in the popup also closes it).
  - A file at a commit shows its outline at that commit without change marks. A file without language support shows the Symbol panel's empty message.

### 8.8 Search

- **Search Everywhere:** one popup under the header for finding things by name. Tabs: All, Changesets, Files, Symbols, Threads and Actions, each with a count while typing. An Unchanged files toggle (off by default, remembered) adds the files the changeset does not change, and their symbols, to Files, Symbols and All.
  - It opens with double Shift on All, Ctrl/⌘+P on Files, Ctrl/⌘+Alt+O on Symbols and Ctrl/⌘+Shift+A on Actions. Switch Changeset keeps Ctrl/⌘+Shift+O.
  - Names match as in speed search (word starts, then runs of letters). Changed files and symbols rank above unchanged ones, and ones not fully seen above seen ones. Open comment threads come before resolved ones. The last 30 results picked in a project (remembered per project) only decide between close matches.
  - All shows the best few of each kind with "Show all N", the kinds ordered by their best result. With nothing typed, it shows the recent results and the changed files seen least. A `:line` after a file name opens the file at that line.
  - Actions are all commands. A command that cannot run stays in the list, greyed, with the reason.
  - Enter opens a file as the tree does: in the current view, so in a tab at a commit while a commit is selected, which a chip and the footer say. Shift+Enter shows a changed file or symbol in All changes. A symbol always opens on the changeset's new side, never at a commit. The last row passes the text to Find in Files.
- **Find in the current view** (⌘F): a find bar in All changes, Unified, Split and File views, with match case, whole word and regular expression, the side to search (both, new, old), and changed lines only. Next and previous with counts; matches are marked in the text and the scroll bar. Stepping expands unchanged lines that hide a match, and All changes also searches the unchanged text of every file. One search per window, with a position per tab.
- **Find in Files** (⌘⇧F): a dialog with the same options, a file mask, and where to search (All changes, a range of versions, one commit, or the whole repository without ignored files). Filters for side, changed lines only, and changed lines not yet seen. Results are grouped by file with review progress and status, with a preview of the code around the selected match, and the first 1,000 matches are listed. A result opens in All changes when that is shown, otherwise in the file's own tab (Alt+Enter: always there), with the find bar holding the search. Lines are matched one at a time. The find bar uses JavaScript regular expressions, Find in Files uses Rust's. Where it searched and the selected result belong to the changeset.
- **Speed search:** typing in any menu, dropdown, list or tree filters it, as in IntelliJ: camel-hump matching, matches highlighted, Escape clears, and trees keep the folders leading to a match.

### 8.9 Keyboard and menus

- Every action has a keyboard shortcut, shown in menus and tooltips; the Keymap page in Settings changes them. Double Shift opens Search Everywhere. The editor font is JetBrains Mono.
- **The main menu** lists every command with its shortcut: File, View, Navigate, Changeset, Review and Help, plus the app menu, Edit and Window in the macOS menu bar (the native one). On Windows and Linux the ☰ button in the title bar opens it, also on the welcome screen; pressing and releasing Alt alone opens it too, then a first letter opens a menu. Alt with Shift held never opens it (on Windows Alt+Shift switches the keyboard layout).
- A menu item that does not apply is disabled, with the reason as its tooltip in the drawn menu, never hidden.
- Close Tab is Ctrl/⌘+W. Previous/Next Tab are Alt+←/→ (⇧⌘[ and ⇧⌘] on macOS, where ⌥←/→ moves the cursor by word) and, on every platform, Ctrl+PageUp/PageDown. The View menu's view mode is one choice of Unified / Side-by-Side / File, on Alt+Shift+1/2/3, matched by the physical key so every keyboard layout works. Alt+F2 shows the file in the tree. Settings open with Ctrl+Alt+S (⌘, on macOS).
- **Context menus** are the operating system's own, everywhere the app has one: the file tree, the editors and diff views, tabs. They replace the browser's default menu and show each item's shortcut. In a plain browser, the app draws the same items itself.

### 8.10 Settings

- Two groups of pages: Application (General, Appearance, Keymap, Language servers, Accounts) and Project, named after the open project (Trust, Review filter). The Project group is greyed with "Open a project first" while no project is open. "Project Settings…" (disabled without an open project) opens it from the app menu and from the bottom of the project switcher.
- **Review filter page:**
  - Review roots as a list, each with the current changeset's changed files under it. The backend counts them. A typed folder is checked against the repository's folders, and "Browse…" picks one from a folder tree.
  - Include and exclude patterns as two lists. Each pattern shows the files and lines it matches in the current changeset (the backend computes this from the draft, never the UI), or why it is invalid.
  - Checkboxes for hiding binary files and reviewing generated files, and the live count of excluded changes.
- Dialogs can be resized and keep their size.

### 8.11 Background work and progress

- While a project opens and while work runs (scanning, diffing, servers starting and indexing, dependency restores), the footer shows a progress indicator naming the step, with details on click. The review screen shows as early as possible instead of a loading screen.
- A failed sync or checks refresh stays in the footer as a warning until one of the same kind succeeds or another changeset becomes current.
- Each language server's status is in the footer (§6.1).
- **Work for the review screen waits for it.** While the queue is shown, Vet keeps up to date only what the queue shows: its lists, line counts and pull request details, and for the current changeset the queue panel's file list, the files changed since the last review, checks, comments and pull request details.

### 8.12 Window and appearance

- One window shows either the welcome screen (recent projects, Open…) or a project, and remembers a position and size for each. The project switcher (recent projects and "Open…") and the changeset switcher are in the title bar.
- Light and dark themes, or following the system, built from shared design tokens. Controls behave like a desktop app: hover, pressed and toggled states, and no text selection outside content.
- File type icons for files and folders in the file tree and on tabs, by extension and well-known names (a folder's icon open or closed).

## 9. Storage and retention

- **Location:** the backend side's own filesystem (in WSL the Linux home directory, never `/mnt/*`), in the usual data directory of each platform.
- **What is stored:** snapshots and their order, sessions, seen marks, comments, the content store, which projects are trusted, and for each pull request whether it is from a fork and which head commit it is trusted at. Also what the platform last said about recently read pull requests besides their comments (description, reviewers, labels, conversation and state), so revisiting one shows them before Vet asks the platform again. A crash never leaves a record pointing at content that was not written.
- **Upgrades:** a newer Vet keeps all data of an older one. An older Vet does not open the data of a newer one, and never deletes it.
- **Settings and preferences:** everything Vet remembers besides review data is stored with it, except tokens (in the keychain) and the window's size and position (a file of the window).
  - The backend's own settings (language-server cache size, how long finished reviews are kept) apply to the whole app.
  - The window's preferences are stored either for the whole app (theme, soft wrap, key bindings, panel and dialog sizes, table columns and sorting, folded sections, the queue's and the switcher's tab, Search Everywhere's Unchanged files toggle, the Structure tool's sorting and filter, and when the Comments tool of each pull request was last opened) or for one project (its open tabs per changeset, and the results last picked in Search Everywhere).
  - A change is saved at once and sent to every window connected to the backend, which follows it.
- **Retention:** content is kept while an open session, an unresolved or unsynced comment, or a seen mark refers to it. Finished reviews are kept 30 days (configurable). Content nothing refers to is deleted.
- **Language-server cache:** a file of its own, apart from the review data. It holds cached answers (I1) within a size limit (256 MB by default, adjustable in Settings), dropping the least recently used. It is disposable: deleting it, or a format Vet does not recognise, only makes Vet slower.
- **Materialized snapshots:** one directory per snapshot and analysis policy. They are kept while an open session refers to them, then the least recently used are deleted to stay within a disk budget (20 GB by default, adjustable in Settings), and after 14 days unused. Settings shows the current use next to a "Clear semantic caches" button.

## 10. Open question

**Risk ordering:** whether the queue should rank changesets by risk, and what it would use to judge risk.
