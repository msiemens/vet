//! The automatic merge of a merge commit's parents, which `git show --remerge-diff` compares the
//! commit with: what the merge brings beyond it is what the person who made it decided.

use std::collections::BTreeMap;

use gix::bstr::ByteSlice;
use vet_app::changeset::CommitId;
use vet_app::ports::{CommitDetail, GitError, Remerged};
use vet_core::content::{RepoPath, Snapshot};
use vet_core::merkle;

use super::tree::tree_entry;
use super::{Inner, chained, err, object_id};

impl Inner {
    pub(super) fn commit_detail(&self, commit: &CommitId) -> Result<CommitDetail, GitError> {
        let repo = self.open()?;
        let found = repo.find_commit(object_id(commit)?).map_err(chained)?;
        let message = found.message().map_err(err)?;
        let author = found.author().map_err(err)?;
        Ok(CommitDetail {
            info: vet_app::ports::CommitInfo {
                id: commit.clone(),
                subject: message.summary().to_string(),
                author: author.name.to_string(),
                time: author.time().map_err(err)?.seconds,
            },
            body: message
                .body()
                .map(|b| b.to_str_lossy().into_owned())
                .unwrap_or_default(),
            parents: found.parent_ids().map(|p| CommitId(p.to_string())).collect(),
        })
    }

    /// The merge of the two parents of `commit`, with the conflicts it could not resolve kept as
    /// markers in the files, as git does for a remerge. `None` unless `commit` has two parents.
    pub(super) fn remerge(&self, commit: &CommitId) -> Result<Option<Remerged>, GitError> {
        let real = self.open()?;
        let found = real.find_commit(object_id(commit)?).map_err(chained)?;
        let parents: Vec<_> = found.parent_ids().map(|p| p.detach()).collect();
        let [ours, theirs] = parents[..] else {
            return Ok(None);
        };
        // The merged files are written to memory only: the repository gains no objects.
        let repo = real.clone().with_object_memory();
        let options = repo.tree_merge_options().map_err(chained)?;
        let options = gix::merge::commit::Options::from(options).with_allow_missing_merge_base(true);
        let mut merged = repo
            .merge_commits(ours, theirs, Default::default(), options)
            .map_err(chained)?;
        let tree = merged.tree_merge.tree.write().map_err(chained)?;

        let index = repo.index_from_tree(&tree).map_err(chained)?;
        let mut entries = BTreeMap::new();
        let mut contents = Vec::new();
        for e in index.entries() {
            if !real.has_object(e.id) {
                let blob = repo.find_object(e.id).map_err(chained)?;
                contents.push(blob.data.clone());
            }
            let entry = tree_entry(&repo, &self.blobs, e.id, e.mode.bits())?;
            entries.insert(RepoPath(e.path(&index).to_vec()), entry);
        }
        Ok(Some(Remerged {
            snapshot: Snapshot {
                id: merkle::snapshot_id(&entries),
                entries,
            },
            contents,
        }))
    }
}
