//! Git object ids and `BlobId`s of the same content, so unchanged tracked files
//! and tree entries are hashed once. In memory for now.

use std::collections::HashMap;
use std::sync::Mutex;

use gix::objs::{Find, FindHeader};
use vet_app::ports::GitError;
use vet_core::content::BlobId;

use super::err;

#[derive(Default)]
pub(crate) struct BlobCache {
    ids: Mutex<Ids>,
}

#[derive(Default)]
struct Ids {
    by_oid: HashMap<gix::ObjectId, BlobId>,
    by_blob: HashMap<BlobId, gix::ObjectId>,
    /// Pinned commit of each submodule entry's `BlobId`, which only hashes it.
    submodules: HashMap<BlobId, gix::ObjectId>,
}

impl BlobCache {
    /// The `BlobId` of a submodule entry pinned at `commit`.
    pub fn submodule_id(&self, commit: gix::ObjectId) -> BlobId {
        let id = BlobId::of(commit.as_bytes());
        self.ids.lock().expect("not poisoned").submodules.insert(id, commit);
        id
    }

    pub fn submodule_commit(&self, blob: BlobId) -> Option<String> {
        let ids = self.ids.lock().expect("not poisoned");
        ids.submodules.get(&blob).map(|c| c.to_string())
    }

    /// The `BlobId` of the git blob `oid`, reading it from the object database
    /// on a miss.
    pub fn blob_id(&self, repo: &gix::Repository, oid: gix::ObjectId) -> Result<BlobId, GitError> {
        if let Some(id) = self.ids.lock().expect("not poisoned").by_oid.get(&oid) {
            return Ok(*id);
        }
        let mut buf = Vec::new();
        repo.objects
            .try_find(&oid, &mut buf)
            .map_err(err)?
            .ok_or_else(|| GitError(format!("missing object {oid}")))?;
        let id = BlobId::of(&buf);
        let mut ids = self.ids.lock().expect("not poisoned");
        ids.by_oid.insert(oid, id);
        ids.by_blob.insert(id, oid);
        Ok(id)
    }

    /// Size of `blob` if git has it, from the object header.
    pub fn odb_size(&self, repo: &gix::Repository, blob: BlobId) -> Result<Option<u64>, GitError> {
        let oid = self.ids.lock().expect("not poisoned").by_blob.get(&blob).copied();
        let Some(oid) = oid else { return Ok(None) };
        Ok(repo.objects.try_header(&oid).map_err(err)?.map(|h| h.size))
    }

    /// Content of `blob` if git has it and it is within `cap`.
    pub fn odb_content(&self, repo: &gix::Repository, blob: BlobId, cap: u64) -> Result<Option<Vec<u8>>, GitError> {
        let oid = self.ids.lock().expect("not poisoned").by_blob.get(&blob).copied();
        let Some(oid) = oid else { return Ok(None) };
        let mut buf = Vec::new();
        let found = repo.objects.try_find(&oid, &mut buf).map_err(err)?.is_some();
        Ok((found && buf.len() as u64 <= cap).then_some(buf))
    }
}

#[cfg(test)]
mod tests;
