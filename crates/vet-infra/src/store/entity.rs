//! Tables as SeaORM entities. Hashes and paths are blobs; enums are integers
//! mapped in `codec`. Children of a changeset cascade on its deletion, and
//! those of a review (marks) on its deletion.

/// Enumerations stored as integers.
pub mod kinds {
    use sea_orm::entity::prelude::*;

    macro_rules! int_enum {
        ($name:ident { $($variant:ident = $n:literal),+ }) => {
            #[derive(Clone, Copy, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum)]
            #[sea_orm(rs_type = "i32", db_type = "Integer")]
            pub enum $name {
                $(#[sea_orm(num_value = $n)] $variant),+
            }
        };
    }

    int_enum!(Side { Base = 0, Target = 1 });
    int_enum!(EntryKind { File = 0, Symlink = 1, Submodule = 2 });
    int_enum!(SpanState { Seen = 0, SeenAgainstOldBase = 1 });
    int_enum!(ThreadKind { Range = 0, File = 1, Changeset = 2 });
    int_enum!(ThreadStatus { Open = 0, Resolved = 1 });
    int_enum!(FilterKind { Root = 0, Include = 1, Exclude = 2 });
    int_enum!(BaseKind { MergeBase = 0, Commit = 1 });
    int_enum!(ChildKind { File = 0, Dir = 1 });
    int_enum!(PathFlag { Collision = 0, ContentUnavailable = 1 });
    int_enum!(ChangeKind { Mode = 0, Symlink = 1, Submodule = 2, Binary = 3, ContentUnavailable = 4, Rename = 5, Empty = 6 });
    int_enum!(SourceKind { WorkingTree = 0, Branch = 1, PullRequest = 2 });
    int_enum!(VersionOrigin { Settled = 0, BaseMoved = 1, Observed = 2 });
    int_enum!(Lens { Squash = 0, PerCommit = 1 });
}

macro_rules! child_of_changeset {
    () => {
        #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
        pub enum Relation {
            #[sea_orm(
                belongs_to = "super::changeset::Entity",
                from = "Column::Changeset",
                to = "super::changeset::Column::Id",
                on_delete = "Cascade"
            )]
            Changeset,
        }

        impl ActiveModelBehavior for ActiveModel {}
    };
}

macro_rules! child_of_review {
    () => {
        #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
        pub enum Relation {
            #[sea_orm(
                belongs_to = "super::review::Entity",
                from = "Column::Review",
                to = "super::review::Column::Id",
                on_delete = "Cascade"
            )]
            Review,
        }

        impl ActiveModelBehavior for ActiveModel {}
    };
}

pub mod snapshot {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "snapshot")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Vec<u8>,
        /// Unix seconds.
        pub created: i64,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

/// Facts about a snapshot that are not part of its identity (`SnapshotStatus`).
pub mod snapshot_path {
    use super::kinds::*;
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "snapshot_path")]
    pub struct Model {
        #[sea_orm(primary_key)]
        pub id: i64,
        pub snapshot: Vec<u8>,
        pub flag: PathFlag,
        pub path: Vec<u8>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {
        #[sea_orm(
            belongs_to = "super::snapshot::Entity",
            from = "Column::Snapshot",
            to = "super::snapshot::Column::Id",
            on_delete = "Cascade"
        )]
        Snapshot,
    }

    impl ActiveModelBehavior for ActiveModel {}
}

/// A Merkle directory node, keyed by its hash and shared between snapshots. A
/// snapshot's id is its root node's hash.
pub mod tree_node {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "tree_node")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Vec<u8>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

/// One entry of a node. A `Dir` row sets `dir`; a `File` row sets the entry fields.
pub mod tree_child {
    use super::kinds::*;
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "tree_child")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub node: Vec<u8>,
        #[sea_orm(primary_key, auto_increment = false)]
        pub name: Vec<u8>,
        pub kind: ChildKind,
        pub dir: Option<Vec<u8>>,
        pub blob: Option<Vec<u8>>,
        pub mode: Option<i64>,
        pub entry_kind: Option<EntryKind>,
        pub unfiltered: Option<bool>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {
        #[sea_orm(
            belongs_to = "super::tree_node::Entity",
            from = "Column::Node",
            to = "super::tree_node::Column::Id",
            on_delete = "Cascade"
        )]
        Node,
    }

    impl ActiveModelBehavior for ActiveModel {}
}

/// What is reviewed, keyed by the repository root and its source. `key` is the
/// branch name, or `host/path#number` of a pull request, and empty for the
/// working tree.
pub mod changeset {
    use super::kinds::*;
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "changeset")]
    pub struct Model {
        #[sea_orm(primary_key)]
        pub id: i64,
        #[sea_orm(unique_key = "source")]
        pub repo: String,
        #[sea_orm(unique_key = "source")]
        pub kind: SourceKind,
        #[sea_orm(unique_key = "source")]
        pub key: String,
        /// The lens the viewer last chose; none until they did.
        pub lens: Option<Lens>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

/// A state of a changeset; the numbers of a changeset's versions are unique.
pub mod version {
    use super::kinds::*;
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "version")]
    pub struct Model {
        #[sea_orm(primary_key)]
        pub id: i64,
        #[sea_orm(unique_key = "number")]
        pub changeset: i64,
        #[sea_orm(unique_key = "number")]
        pub n: i64,
        pub base: Vec<u8>,
        pub base_commit: String,
        pub target: Vec<u8>,
        pub tip: String,
        /// Unix seconds.
        pub at: i64,
        pub origin: VersionOrigin,
    }

    child_of_changeset!();
}

/// A file a version changed against the one before it, with the lines it gained and lost.
pub mod version_file {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "version_file")]
    pub struct Model {
        #[sea_orm(primary_key)]
        pub id: i64,
        pub version: i64,
        pub path: Vec<u8>,
        pub added: i64,
        pub removed: i64,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {
        #[sea_orm(
            belongs_to = "super::version::Entity",
            from = "Column::Version",
            to = "super::version::Column::Id",
            on_delete = "Cascade"
        )]
        Version,
    }

    impl ActiveModelBehavior for ActiveModel {}
}

/// How the review's base was chosen. `base_value` is the branch, the reference
/// or the commit id.
pub mod review {
    use super::kinds::*;
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "review")]
    pub struct Model {
        #[sea_orm(primary_key)]
        pub id: i64,
        pub changeset: i64,
        pub base_kind: BaseKind,
        pub base_value: String,
        /// The number of the version something was last marked in.
        pub last_reviewed: Option<i64>,
        /// Unix seconds.
        pub opened_at: i64,
        pub closed_at: Option<i64>,
        /// Whether the review was complete when it was finished; false while open.
        pub complete: bool,
        /// How far the review was when it was last measured: all three, or none.
        pub summary_total: Option<i64>,
        pub summary_seen: Option<i64>,
        pub summary_old_base: Option<i64>,
    }

    child_of_changeset!();
}

/// A repository's review filter, keyed by the repository root like changesets are.
/// No row means the default filter.
pub mod project_filter {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "project_filter")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub repo: String,
        pub hide_binary: bool,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

/// The base last chosen for a repository's working tree, keyed by the repository root. No row
/// means it was never chosen.
pub mod project_base {
    use super::kinds::*;
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "project_base")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub repo: String,
        pub base_kind: BaseKind,
        pub base_value: String,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

/// `kind`: 0 root, 1 include, 2 exclude. Order is the row id.
pub mod project_filter_item {
    use super::kinds::*;
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "project_filter_item")]
    pub struct Model {
        #[sea_orm(primary_key)]
        pub id: i64,
        pub repo: String,
        pub kind: FilterKind,
        pub value: Vec<u8>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {
        #[sea_orm(
            belongs_to = "super::project_filter::Entity",
            from = "Column::Repo",
            to = "super::project_filter::Column::Repo",
            on_delete = "Cascade"
        )]
        ProjectFilter,
    }

    impl ActiveModelBehavior for ActiveModel {}
}

/// Owned by a span or a thread; deleted with its owner.
pub mod anchor {
    use super::kinds::*;
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "anchor")]
    pub struct Model {
        #[sea_orm(primary_key)]
        pub id: i64,
        pub changeset: i64,
        pub side: Side,
        pub snapshot: Vec<u8>,
        pub file: Vec<u8>,
        pub start: i64,
        pub end: i64,
        pub raw: Vec<u8>,
        pub norm: Vec<u8>,
        /// Concatenated 32-byte hashes.
        pub context: Vec<u8>,
    }

    child_of_changeset!();
}

pub mod span {
    use super::kinds::*;
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "span")]
    pub struct Model {
        #[sea_orm(primary_key)]
        pub id: i64,
        pub review: i64,
        pub state: SpanState,
        pub base_anchor: Option<i64>,
        pub target_anchor: Option<i64>,
        /// The commit whose own diff the span was marked in; none for the changeset as a whole.
        pub origin_commit: Option<String>,
    }

    child_of_review!();
}

/// A non-text acknowledgement: before and after entries, each absent for an
/// added or deleted path.
pub mod ack {
    use super::kinds::*;
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "ack")]
    pub struct Model {
        #[sea_orm(primary_key)]
        pub id: i64,
        pub review: i64,
        pub change: ChangeKind,
        pub before_path: Option<Vec<u8>>,
        pub before_blob: Option<Vec<u8>>,
        pub before_mode: Option<i64>,
        pub before_kind: Option<EntryKind>,
        pub before_unfiltered: Option<bool>,
        pub after_path: Option<Vec<u8>>,
        pub after_blob: Option<Vec<u8>>,
        pub after_mode: Option<i64>,
        pub after_kind: Option<EntryKind>,
        pub after_unfiltered: Option<bool>,
    }

    child_of_review!();
}

/// `kind`: 0 range (uses `anchor`), 1 file (`file_side`, `file_path`), 2 changeset.
/// `review` is the review a local thread was opened in; it only says where the
/// thread came from. The platform columns are set for a thread of a pull request.
pub mod thread {
    use super::kinds::*;
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "thread")]
    pub struct Model {
        #[sea_orm(primary_key)]
        pub id: i64,
        pub changeset: i64,
        pub review: Option<i64>,
        pub kind: ThreadKind,
        pub file_side: Option<Side>,
        pub file_path: Option<Vec<u8>>,
        pub anchor: Option<i64>,
        pub status: ThreadStatus,
        /// The thread is on a platform: the changeset is a pull request's. Without a `remote_id` it
        /// is a draft.
        pub on_platform: bool,
        pub remote_id: Option<String>,
        pub resolvable: bool,
        pub resolved_by: Option<String>,
        pub status_pending: bool,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {
        #[sea_orm(
            belongs_to = "super::changeset::Entity",
            from = "Column::Changeset",
            to = "super::changeset::Column::Id",
            on_delete = "Cascade"
        )]
        Changeset,
        #[sea_orm(
            belongs_to = "super::review::Entity",
            from = "Column::Review",
            to = "super::review::Column::Id",
            on_delete = "SetNull"
        )]
        Review,
    }

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod message {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "message")]
    pub struct Model {
        #[sea_orm(primary_key)]
        pub id: i64,
        pub thread: i64,
        pub author: String,
        pub body: String,
        pub at: i64,
        pub mine: bool,
        pub avatar: Option<String>,
        pub edited_at: Option<i64>,
        /// The platform's id of the comment; none for a draft or a message outside a platform.
        pub remote_id: Option<String>,
        /// The text the platform had at the last sync.
        pub synced: Option<String>,
        pub conflict_body: Option<String>,
        pub conflict_at: Option<i64>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {
        #[sea_orm(
            belongs_to = "super::thread::Entity",
            from = "Column::Thread",
            to = "super::thread::Column::Id",
            on_delete = "Cascade"
        )]
        Thread,
    }

    impl ActiveModelBehavior for ActiveModel {}
}

/// The custom language server path, unique per language.
pub mod server_path {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "server_path")]
    pub struct Model {
        #[sea_orm(primary_key)]
        pub id: i64,
        #[sea_orm(unique)]
        pub language: String,
        pub path: String,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

/// A recently opened repository. The newest has the highest id.
pub mod recent_project {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "recent_project")]
    pub struct Model {
        #[sea_orm(primary_key)]
        pub id: i64,
        #[sea_orm(unique)]
        pub path: String,
        /// Unix seconds.
        pub opened_at: i64,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}
