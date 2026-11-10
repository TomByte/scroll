use rusqlite::types::{FromSql, FromSqlResult, ToSql, ToSqlOutput, ValueRef};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

macro_rules! record_id {
    ($name:ident) => {
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
        )]
        pub struct $name(pub Uuid);

        impl $name {
            /// Time-ordered (UUIDv7), so new records sort after old ones in indexes.
            pub fn generate() -> Self {
                Self(Uuid::now_v7())
            }
        }

        impl ToSql for $name {
            fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
                self.0.to_sql()
            }
        }

        impl FromSql for $name {
            fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
                Uuid::column_result(value).map(Self)
            }
        }
    };
}

record_id!(WorkspaceId);
record_id!(PageId);
record_id!(UserId);
record_id!(DatabaseId);
record_id!(PropertyId);
record_id!(SelectOptionId);
record_id!(ViewId);
record_id!(TransactionId);
record_id!(TagId);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Role {
    Owner,
    Editor,
    Viewer,
}

impl Role {
    /// Whether the member may change the workspace's records and pages. A viewer only reads.
    pub fn can_edit(self) -> bool {
        match self {
            Role::Owner | Role::Editor => true,
            Role::Viewer => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owners_and_editors_edit_and_viewers_only_read() {
        assert!(Role::Owner.can_edit());
        assert!(Role::Editor.can_edit());
        assert!(!Role::Viewer.can_edit());
    }
}
