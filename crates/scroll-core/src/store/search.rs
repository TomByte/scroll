//! Full-text search over page titles, page text and tag names (FTS5). Titles and tags are indexed
//! as records change; page text when a page is saved or compacted.

use rusqlite::{Connection, OptionalExtension, params};

use crate::id::{PageId, TagId};
use crate::store::{StoreError, WorkspaceStore};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchHit {
    pub page_id: PageId,
    pub title: String,
    /// A short excerpt around the match, from the title or the text, whichever matched best.
    pub snippet: String,
}

const SNIPPET_WORDS: i64 = 12;

impl WorkspaceStore {
    /// Pages whose title, text or tag names contain every word of `query` (as a word prefix),
    /// best match first. Archived pages are left out. Database rows are pages and are found too.
    ///
    /// Ranking costs in proportion to the number of matches, so a query matching more than
    /// `RANKED_MATCH_LIMIT` pages (a letter or two) returns the newest matches unranked instead;
    /// typing on narrows it to a ranked result.
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchHit>, StoreError> {
        let Some(expression) = match_expression(query) else {
            return Ok(Vec::new());
        };
        let sql = if is_broad(&self.conn, &expression)? {
            NEWEST_MATCHES
        } else {
            BEST_MATCHES
        };
        let mut statement = self.conn.prepare_cached(sql)?;
        let hits = statement
            .query_map(params![expression, limit, SNIPPET_WORDS], |row| {
                Ok(SearchHit {
                    page_id: row.get(0)?,
                    title: row.get(1)?,
                    snippet: row.get(2)?,
                })
            })?
            .collect::<Result<_, _>>()?;
        Ok(hits)
    }
}

/// Above this many matches a query is too broad to rank quickly (about 30 ms at this size).
pub(crate) const RANKED_MATCH_LIMIT: usize = 20_000;

const BEST_MATCHES: &str = "SELECT page.id, page.title, snippet(search, -1, '', '', '…', ?3)
     FROM search CROSS JOIN page ON page.local_key = search.rowid
     WHERE search MATCH ?1 AND NOT page.is_archived
     ORDER BY rank
     LIMIT ?2";

/// Walks the index from the newest entry and stops at `limit`, so its cost does not grow with
/// the number of matches.
const NEWEST_MATCHES: &str = "SELECT page.id, page.title, snippet(search, -1, '', '', '…', ?3)
     FROM search CROSS JOIN page ON page.local_key = search.rowid
     WHERE search MATCH ?1 AND NOT page.is_archived
     ORDER BY search.rowid DESC
     LIMIT ?2";

fn is_broad(conn: &Connection, expression: &str) -> Result<bool, StoreError> {
    let capped_count: usize = conn
        .prepare_cached(
            "SELECT count(*) FROM (SELECT 1 FROM search WHERE search MATCH ?1 LIMIT ?2)",
        )?
        .query_row(params![expression, RANKED_MATCH_LIMIT + 1], |row| {
            row.get(0)
        })?;
    Ok(capped_count > RANKED_MATCH_LIMIT)
}

pub(in crate::store) fn insert_entry(
    conn: &Connection,
    local_key: i64,
    title: &str,
) -> Result<(), StoreError> {
    conn.prepare_cached(&format!(
        "INSERT INTO search (rowid, title, body, tags) VALUES (?1, ?2, '', {TAG_NAMES_OF_PAGE})"
    ))?
    .execute(params![local_key, title])?;
    Ok(())
}

/// The names of the tags of the page whose `local_key` is `?1`, space separated.
const TAG_NAMES_OF_PAGE: &str = "(SELECT coalesce(group_concat(tag.name, ' '), '')
    FROM page JOIN page_tag ON page_tag.page_id = page.id JOIN tag ON tag.id = page_tag.tag_id
    WHERE page.local_key = ?1)";

/// Re-indexes the page's tag names after it gained or lost a tag.
pub(in crate::store) fn refresh_tags(conn: &Connection, page_id: PageId) -> Result<(), StoreError> {
    let local_key: Option<i64> = conn
        .prepare_cached("SELECT local_key FROM page WHERE id = ?1")?
        .query_row([page_id], |row| row.get(0))
        .optional()?;
    let Some(local_key) = local_key else {
        return Ok(());
    };
    conn.prepare_cached(&format!(
        "UPDATE search SET tags = {TAG_NAMES_OF_PAGE} WHERE rowid = ?1"
    ))?
    .execute([local_key])?;
    Ok(())
}

/// Re-indexes the tag names of every page with a membership of the tag, after the tag was
/// renamed, deleted or restored.
pub(in crate::store) fn refresh_tags_of_pages_tagged(
    conn: &Connection,
    tag: TagId,
) -> Result<(), StoreError> {
    let local_keys: Vec<i64> = conn
        .prepare_cached(
            "SELECT page.local_key FROM page_tag JOIN page ON page.id = page_tag.page_id
             WHERE page_tag.tag_id = ?1",
        )?
        .query_map([tag], |row| row.get(0))?
        .collect::<Result<_, _>>()?;
    let mut statement = conn.prepare_cached(&format!(
        "UPDATE search SET tags = {TAG_NAMES_OF_PAGE} WHERE rowid = ?1"
    ))?;
    for local_key in local_keys {
        statement.execute([local_key])?;
    }
    Ok(())
}

pub(in crate::store) fn update_title(
    conn: &Connection,
    page_id: PageId,
    title: &str,
) -> Result<(), StoreError> {
    conn.prepare_cached(
        "UPDATE search SET title = ?2 WHERE rowid = (SELECT local_key FROM page WHERE id = ?1)",
    )?
    .execute(params![page_id, title])?;
    Ok(())
}

pub(in crate::store) fn update_body(
    conn: &Connection,
    page_id: PageId,
    body: &str,
) -> Result<(), StoreError> {
    conn.prepare_cached(
        "UPDATE search SET body = ?2 WHERE rowid = (SELECT local_key FROM page WHERE id = ?1)",
    )?
    .execute(params![page_id, body])?;
    Ok(())
}

pub(in crate::store) fn delete_entry(conn: &Connection, page_id: PageId) -> Result<(), StoreError> {
    conn.prepare_cached(
        "DELETE FROM search WHERE rowid = (SELECT local_key FROM page WHERE id = ?1)",
    )?
    .execute([page_id])?;
    Ok(())
}

/// Turns what the user typed into an FTS5 query: each word quoted (so punctuation and FTS5 operators
/// are taken literally) and matched as a prefix, all words required.
fn match_expression(query: &str) -> Option<String> {
    let terms: Vec<String> = query
        .split_whitespace()
        .map(|word| format!("\"{}\"*", word.replace('"', "\"\"")))
        .collect();
    if terms.is_empty() {
        return None;
    }
    Some(terms.join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::operation::{FieldChange, Operation, PageField, Record, RecordId, Transaction};
    use crate::page_doc::{BlockKind, BlockPlacement, PageDoc};
    use crate::position::Position;
    use crate::record::Page;
    use crate::store::open_in_memory;

    fn create_page(store: &mut WorkspaceStore, title: &str) -> PageId {
        let page = Page {
            id: PageId::generate(),
            parent: None,
            title: title.into(),
            icon: None,
            position: Position::between(None, None).unwrap(),
            row_of: None,
            is_archived: false,
        };
        let id = page.id;
        store
            .apply_local_transaction(&Transaction::new(vec![Operation::Create(Record::Page(
                page,
            ))]))
            .unwrap();
        id
    }

    fn hit_ids(store: &WorkspaceStore, query: &str) -> Vec<PageId> {
        store
            .search(query, 10)
            .unwrap()
            .into_iter()
            .map(|hit| hit.page_id)
            .collect()
    }

    #[test]
    fn titles_are_searchable_by_word_prefix_and_follow_renames() {
        let mut store = open_in_memory();
        let page = create_page(&mut store, "Quarterly planning");
        create_page(&mut store, "Recipes");

        assert_eq!(hit_ids(&store, "plan"), vec![page]);
        assert_eq!(hit_ids(&store, "quarterly plan"), vec![page]);
        assert!(hit_ids(&store, "quarterly recipes").is_empty());

        let rename = FieldChange::Page {
            id: page,
            field: PageField::Title("Roadmap".into()),
        };
        store
            .apply_local_transaction(&Transaction::new(vec![Operation::Set(rename)]))
            .unwrap();
        assert!(hit_ids(&store, "plan").is_empty());
        assert_eq!(hit_ids(&store, "road"), vec![page]);
    }

    #[test]
    fn page_text_is_searchable_once_saved_and_gone_once_deleted() {
        let mut store = open_in_memory();
        let page = create_page(&mut store, "Notes");
        let doc = PageDoc::new();
        let block = doc
            .insert_block(BlockPlacement::LastChildOf(None), BlockKind::Paragraph)
            .unwrap();
        doc.insert_text(block, 0, "the migration finished on tuesday")
            .unwrap();
        store.save_page_doc(page, &doc, 0).unwrap();

        let hits = store.search("tues", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].title, "Notes");
        assert!(
            hits[0].snippet.contains("tuesday"),
            "snippet was {:?}",
            hits[0].snippet
        );

        store
            .apply_local_transaction(&Transaction::new(vec![Operation::Delete(RecordId::Page(
                page,
            ))]))
            .unwrap();
        assert!(hit_ids(&store, "tuesday").is_empty());
    }

    #[test]
    fn archived_pages_are_not_found() {
        let mut store = open_in_memory();
        let page = create_page(&mut store, "Old plans");
        let archive = FieldChange::Page {
            id: page,
            field: PageField::IsArchived(true),
        };
        store
            .apply_local_transaction(&Transaction::new(vec![Operation::Set(archive)]))
            .unwrap();
        assert!(hit_ids(&store, "plans").is_empty());
    }

    #[test]
    fn a_query_too_broad_to_rank_returns_the_newest_matches() {
        let mut store = open_in_memory();
        let best = create_page(&mut store, "alpha alpha alpha");
        let operations: Vec<Operation> = (0..RANKED_MATCH_LIMIT)
            .map(|index| {
                Operation::Create(Record::Page(Page {
                    id: PageId::generate(),
                    parent: None,
                    title: format!("alpha {index} filler words here"),
                    icon: None,
                    position: Position::between(None, None).unwrap(),
                    row_of: None,
                    is_archived: false,
                }))
            })
            .collect();
        store
            .apply_local_transaction(&Transaction::new(operations))
            .unwrap();
        let newest = create_page(&mut store, "alpha newest");

        let broad = store.search("alpha", 3).unwrap();
        assert_eq!(broad[0].page_id, newest);
        assert!(broad.iter().all(|hit| hit.page_id != best));

        // Narrowed below the limit, results are ranked again.
        assert_eq!(hit_ids(&store, "alpha newest"), vec![newest]);
    }

    #[test]
    fn query_syntax_in_user_input_is_taken_literally() {
        let mut store = open_in_memory();
        let page = create_page(&mut store, "C++ \"quoted\" AND notes");
        assert_eq!(hit_ids(&store, "\"quoted"), vec![page]);
        assert_eq!(hit_ids(&store, "AND"), vec![page]);
        assert!(hit_ids(&store, "NEAR(").is_empty());
        assert!(hit_ids(&store, "   ").is_empty());
    }
}
