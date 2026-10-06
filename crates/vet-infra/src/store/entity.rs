//! Tables as SeaORM entities. Hashes and paths are blobs; enums are integers
//! mapped in `codec`. Children of a session cascade on its deletion.

macro_rules! child_of_session {
    () => {
        #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
        pub enum Relation {
            #[sea_orm(belongs_to = "super::session::Entity", from = "Column::Session", to = "super::session::Column::Id", on_delete = "Cascade")]
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

pub mod snapshot_entry {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "snapshot_entry")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub snapshot: Vec<u8>,
        #[sea_orm(primary_key, auto_increment = false)]
        pub path: Vec<u8>,
        pub blob: Vec<u8>,
        pub mode: i64,
        pub kind: i64,
        pub unfiltered: bool,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {
        #[sea_orm(belongs_to = "super::snapshot::Entity", from = "Column::Snapshot", to = "super::snapshot::Column::Id", on_delete = "Cascade")]
        Snapshot,
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
        #[sea_orm(belongs_to = "super::pin::Entity", from = "Column::Pin", to = "super::pin::Column::Id")]
        Pin,
    }

    impl ActiveModelBehavior for ActiveModel {}
}

/// `kind`: 0 root, 1 include, 2 exclude. Order is the row id.
pub mod filter_item {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "filter_item")]
    pub struct Model {
        #[sea_orm(primary_key)]
        pub id: i64,
        pub session: i64,
        pub kind: i64,
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
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "anchor")]
    pub struct Model {
        #[sea_orm(primary_key)]
        pub id: i64,
        pub session: i64,
        pub side: i64,
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
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "span")]
    pub struct Model {
        #[sea_orm(primary_key)]
        pub id: i64,
        pub session: i64,
        pub state: i64,
        pub base_anchor: Option<i64>,
        pub target_anchor: Option<i64>,
    }

    child_of_session!();
}

/// A non-text acknowledgement: before and after entries, each absent for an
/// added or deleted path.
pub mod ack {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "ack")]
    pub struct Model {
        #[sea_orm(primary_key)]
        pub id: i64,
        pub session: i64,
        pub before_path: Option<Vec<u8>>,
        pub before_blob: Option<Vec<u8>>,
        pub before_mode: Option<i64>,
        pub before_kind: Option<i64>,
        pub before_unfiltered: Option<bool>,
        pub after_path: Option<Vec<u8>>,
        pub after_blob: Option<Vec<u8>>,
        pub after_mode: Option<i64>,
        pub after_kind: Option<i64>,
        pub after_unfiltered: Option<bool>,
    }

    child_of_session!();
}

/// `kind`: 0 range (uses `anchor`), 1 file (`file_side`, `file_path`), 2 changeset.
pub mod thread {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "thread")]
    pub struct Model {
        #[sea_orm(primary_key)]
        pub id: i64,
        pub session: i64,
        pub kind: i64,
        pub file_side: Option<i64>,
        pub file_path: Option<Vec<u8>>,
        pub anchor: Option<i64>,
        pub status: i64,
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
        #[sea_orm(belongs_to = "super::thread::Entity", from = "Column::Thread", to = "super::thread::Column::Id", on_delete = "Cascade")]
        Thread,
    }

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod run_approval {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "run_approval")]
    pub struct Model {
        #[sea_orm(primary_key)]
        pub id: i64,
        #[sea_orm(indexed)]
        pub repo: String,
        pub hash: Vec<u8>,
        pub at: i64,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}
