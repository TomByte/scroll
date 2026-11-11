//! Reading tags: usage counts, a page's tags, and the pages that carry a set of tags. Every
//! reader joins the page and tag tables, since a membership can outlive a deleted page or tag
//! while that delete waits for the server.

use crate::id::{PageId, TagId};
use crate::record::{Page, Tag};
use crate::store::kinds::page::{PAGE_COLUMNS, page_from_row};
use crate::store::kinds::tag::encoded_tag;
use crate::store::{StoreError, WorkspaceStore, sql_limit};

/// A tag and how many pages that are not archived carry it. A page whose delete is waiting for
/// the server is not counted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagUsage {
    pub tag: Tag,
    pub page_count: usize,
}

impl WorkspaceStore {
    /// Every tag, by name ignoring case.
    pub fn tags(&self) -> Result<Vec<TagUsage>, StoreError> {
        let mut statement = self.conn.prepare_cached(
            "WITH every AS (
                 SELECT tag_id, count(*) AS n FROM page_tag INDEXED BY page_tag_by_tag
                 GROUP BY tag_id),
             gone AS (
                 SELECT tag_id, count(*) AS n FROM page_tag INDEXED BY page_tag_of_gone_page
                 WHERE is_page_gone GROUP BY tag_id),
             archived AS (
                 SELECT page_tag.tag_id, count(*) AS n
                 FROM page INDEXED BY page_archived JOIN page_tag ON page_tag.page_id = page.id
                 WHERE page.is_archived GROUP BY page_tag.tag_id)
             SELECT tag.id, tag.name, tag.color,
                 coalesce(every.n, 0) - coalesce(gone.n, 0) - coalesce(archived.n, 0)
             FROM tag
             LEFT JOIN every ON every.tag_id = tag.id
             LEFT JOIN gone ON gone.tag_id = tag.id
             LEFT JOIN archived ON archived.tag_id = tag.id
             ORDER BY tag.name_key, tag.id",
        )?;
        let rows = statement.query_map([], |row| Ok((encoded_tag(row)?, row.get(3)?)))?;
        rows.map(|row| {
            let (tag, page_count) = row?;
            Ok(TagUsage {
                tag: tag.decode()?,
                page_count,
            })
        })
        .collect()
    }

    /// The page's tags, by name ignoring case.
    pub fn page_tags(&self, page: PageId) -> Result<Vec<Tag>, StoreError> {
        let mut statement = self.conn.prepare_cached(
            "SELECT tag.id, tag.name, tag.color FROM page_tag JOIN tag ON tag.id = page_tag.tag_id
             WHERE page_tag.page_id = ?1 ORDER BY tag.name_key, tag.id",
        )?;
        let rows = statement.query_map([page], encoded_tag)?;
        rows.map(|row| row?.decode()).collect()
    }

    /// Pages that are not archived and carry every one of `tags`, by title ignoring case, at most
    /// `limit` of them. No tags matches no pages.
    pub fn pages_with_tags(&self, tags: &[TagId], limit: usize) -> Result<Vec<Page>, StoreError> {
        let mut tags = tags.to_vec();
        tags.sort();
        tags.dedup();
        if tags.is_empty() {
            return Ok(Vec::new());
        }
        // Each further tag narrows the candidates through the (page, tag) primary key.
        let narrowing: String = (2..=tags.len())
            .map(|index| {
                format!(
                    " AND EXISTS (SELECT 1 FROM page_tag AS other
                     WHERE other.page_id = first.page_id AND other.tag_id = ?{index})"
                )
            })
            .collect();
        let limit_index = tags.len() + 1;
        let sql = format!(
            "SELECT {PAGE_COLUMNS} FROM page_tag AS first JOIN page ON page.id = first.page_id
             WHERE first.tag_id = ?1 AND NOT page.is_archived{narrowing}
             ORDER BY page.title COLLATE NOCASE, page.id
             LIMIT ?{limit_index}"
        );
        let mut parameters: Vec<&dyn rusqlite::ToSql> =
            tags.iter().map(|tag| tag as &dyn rusqlite::ToSql).collect();
        let limit = sql_limit(limit);
        parameters.push(&limit);
        let pages = self
            .conn
            .prepare_cached(&sql)?
            .query_map(parameters.as_slice(), page_from_row)?
            .collect::<Result<_, _>>()?;
        Ok(pages)
    }
}

#[cfg(test)]
mod tests;
