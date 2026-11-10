//! Tags and which pages have them.

use scroll_core::id::{PageId, TagId};
use scroll_core::operation::{Record, RecordId, TagField};
use scroll_core::record::{PageTag, Tag};
use scroll_core::validation::{
    ValidationError, check_tag_name, check_tag_name_is_free, tag_name_key,
};

use super::WorkspaceRecords;
use crate::lock;
use crate::records::catch_up::{ID_PAIR_BYTES, Item, SnapshotSource, stored_record};
use crate::records::{RecordError, decode, encode};

pub(super) const SNAPSHOT: &[SnapshotSource] = &[
    SnapshotSource {
        sql: "SELECT record FROM tags WHERE workspace_id = $1",
        read: |row| stored_record(row, Record::Tag),
    },
    SnapshotSource {
        sql: "SELECT page_id, tag_id FROM page_tags WHERE workspace_id = $1",
        read: |row| {
            let membership = PageTag {
                page: PageId(row.get("page_id")),
                tag: TagId(row.get("tag_id")),
            };
            Ok((Item::Record(Record::PageTag(membership)), ID_PAIR_BYTES))
        },
    },
];

impl WorkspaceRecords<'_> {
    pub(super) async fn create_tag(&self, tag: &Tag) -> Result<(), RecordError> {
        let id_is_taken = self
            .exists("SELECT EXISTS (SELECT 1 FROM tags WHERE id = $1)", tag.id.0)
            .await?;
        if id_is_taken {
            return Err(ValidationError::RecordAlreadyExists(RecordId::Tag(tag.id)).into());
        }
        check_tag_name(&tag.name)?;
        check_tag_name_is_free(tag.id, &tag.name, self.tag_named(&tag.name).await?)?;
        self.execute(
            "INSERT INTO tags (id, workspace_id, name_key, record) VALUES ($1, $2, $3, $4)",
            &[
                &tag.id.0,
                &self.workspace_id.0,
                &tag_name_key(&tag.name),
                &encode(tag)?,
            ],
        )
        .await?;
        lock(&self.tags).insert(tag.id, Some(tag.clone()));
        Ok(())
    }

    pub(super) async fn set_tag_field(
        &self,
        id: TagId,
        field: &TagField,
    ) -> Result<(), RecordError> {
        let mut tag = self.require_tag(id).await?;
        if let TagField::Name(name) = field {
            check_tag_name(name)?;
            check_tag_name_is_free(id, name, self.tag_named(name).await?)?;
        }
        field.apply_to(&mut tag);
        self.execute(
            "UPDATE tags SET name_key = $2, record = $3 WHERE id = $1",
            &[&id.0, &tag_name_key(&tag.name), &encode(&tag)?],
        )
        .await?;
        lock(&self.tags).insert(id, Some(tag));
        Ok(())
    }

    /// Takes the tag off every page that has it, too.
    pub(super) async fn delete_tag(&self, id: TagId) -> Result<(), RecordError> {
        self.require_tag(id).await?;
        self.execute("DELETE FROM page_tags WHERE tag_id = $1", &[&id.0])
            .await?;
        self.execute("DELETE FROM tags WHERE id = $1", &[&id.0])
            .await?;
        lock(&self.tags).insert(id, None);
        Ok(())
    }

    /// A page that already has the tag keeps it unchanged.
    pub(super) async fn tag_page(&self, membership: PageTag) -> Result<(), RecordError> {
        self.require_page(membership.page).await?;
        self.require_tag(membership.tag).await?;
        self.buffer
            .set_membership((membership.page, membership.tag), true);
        Ok(())
    }

    /// Nothing to check: taking off a tag the page does not have changes nothing.
    pub(super) fn untag_page(&self, membership: PageTag) {
        self.buffer
            .set_membership((membership.page, membership.tag), false);
    }
}

impl WorkspaceRecords<'_> {
    async fn require_tag(&self, id: TagId) -> Result<Tag, RecordError> {
        let known = lock(&self.tags).get(&id).cloned();
        let tag = match known {
            Some(tag) => tag,
            None => {
                let row = self
                    .query_opt(
                        "SELECT record FROM tags WHERE id = $1 AND workspace_id = $2",
                        &[&id.0, &self.workspace_id.0],
                    )
                    .await?;
                let tag: Option<Tag> = row.map(|row| decode(row.get("record"))).transpose()?;
                lock(&self.tags).insert(id, tag.clone());
                tag
            }
        };
        Ok(tag.ok_or(ValidationError::TagNotFound(id))?)
    }

    /// The tag in this workspace whose name is the same as `name` by `tag_name_key`, if any.
    async fn tag_named(&self, name: &str) -> Result<Option<TagId>, RecordError> {
        let row = self
            .query_opt(
                "SELECT id FROM tags WHERE workspace_id = $1 AND name_key = $2 LIMIT 1",
                &[&self.workspace_id.0, &tag_name_key(name)],
            )
            .await?;
        Ok(row.map(|row| TagId(row.get("id"))))
    }
}
