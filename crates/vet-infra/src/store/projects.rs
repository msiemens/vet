//! `vet_app::ports::ProjectStore`: what is held for a repository root, what it takes, and how it
//! goes. A root's changesets, filter, trust, preferences and recent entry are all keyed by the
//! root as recorded.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use sea_orm::{
    ColumnTrait, ConnectionTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder, QuerySelect, QueryTrait,
    TransactionTrait,
};
use vet_app::changeset::ChangesetId;
use vet_app::ports::{ProjectFacts, ProjectStore, StoreError};
use vet_core::content::SnapshotId;

use super::codec::pull_request_remote;
use super::entity::*;
use super::gc::{CHUNK, dir_children, snapshot_id};
use super::{Res, SqliteStore};

/// A version as `project_facts` reads it: whether it has snapshots, base, target.
type VersionRow = (bool, Vec<u8>, Vec<u8>);

/// The distinct values of one text column.
async fn distinct_texts<E: EntityTrait>(db: &impl ConnectionTrait, column: E::Column) -> Res<Vec<String>> {
    E::find()
        .select_only()
        .column(column)
        .distinct()
        .into_tuple()
        .all(db)
        .await
}

/// The ids of the changesets of `repo`, as a subquery.
fn changesets_of(repo: &str) -> sea_orm::sea_query::SelectStatement {
    changeset::Entity::find()
        .filter(changeset::Column::Repo.eq(repo))
        .select_only()
        .column(changeset::Column::Id)
        .into_query()
}

/// Labels the tree nodes each snapshot reaches with the owners of the snapshot; a node that
/// snapshots of several owners reach has all of them.
fn label_nodes(
    owners: HashMap<Vec<u8>, BTreeSet<usize>>,
    children: &HashMap<Vec<u8>, Vec<Vec<u8>>>,
) -> HashMap<Vec<u8>, BTreeSet<usize>> {
    let mut labels: HashMap<Vec<u8>, BTreeSet<usize>> = HashMap::new();
    for (root, owners) in owners {
        let mut stack = vec![root];
        while let Some(node) = stack.pop() {
            let held = labels.entry(node.clone()).or_default();
            let before = held.len();
            held.extend(owners.iter().copied());
            if held.len() == before {
                continue;
            }
            stack.extend(children.get(&node).into_iter().flatten().cloned());
        }
    }
    labels
}

impl ProjectStore for SqliteStore {
    async fn known_projects(&self) -> Result<Vec<String>, StoreError> {
        let db = self.read().await?;
        let mut roots: BTreeSet<String> = BTreeSet::new();
        roots.extend(distinct_texts::<changeset::Entity>(&db, changeset::Column::Repo).await?);
        roots.extend(distinct_texts::<project_trust::Entity>(&db, project_trust::Column::Repo).await?);
        roots.extend(distinct_texts::<project_filter::Entity>(&db, project_filter::Column::Repo).await?);
        roots.extend(distinct_texts::<recent_project::Entity>(&db, recent_project::Column::Path).await?);
        roots.extend(
            preference::Entity::find()
                .filter(preference::Column::Scope.ne(""))
                .select_only()
                .column(preference::Column::Scope)
                .distinct()
                .into_tuple::<String>()
                .all(&db)
                .await?,
        );
        Ok(roots.into_iter().collect())
    }

    async fn project_facts(&self, repo: String) -> Result<ProjectFacts, StoreError> {
        let db = self.read().await?;
        let ids: Vec<i64> = changeset::Entity::find()
            .filter(changeset::Column::Repo.eq(repo.as_str()))
            .order_by_asc(changeset::Column::Id)
            .select_only()
            .column(changeset::Column::Id)
            .into_tuple()
            .all(&db)
            .await?;
        let versions: Vec<VersionRow> = version::Entity::find()
            .filter(version::Column::Changeset.in_subquery(changesets_of(&repo)))
            .select_only()
            .columns([
                version::Column::Available,
                version::Column::Base,
                version::Column::Target,
            ])
            .into_tuple()
            .all(&db)
            .await?;
        let anchored: Vec<Vec<u8>> = anchor::Entity::find()
            .filter(anchor::Column::Changeset.in_subquery(changesets_of(&repo)))
            .select_only()
            .column(anchor::Column::Snapshot)
            .distinct()
            .into_tuple()
            .all(&db)
            .await?;
        let open: Vec<i64> = review::Entity::find()
            .filter(review::Column::Changeset.in_subquery(changesets_of(&repo)))
            .filter(review::Column::ClosedAt.is_null())
            .select_only()
            .column(review::Column::Changeset)
            .into_tuple()
            .all(&db)
            .await?;

        let mut snapshots = BTreeSet::new();
        for (available, base, target) in &versions {
            if *available {
                snapshots.insert(snapshot_id(base)?);
                snapshots.insert(snapshot_id(target)?);
            }
        }
        for snapshot in &anchored {
            snapshots.insert(snapshot_id(snapshot)?);
        }

        let threads = thread::Entity::find()
            .filter(thread::Column::Changeset.in_subquery(changesets_of(&repo)))
            .select_only()
            .column(thread::Column::Id)
            .into_query();
        let unposted = message::Entity::find()
            .filter(message::Column::Thread.in_subquery(threads))
            .filter(message::Column::RemoteId.is_null())
            .count(&db)
            .await?;
        let voting = changeset::Entity::find()
            .filter(changeset::Column::Repo.eq(repo.as_str()))
            .filter(changeset::Column::Vote.is_not_null())
            .count(&db)
            .await?;
        Ok(ProjectFacts {
            changesets: ids.into_iter().map(|id| ChangesetId(id as u64)).collect(),
            snapshots: snapshots.into_iter().collect(),
            open_reviews: u32::try_from(open.len()).unwrap_or(u32::MAX),
            unposted_comments: u32::try_from(unposted).unwrap_or(u32::MAX),
            pending_votes: u32::try_from(voting).unwrap_or(u32::MAX),
            trusted: project_trust::Entity::find_by_id(repo).one(&db).await?.is_some(),
        })
    }

    async fn review_data(&self) -> Result<BTreeMap<String, u64>, StoreError> {
        let db = self.read().await?;
        let repos: Vec<(i64, String)> = changeset::Entity::find()
            .select_only()
            .columns([changeset::Column::Id, changeset::Column::Repo])
            .into_tuple()
            .all(&db)
            .await?;
        let roots: Vec<&String> = repos
            .iter()
            .map(|(_, repo)| repo)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let index: HashMap<&String, usize> = roots.iter().enumerate().map(|(at, root)| (*root, at)).collect();
        let owner_of: HashMap<i64, usize> = repos.iter().map(|(id, repo)| (*id, index[repo])).collect();

        let mut owners: HashMap<Vec<u8>, BTreeSet<usize>> = HashMap::new();
        let versions: Vec<(i64, Vec<u8>, Vec<u8>)> = version::Entity::find()
            .filter(version::Column::Available.eq(true))
            .select_only()
            .columns([
                version::Column::Changeset,
                version::Column::Base,
                version::Column::Target,
            ])
            .into_tuple()
            .all(&db)
            .await?;
        for (changeset, base, target) in versions {
            if let Some(&owner) = owner_of.get(&changeset) {
                owners.entry(base).or_default().insert(owner);
                owners.entry(target).or_default().insert(owner);
            }
        }
        let anchors: Vec<(i64, Vec<u8>)> = anchor::Entity::find()
            .select_only()
            .columns([anchor::Column::Changeset, anchor::Column::Snapshot])
            .into_tuple()
            .all(&db)
            .await?;
        for (changeset, snapshot) in anchors {
            if let Some(&owner) = owner_of.get(&changeset) {
                owners.entry(snapshot).or_default().insert(owner);
            }
        }

        let labels = label_nodes(owners, &dir_children(&db).await?);
        let labelled: Vec<&Vec<u8>> = labels.keys().collect();
        let mut blob_owners: HashMap<Vec<u8>, BTreeSet<usize>> = HashMap::new();
        for chunk in labelled.chunks(CHUNK) {
            let files: Vec<(Vec<u8>, Vec<u8>)> = tree_child::Entity::find()
                .filter(tree_child::Column::Node.is_in(chunk.iter().map(|n| (*n).clone())))
                .filter(tree_child::Column::Blob.is_not_null())
                .select_only()
                .columns([tree_child::Column::Node, tree_child::Column::Blob])
                .into_tuple()
                .all(&db)
                .await?;
            for (node, blob) in files {
                if let Some(owners) = labels.get(&node) {
                    blob_owners.entry(blob).or_default().extend(owners.iter().copied());
                }
            }
        }
        let mut alone: Vec<Vec<Vec<u8>>> = vec![Vec::new(); roots.len()];
        for (blob, owners) in blob_owners {
            if let (Some(&only), 1) = (owners.first(), owners.len()) {
                alone[only].push(blob);
            }
        }
        let mut sizes = BTreeMap::new();
        for (root, blobs) in roots.into_iter().zip(alone) {
            sizes.insert(root.clone(), self.content.stored_bytes(&blobs).await?);
        }
        Ok(sizes)
    }

    async fn remove_project(&self, repo: String) -> Result<(), StoreError> {
        let txn = self.writer.db.begin().await?;
        let gone = changeset::Entity::find()
            .filter(changeset::Column::Repo.eq(repo.as_str()))
            .all(&txn)
            .await?;
        let remotes_gone: HashSet<(String, String)> = gone
            .iter()
            .filter(|c| c.kind == kinds::SourceKind::PullRequest)
            .filter_map(|c| pull_request_remote(&c.key))
            .collect();
        changeset::Entity::delete_many()
            .filter(changeset::Column::Repo.eq(repo.as_str()))
            .exec(&txn)
            .await?;
        project_trust::Entity::delete_by_id(repo.as_str()).exec(&txn).await?;
        project_filter::Entity::delete_by_id(repo.as_str()).exec(&txn).await?;
        preference::Entity::delete_many()
            .filter(preference::Column::Scope.eq(repo.as_str()))
            .exec(&txn)
            .await?;
        recent_project::Entity::delete_many()
            .filter(recent_project::Column::Path.eq(repo.as_str()))
            .exec(&txn)
            .await?;

        let remotes_kept: HashSet<(String, String)> = changeset::Entity::find()
            .filter(changeset::Column::Kind.eq(kinds::SourceKind::PullRequest))
            .all(&txn)
            .await?
            .iter()
            .filter_map(|c| pull_request_remote(&c.key))
            .collect();
        for (host, path) in remotes_gone.difference(&remotes_kept) {
            pull_request::Entity::delete_many()
                .filter(pull_request::Column::Host.eq(host.as_str()))
                .filter(pull_request::Column::Path.eq(path.as_str()))
                .exec(&txn)
                .await?;
        }
        Ok(txn.commit().await?)
    }

    async fn named_snapshots(&self, snapshots: Vec<SnapshotId>) -> Result<Vec<SnapshotId>, StoreError> {
        let db = self.read().await?;
        let mut named: HashSet<Vec<u8>> = HashSet::new();
        for chunk in snapshots.chunks(CHUNK) {
            let keys: Vec<Vec<u8>> = chunk.iter().map(|s| s.0.0.to_vec()).collect();
            for column in [version::Column::Base, version::Column::Target] {
                named.extend(
                    version::Entity::find()
                        .filter(column.is_in(keys.clone()))
                        .select_only()
                        .column(column)
                        .distinct()
                        .into_tuple::<Vec<u8>>()
                        .all(&db)
                        .await?,
                );
            }
            named.extend(
                anchor::Entity::find()
                    .filter(anchor::Column::Snapshot.is_in(keys))
                    .select_only()
                    .column(anchor::Column::Snapshot)
                    .distinct()
                    .into_tuple::<Vec<u8>>()
                    .all(&db)
                    .await?,
            );
        }
        Ok(snapshots
            .into_iter()
            .filter(|s| named.contains(s.0.0.as_slice()))
            .collect())
    }
}
