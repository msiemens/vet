//! `vet_app::ports::ReviewWrites` on the one write connection.

use sea_orm::{ActiveModelTrait, ColumnTrait, DbErr, EntityTrait, QueryFilter, Set, TransactionTrait};
use vet_app::changeset::{BaseChoice, ChangesetId, CommitId, Lens, Source, Version};
use vet_app::platform::PullRequestHead;
use vet_app::ports::{ReviewWrites, StoreError};
use vet_app::review::{NonTextAck, ReviewId, ReviewSummary, SpanId, SpanOrigin};
use vet_core::content::{Snapshot, SnapshotStatus};
use vet_core::filter::ReviewFilter;
use vet_core::merkle;
use vet_core::review::{
    CommentTarget, Message, MessageId, PendingVote, PlatformThread, ReviewedSpan, ThreadId, ThreadStatus,
};

use super::codec::*;
use super::entity::*;
use super::{
    ReviewWriter, SqliteStore, find_changeset, insert_changeset, insert_chunked, insert_version, put_anchor,
    write_filter,
};
use std::collections::HashSet;

impl ReviewWrites for ReviewWriter {
    async fn put_snapshot(&self, snapshot: Snapshot, status: SnapshotStatus, at: i64) -> Result<(), StoreError> {
        let (root, nodes) = merkle::build(&snapshot.entries);
        if root != snapshot.id {
            return Err(StoreError("snapshot id does not match its entries".into()));
        }
        let txn = self.db.begin().await?;
        let key = root.0.0.to_vec();
        if snapshot::Entity::find_by_id(key.clone()).one(&txn).await?.is_none() {
            snapshot::ActiveModel {
                id: Set(key.clone()),
                created: Set(at),
            }
            .insert(&txn)
            .await?;
            insert_chunked(&txn, status_rows(&key, &status)).await?;

            let hashes: Vec<Vec<u8>> = nodes.keys().map(|h| h.0.to_vec()).collect();
            let mut present = HashSet::new();
            for chunk in hashes.chunks(1000) {
                present.extend(
                    tree_node::Entity::find()
                        .filter(tree_node::Column::Id.is_in(chunk.to_vec()))
                        .all(&txn)
                        .await?
                        .into_iter()
                        .map(|n| n.id),
                );
            }
            let missing: Vec<_> = nodes
                .iter()
                .filter(|(h, _)| !present.contains(h.0.as_slice()))
                .collect();
            insert_chunked(
                &txn,
                missing
                    .iter()
                    .map(|(h, _)| tree_node::ActiveModel { id: Set(h.0.to_vec()) })
                    .collect(),
            )
            .await?;
            insert_chunked(&txn, missing.iter().flat_map(|(h, n)| child_rows(**h, n)).collect()).await?;
        }
        Ok(txn.commit().await?)
    }

    async fn set_lens(&self, repo: String, source: Source, lens: Lens) -> Result<(), StoreError> {
        let txn = self.db.begin().await?;
        match find_changeset(&txn, repo.clone(), &source).await? {
            Some(id) => {
                changeset::ActiveModel {
                    id: Set(id),
                    lens: Set(Some(lens.into())),
                    ..Default::default()
                }
                .update(&txn)
                .await?;
            }
            None => {
                let (kind, key) = source_columns(&source);
                changeset::ActiveModel {
                    repo: Set(repo),
                    kind: Set(kind),
                    key: Set(key),
                    lens: Set(Some(lens.into())),
                    ..Default::default()
                }
                .insert(&txn)
                .await?;
            }
        }
        Ok(txn.commit().await?)
    }

    async fn set_pull_request_head(
        &self,
        repo: String,
        source: Source,
        head: PullRequestHead,
    ) -> Result<(), StoreError> {
        let txn = self.db.begin().await?;
        let id = match find_changeset(&txn, repo.clone(), &source).await? {
            Some(id) => id,
            None => insert_changeset(&txn, repo, &source).await?,
        };
        changeset::ActiveModel {
            id: Set(id),
            title: Set(Some(head.title)),
            draft: Set(head.draft),
            fork: Set(head.fork),
            target_branch: Set(Some(head.target_branch)),
            ..Default::default()
        }
        .update(&txn)
        .await?;
        Ok(txn.commit().await?)
    }

    async fn set_pull_request_trusted_head(
        &self,
        repo: String,
        source: Source,
        head: Option<CommitId>,
    ) -> Result<(), StoreError> {
        let txn = self.db.begin().await?;
        let id = match find_changeset(&txn, repo.clone(), &source).await? {
            Some(id) => id,
            None => insert_changeset(&txn, repo, &source).await?,
        };
        changeset::ActiveModel {
            id: Set(id),
            trusted_head: Set(head.map(|h| h.0)),
            ..Default::default()
        }
        .update(&txn)
        .await?;
        Ok(txn.commit().await?)
    }

    async fn set_pending_vote(
        &self,
        repo: String,
        source: Source,
        vote: Option<PendingVote>,
    ) -> Result<(), StoreError> {
        let txn = self.db.begin().await?;
        let id = match find_changeset(&txn, repo.clone(), &source).await? {
            Some(id) => id,
            None => insert_changeset(&txn, repo, &source).await?,
        };
        let mut row = changeset::ActiveModel {
            id: Set(id),
            ..Default::default()
        };
        vote_columns(&mut row, vote.as_ref());
        row.update(&txn).await?;
        Ok(txn.commit().await?)
    }

    async fn ensure_changeset(&self, repo: String, source: Source) -> Result<ChangesetId, StoreError> {
        let txn = self.db.begin().await?;
        let id = match find_changeset(&txn, repo.clone(), &source).await? {
            Some(id) => id,
            None => insert_changeset(&txn, repo, &source).await?,
        };
        txn.commit().await?;
        Ok(ChangesetId(id as u64))
    }

    async fn append_version(&self, changeset: ChangesetId, version: Version) -> Result<(), StoreError> {
        let txn = self.db.begin().await?;
        insert_version(&txn, changeset, &version).await?;
        Ok(txn.commit().await?)
    }

    async fn start_review(
        &self,
        repo: String,
        source: Source,
        base: BaseChoice,
        first: Version,
        now: i64,
    ) -> Result<(ChangesetId, ReviewId), StoreError> {
        let txn = self.db.begin().await?;
        let changeset = match find_changeset(&txn, repo.clone(), &source).await? {
            Some(id) => id,
            None => insert_changeset(&txn, repo, &source).await?,
        };
        version::Entity::delete_many()
            .filter(version::Column::Changeset.eq(changeset))
            .exec(&txn)
            .await?;
        insert_version(&txn, ChangesetId(changeset as u64), &first).await?;
        let (base_kind, base_value) = base_columns(base);
        let review = review::ActiveModel {
            changeset: Set(changeset),
            base_kind: Set(base_kind),
            base_value: Set(base_value),
            opened_at: Set(now),
            complete: Set(false),
            ..Default::default()
        }
        .insert(&txn)
        .await?;
        txn.commit().await?;
        Ok((ChangesetId(changeset as u64), ReviewId(review.id as u64)))
    }

    async fn close_review(&self, id: ReviewId, now: i64, complete: bool) -> Result<(), StoreError> {
        review::ActiveModel {
            id: Set(id.0 as i64),
            closed_at: Set(Some(now)),
            complete: Set(complete),
            ..Default::default()
        }
        .update(&self.db)
        .await?;
        Ok(())
    }

    async fn set_filter(&self, repo: String, filter: ReviewFilter) -> Result<(), StoreError> {
        let txn = self.db.begin().await?;
        project_filter::Entity::delete_by_id(repo.clone()).exec(&txn).await?;
        write_filter(&txn, &repo, &filter).await?;
        Ok(txn.commit().await?)
    }

    async fn set_project_trusted(&self, repo: String, trusted: bool) -> Result<(), StoreError> {
        let txn = self.db.begin().await?;
        project_trust::Entity::delete_by_id(repo.clone()).exec(&txn).await?;
        if trusted {
            project_trust::ActiveModel { repo: Set(repo) }.insert(&txn).await?;
        }
        Ok(txn.commit().await?)
    }

    async fn set_last_reviewed(&self, id: ReviewId, version: u32) -> Result<(), StoreError> {
        review::ActiveModel {
            id: Set(id.0 as i64),
            last_reviewed: Set(Some(version.into())),
            ..Default::default()
        }
        .update(&self.db)
        .await?;
        Ok(())
    }

    async fn set_review_summary(&self, id: ReviewId, summary: ReviewSummary) -> Result<(), StoreError> {
        review::ActiveModel {
            id: Set(id.0 as i64),
            summary_total: Set(Some(summary.total.into())),
            summary_seen: Set(Some(summary.seen.into())),
            summary_old_base: Set(Some(summary.old_base.into())),
            ..Default::default()
        }
        .update(&self.db)
        .await?;
        Ok(())
    }

    async fn edit_spans(
        &self,
        id: ReviewId,
        remove: Vec<SpanId>,
        add: Vec<(ReviewedSpan, SpanOrigin)>,
    ) -> Result<Vec<SpanId>, StoreError> {
        let txn = self.db.begin().await?;
        let rid = id.0 as i64;
        let changeset = review::Entity::find_by_id(rid)
            .one(&txn)
            .await?
            .ok_or_else(|| DbErr::RecordNotFound(format!("review {}", id.0)))?
            .changeset;
        // Anchors are owned by their spans, so they go with them.
        let remove: Vec<i64> = remove.iter().map(|s| s.0 as i64).collect();
        let doomed = span::Entity::find()
            .filter(span::Column::Review.eq(rid))
            .filter(span::Column::Id.is_in(remove))
            .all(&txn)
            .await?;
        span::Entity::delete_many()
            .filter(span::Column::Id.is_in(doomed.iter().map(|s| s.id)))
            .exec(&txn)
            .await?;
        anchor::Entity::delete_many()
            .filter(anchor::Column::Id.is_in(doomed.iter().flat_map(|s| [s.base_anchor, s.target_anchor]).flatten()))
            .exec(&txn)
            .await?;
        let mut ids = Vec::new();
        for (s, origin) in &add {
            let row = span::ActiveModel {
                review: Set(rid),
                state: Set(s.state.into()),
                base_anchor: Set(put_anchor(&txn, changeset, s.base.as_ref()).await?),
                target_anchor: Set(put_anchor(&txn, changeset, s.target.as_ref()).await?),
                origin_commit: Set(match origin {
                    SpanOrigin::Squash => None,
                    SpanOrigin::Commit(c) => Some(c.0.clone()),
                }),
                ..Default::default()
            }
            .insert(&txn)
            .await?;
            ids.push(SpanId(row.id as u64));
        }
        txn.commit().await?;
        Ok(ids)
    }

    async fn acknowledge(&self, id: ReviewId, ack: NonTextAck) -> Result<(), StoreError> {
        let txn = self.db.begin().await?;
        let rid = id.0 as i64;
        let existing = ack::Entity::find()
            .filter(ack::Column::Review.eq(rid))
            .filter(ack_filter(&ack))
            .one(&txn)
            .await?;
        if existing.is_none() {
            ack_row(rid, &ack).insert(&txn).await?;
        }
        Ok(txn.commit().await?)
    }

    async fn unacknowledge(&self, id: ReviewId, ack: NonTextAck) -> Result<(), StoreError> {
        ack::Entity::delete_many()
            .filter(ack::Column::Review.eq(id.0 as i64))
            .filter(ack_filter(&ack))
            .exec(&self.db)
            .await?;
        Ok(())
    }

    async fn add_thread(
        &self,
        changeset: ChangesetId,
        review: Option<ReviewId>,
        target: CommentTarget,
        opener: Message,
        platform: Option<PlatformThread>,
        status: ThreadStatus,
    ) -> Result<(ThreadId, MessageId), StoreError> {
        let txn = self.db.begin().await?;
        let cid = changeset.0 as i64;
        let anchor = match &target {
            CommentTarget::Range(a) => put_anchor(&txn, cid, Some(a)).await?,
            _ => None,
        };
        let mut row = thread::ActiveModel {
            changeset: Set(cid),
            review: Set(review.map(|r| r.0 as i64)),
            status: Set(status.into()),
            ..Default::default()
        };
        set_target(&mut row, &target, anchor);
        set_platform(&mut row, platform.as_ref());
        let thread = row.insert(&txn).await?.id;
        let message = message_row(thread, &opener).insert(&txn).await?.id;
        txn.commit().await?;
        Ok((ThreadId(thread as u64), MessageId(message as u64)))
    }

    async fn add_message(&self, thread: ThreadId, m: Message) -> Result<MessageId, StoreError> {
        SqliteStore::find_thread(&self.db, thread).await?;
        let row = message_row(thread.0 as i64, &m).insert(&self.db).await?;
        Ok(MessageId(row.id as u64))
    }

    async fn replace_message(&self, m: Message) -> Result<(), StoreError> {
        let old = message::Entity::find_by_id(m.id.0 as i64)
            .one(&self.db)
            .await?
            .ok_or_else(|| DbErr::RecordNotFound(format!("message {}", m.id.0)))?;
        let mut row = message_row(old.thread, &m);
        row.id = Set(old.id);
        row.update(&self.db).await?;
        Ok(())
    }

    async fn delete_message(&self, message: MessageId) -> Result<(), StoreError> {
        message::Entity::delete_by_id(message.0 as i64).exec(&self.db).await?;
        Ok(())
    }

    async fn set_thread_status(&self, thread: ThreadId, status: ThreadStatus) -> Result<(), StoreError> {
        thread::ActiveModel {
            id: Set(thread.0 as i64),
            status: Set(status.into()),
            ..Default::default()
        }
        .update(&self.db)
        .await?;
        Ok(())
    }

    async fn set_thread_platform(&self, thread: ThreadId, platform: PlatformThread) -> Result<(), StoreError> {
        let mut row = thread::ActiveModel {
            id: Set(thread.0 as i64),
            ..Default::default()
        };
        set_platform(&mut row, Some(&platform));
        row.update(&self.db).await?;
        Ok(())
    }

    async fn set_thread_target(&self, thread: ThreadId, target: CommentTarget) -> Result<(), StoreError> {
        let txn = self.db.begin().await?;
        let old = SqliteStore::find_thread(&txn, thread).await?;
        if let Some(a) = old.anchor {
            anchor::Entity::delete_by_id(a).exec(&txn).await?;
        }
        let anchor = match &target {
            CommentTarget::Range(a) => put_anchor(&txn, old.changeset, Some(a)).await?,
            _ => None,
        };
        let mut row = thread::ActiveModel {
            id: Set(old.id),
            ..Default::default()
        };
        set_target(&mut row, &target, anchor);
        row.update(&txn).await?;
        Ok(txn.commit().await?)
    }

    async fn delete_thread(&self, thread: ThreadId) -> Result<(), StoreError> {
        let txn = self.db.begin().await?;
        let old = SqliteStore::find_thread(&txn, thread).await?;
        thread::Entity::delete_by_id(old.id).exec(&txn).await?;
        if let Some(a) = old.anchor {
            anchor::Entity::delete_by_id(a).exec(&txn).await?;
        }
        Ok(txn.commit().await?)
    }
}
