//! Tables as SeaORM entities. Hashes and paths are blobs; enums are integers
//! mapped in `codec`. Children of a session cascade on its deletion.

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
    int_enum!(ChildKind { File = 0, Dir = 1 });
    int_enum!(PathFlag { Collision = 0, ContentUnavailable = 1 });
    int_enum!(ChangeKind { Mode = 0, Symlink = 1, Submodule = 2, Binary = 3, ContentUnavailable = 4, Rename = 5 });
}

macro_rules! child_of_session {
    () => {
        #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
        pub enum Relation {
            #[sea_orm(
                belongs_to = "super::session::Entity",
                from = "Column::Session",
                to = "super::session::Column::Id",
                on_delete = "Cascade"
            )]
            Session,
        }

        impl ActiveModelBehavior for ActiveModel {}
    };
}

pub mod pin {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "pin")]
    pub struct Model {
        #[sea_orm(primary_key)]
        pub id: i64,
        pub head: String,
        pub tree: Vec<u8>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
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

pub mod session {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "session")]
    pub struct Model {
        #[sea_orm(primary_key)]
        pub id: i64,
        pub pin: i64,
        pub base: Vec<u8>,
        pub target: Vec<u8>,
        pub hide_binary: bool,
        pub closed_at: Option<i64>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {
        #[sea_orm(
            belongs_to = "super::pin::Entity",
            from = "Column::Pin",
            to = "super::pin::Column::Id"
        )]
        Pin,
    }

    impl ActiveModelBehavior for ActiveModel {}
}

/// `kind`: 0 root, 1 include, 2 exclude. Order is the row id.
pub mod filter_item {
    use super::kinds::*;
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "filter_item")]
    pub struct Model {
        #[sea_orm(primary_key)]
        pub id: i64,
        pub session: i64,
        pub kind: FilterKind,
        pub value: Vec<u8>,
    }

    child_of_session!();
}

/// Settled snapshots of a session's tip; order is the row id.
pub mod timeline {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "timeline")]
    pub struct Model {
        #[sea_orm(primary_key)]
        pub id: i64,
        pub session: i64,
        pub snapshot: Vec<u8>,
    }

    child_of_session!();
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
        pub session: i64,
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

    child_of_session!();
}

pub mod span {
    use super::kinds::*;
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "span")]
    pub struct Model {
        #[sea_orm(primary_key)]
        pub id: i64,
        pub session: i64,
        pub state: SpanState,
        pub base_anchor: Option<i64>,
        pub target_anchor: Option<i64>,
    }

    child_of_session!();
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
        pub session: i64,
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

    child_of_session!();
}

/// `kind`: 0 range (uses `anchor`), 1 file (`file_side`, `file_path`), 2 changeset.
pub mod thread {
    use super::kinds::*;
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "thread")]
    pub struct Model {
        #[sea_orm(primary_key)]
        pub id: i64,
        pub session: i64,
        pub kind: ThreadKind,
        pub file_side: Option<Side>,
        pub file_path: Option<Vec<u8>>,
        pub anchor: Option<i64>,
        pub status: ThreadStatus,
        pub remote_id: Option<String>,
        pub remote_status: Option<String>,
        pub remote_resolvable: Option<bool>,
        pub remote_blocks_merge: Option<bool>,
    }

    child_of_session!();
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
