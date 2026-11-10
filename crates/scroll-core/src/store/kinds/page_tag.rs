//! Which pages carry which tags.
//!
//! A membership row can outlive its page or tag while that delete is pending (a rejection would
//! bring both back), so every reader joins the page and tag tables. Confirmed deletes remove the
//! rows (`delete_memberships_of_page`, `delete_memberships_of_tag`); the sync side first notes
//! which of them the server no longer has (`sync::confirmed::delete_confirmed_dependants`).

use rusqlite::{Connection, Row, params};

use crate::id::{PageId, TagId};
use crate::operation::RecordId;
use crate::record::PageTag;
use crate::store::{StoreError, search};

pub(in crate::store) fn page_has_tag(
    conn: &Connection,
    membership: PageTag,
) -> Result<bool, StoreError> {
    Ok(conn
        .prepare_cached(
            "SELECT EXISTS (SELECT 1 FROM page_tag WHERE page_id = ?1 AND tag_id = ?2)",
        )?
        .query_row(params![membership.page, membership.tag], |row| row.get(0))?)
}

/// Needs only the page to exist: the tag may be one whose delete is pending here.
pub(in crate::store) fn insert_page_tag(
    conn: &Connection,
    membership: PageTag,
) -> Result<(), StoreError> {
    let inserted_count = conn
        .prepare_cached(
            "INSERT OR IGNORE INTO page_tag (page_id, tag_id)
         SELECT ?1, ?2 WHERE EXISTS (SELECT 1 FROM page WHERE id = ?1)",
        )?
        .execute(params![membership.page, membership.tag])?;
    if inserted_count > 0 {
        search::refresh_tags(conn, membership.page)?;
    }
    Ok(())
}

pub(in crate::store) fn delete_page_tag(
    conn: &Connection,
    membership: PageTag,
) -> Result<(), StoreError> {
    let deleted_count = conn
        .prepare_cached("DELETE FROM page_tag WHERE page_id = ?1 AND tag_id = ?2")?
        .execute(params![membership.page, membership.tag])?;
    if deleted_count > 0 {
        search::refresh_tags(conn, membership.page)?;
    }
    Ok(())
}

/// A deleted page keeps its memberships until the server confirms the delete; they no longer
/// count towards their tags' usage.
pub(in crate::store) fn mark_memberships_of_gone_page(
    conn: &Connection,
    page: PageId,
) -> Result<(), StoreError> {
    conn.prepare_cached("UPDATE page_tag SET is_page_gone = 1 WHERE page_id = ?1")?
        .execute([page])?;
    Ok(())
}

/// A page that came back (its delete was rejected) counts again.
pub(in crate::store) fn mark_memberships_of_returned_page(
    conn: &Connection,
    page: PageId,
) -> Result<(), StoreError> {
    conn.prepare_cached(
        "UPDATE page_tag SET is_page_gone = 0 WHERE page_id = ?1 AND is_page_gone",
    )?
    .execute([page])?;
    Ok(())
}

/// The server no longer has the page, so it has none of its memberships either.
pub(in crate::store) fn delete_memberships_of_page(
    conn: &Connection,
    page: PageId,
) -> Result<(), StoreError> {
    conn.execute("DELETE FROM page_tag WHERE page_id = ?1", [page])?;
    Ok(())
}

/// The server no longer has the tag, so no page carries it there.
pub(in crate::store) fn delete_memberships_of_tag(
    conn: &Connection,
    tag: TagId,
) -> Result<(), StoreError> {
    conn.execute("DELETE FROM page_tag WHERE tag_id = ?1", [tag])?;
    Ok(())
}

/// The memberships that go with the page or tag `id` when its delete is final; none for other
/// kinds.
pub(in crate::store) fn memberships_depending_on(
    conn: &Connection,
    id: RecordId,
) -> Result<Vec<PageTag>, StoreError> {
    match id {
        RecordId::Page(page) => memberships(
            conn,
            "SELECT page_id, tag_id FROM page_tag WHERE page_id = ?1",
            page.0,
        ),
        RecordId::Tag(tag) => memberships(
            conn,
            "SELECT page_id, tag_id FROM page_tag WHERE tag_id = ?1",
            tag.0,
        ),
        RecordId::Database(_) | RecordId::View(_) | RecordId::PageTag { .. } => Ok(Vec::new()),
    }
}

/// Every membership whose page and tag both exist, for exporting.
pub(in crate::store) fn all_page_tags(conn: &Connection) -> Result<Vec<PageTag>, StoreError> {
    let mut statement = conn.prepare_cached(
        "SELECT page_tag.page_id, page_tag.tag_id FROM page_tag
         JOIN page ON page.id = page_tag.page_id JOIN tag ON tag.id = page_tag.tag_id
         ORDER BY page_tag.page_id, page_tag.tag_id",
    )?;
    let rows = statement.query_map([], page_tag_from_row)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

fn memberships(conn: &Connection, sql: &str, key: uuid::Uuid) -> Result<Vec<PageTag>, StoreError> {
    let mut statement = conn.prepare_cached(sql)?;
    let rows = statement.query_map([key], page_tag_from_row)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

fn page_tag_from_row(row: &Row<'_>) -> rusqlite::Result<PageTag> {
    Ok(PageTag {
        page: row.get(0)?,
        tag: row.get(1)?,
    })
}

/// Membership rows the staged snapshot does not have.
pub(in crate::store) fn memberships_not_staged(
    conn: &Connection,
    staging_number: u8,
) -> Result<Vec<RecordId>, StoreError> {
    let mut statement = conn.prepare(
        "SELECT page_id, tag_id FROM page_tag WHERE NOT EXISTS (SELECT 1 FROM staged_record
         WHERE kind = ?1 AND key = page_tag.page_id AND second_key = page_tag.tag_id)",
    )?;
    let memberships = statement.query_map([staging_number], page_tag_from_row)?;
    memberships
        .map(|membership| {
            let membership = membership?;
            Ok(RecordId::PageTag {
                page: membership.page,
                tag: membership.tag,
            })
        })
        .collect()
}
