pub(in crate::store) mod legacy;

use rusqlite::Connection;

use crate::store::StoreError;
use crate::store::schema::legacy::add_column_widths_to_encoded_views;
use crate::store::sync::queue::index_existing_queue;

/// One step of the schema history. Code steps rewrite data the SQL cannot.
enum Migration {
    Sql(&'static str),
    Code(fn(&Connection) -> Result<(), StoreError>),
}

/// Applied in order; `PRAGMA user_version` records how many have run. Append only.
const MIGRATIONS: &[Migration] = &[
    Migration::Sql(
        "
    CREATE TABLE page_doc (
        page_id BLOB PRIMARY KEY,
        seq     INTEGER NOT NULL,
        bytes   BLOB NOT NULL
    ) WITHOUT ROWID;

    CREATE TABLE page_update (
        page_id BLOB NOT NULL,
        seq     INTEGER NOT NULL,
        bytes   BLOB NOT NULL,
        PRIMARY KEY (page_id, seq)
    ) WITHOUT ROWID;
    "),
    // Records, last-write-wins bookkeeping and the client sync queue.
    Migration::Sql(
        "
    CREATE TABLE page (
        -- A stable integer key, so the search index can use it as its rowid.
        local_key   INTEGER PRIMARY KEY,
        id          BLOB NOT NULL UNIQUE,
        parent_id   BLOB,
        title       TEXT NOT NULL,
        icon        TEXT,
        position    TEXT NOT NULL,
        database_id BLOB,
        is_archived INTEGER NOT NULL
    );
    CREATE INDEX page_by_parent ON page (parent_id, position, id);
    CREATE INDEX page_by_database ON page (database_id, position, id);

    CREATE TABLE database (
        id BLOB PRIMARY KEY
    ) WITHOUT ROWID;

    CREATE TABLE database_property (
        database_id BLOB NOT NULL,
        property_id BLOB NOT NULL,
        definition  BLOB NOT NULL,
        PRIMARY KEY (database_id, property_id)
    ) WITHOUT ROWID;

    CREATE TABLE property_value (
        page_id     BLOB NOT NULL,
        property_id BLOB NOT NULL,
        value       BLOB NOT NULL,
        PRIMARY KEY (page_id, property_id)
    ) WITHOUT ROWID;

    CREATE TABLE view (
        id              BLOB PRIMARY KEY,
        database_id     BLOB NOT NULL,
        view_type       BLOB NOT NULL,
        filter          BLOB NOT NULL,
        sort            BLOB NOT NULL,
        visible_columns BLOB NOT NULL
    ) WITHOUT ROWID;
    CREATE INDEX view_by_database ON view (database_id);

    -- Per record field: the sequence number of the last server-ordered write applied,
    -- and the local transaction whose unacknowledged write it currently holds, if any.
    CREATE TABLE field_clock (
        record     BLOB NOT NULL,
        field      TEXT NOT NULL,
        seq        INTEGER NOT NULL,
        pending_tx BLOB,
        PRIMARY KEY (record, field)
    ) WITHOUT ROWID;
    CREATE INDEX field_clock_by_pending_tx ON field_clock (pending_tx) WHERE pending_tx IS NOT NULL;

    CREATE TABLE tx_queue (
        queue_order INTEGER PRIMARY KEY AUTOINCREMENT,
        change_id   BLOB NOT NULL UNIQUE,
        page_id     BLOB,
        change      BLOB NOT NULL
    );
    CREATE INDEX tx_queue_by_page ON tx_queue (page_id) WHERE page_id IS NOT NULL;

    CREATE TABLE sync_state (
        singleton               INTEGER PRIMARY KEY CHECK (singleton = 1),
        last_seen_workspace_seq INTEGER NOT NULL
    );
    INSERT INTO sync_state (singleton, last_seen_workspace_seq) VALUES (1, 0);
    "),
    // Full-text search; rowid is page.local_key.
    Migration::Sql(
        "
    CREATE VIRTUAL TABLE search USING fts5 (title, body, tokenize = 'unicode61 remove_diacritics 2');
    "),
    // Databases live on a host page; a page names the database it is a row of; views have names.
    Migration::Sql(
        "
    ALTER TABLE page RENAME COLUMN database_id TO row_of_database_id;
    ALTER TABLE database ADD COLUMN host_page_id BLOB NOT NULL DEFAULT x'';
    CREATE INDEX database_by_host_page ON database (host_page_id);
    ALTER TABLE view ADD COLUMN name TEXT NOT NULL DEFAULT '';
    "),
    // Pending local changes are now rebased on a server-confirmed copy of each record they touch.
    Migration::Sql(
        "
    DROP TABLE field_clock;
    CREATE TABLE confirmed_record (
        record BLOB PRIMARY KEY,
        state  BLOB NOT NULL
    ) WITHOUT ROWID;
    "),
    // The workspace's name and whether it is linked to a server (both columns set) or local-only.
    Migration::Sql(
        "
    CREATE TABLE workspace (
        singleton    INTEGER PRIMARY KEY CHECK (singleton = 1),
        name         TEXT NOT NULL,
        server_url   TEXT,
        workspace_id BLOB
    );
    INSERT INTO workspace (singleton, name) VALUES (1, '');
    "),
    // Views gained column widths. The column default is the encoding of an empty list.
    Migration::Sql(
        "ALTER TABLE view ADD COLUMN column_widths BLOB NOT NULL DEFAULT x'00';",
    ),
    // Views inside queued transactions and confirmed records use the old encoding; rewrite them.
    Migration::Code(add_column_widths_to_encoded_views),
    // Tags and page tag memberships. Memberships may outlive their page or tag while a delete is
    // pending, so readers join both. The search index gains a column holding each page's tag names.
    Migration::Sql(
        "
    CREATE TABLE tag (
        id       BLOB PRIMARY KEY,
        name     TEXT NOT NULL,
        name_key TEXT NOT NULL,
        color    BLOB NOT NULL
    ) WITHOUT ROWID;
    CREATE INDEX tag_by_name_key ON tag (name_key);

    CREATE TABLE page_tag (
        page_id BLOB NOT NULL,
        tag_id  BLOB NOT NULL,
        PRIMARY KEY (page_id, tag_id)
    ) WITHOUT ROWID;
    CREATE INDEX page_tag_by_tag ON page_tag (tag_id, page_id);

    CREATE VIRTUAL TABLE search_with_tags USING fts5 (title, body, tags, tokenize = 'unicode61 remove_diacritics 2');
    INSERT INTO search_with_tags (rowid, title, body, tags) SELECT rowid, title, body, '' FROM search;
    DROP TABLE search;
    ALTER TABLE search_with_tags RENAME TO search;
    "),
    // Values are keyed by property first, so a table view reads one property's values in one
    // sequential range instead of one lookup per row. Reading a page's values uses the index.
    Migration::Sql(
        "
    CREATE TABLE property_value_by_property (
        property_id BLOB NOT NULL,
        page_id     BLOB NOT NULL,
        value       BLOB NOT NULL,
        PRIMARY KEY (property_id, page_id)
    ) WITHOUT ROWID;
    INSERT INTO property_value_by_property (property_id, page_id, value)
        SELECT property_id, page_id, value FROM property_value ORDER BY property_id, page_id;
    DROP TABLE property_value;
    ALTER TABLE property_value_by_property RENAME TO property_value;
    CREATE INDEX property_value_by_page ON property_value (page_id);
    "),
    // Which records each queued transaction touches, so acknowledging or rejecting one reads only
    // the transactions that share its records rather than the whole queue.
    Migration::Sql(
        "
    CREATE TABLE queued_record (
        record      BLOB NOT NULL,
        queue_order INTEGER NOT NULL,
        PRIMARY KEY (record, queue_order)
    ) WITHOUT ROWID;
    CREATE INDEX queued_record_by_order ON queued_record (queue_order);
    "),
    Migration::Code(index_existing_queue),
    // A workspace snapshot arriving in parts, kept apart until it is complete.
    Migration::Sql(
        "
    CREATE TABLE staged_snapshot (
        singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
        seq       INTEGER NOT NULL
    );
    CREATE TABLE staged_record (
        kind       INTEGER NOT NULL,
        key        BLOB NOT NULL,
        second_key BLOB NOT NULL,
        record     BLOB NOT NULL,
        PRIMARY KEY (kind, key, second_key)
    ) WITHOUT ROWID;
    CREATE TABLE staged_value (
        page_id     BLOB NOT NULL,
        property_id BLOB NOT NULL,
        value       BLOB NOT NULL,
        PRIMARY KEY (page_id, property_id)
    ) WITHOUT ROWID;
    "),
    // The sidebar's tree without database rows or archived pages, so listing or counting a
    // parent's children never walks a database's rows.
    Migration::Sql(
        "
    CREATE INDEX page_in_tree ON page (parent_id, position, id)
        WHERE row_of_database_id IS NULL AND NOT is_archived;
    ",
    ),
    // Briefly added an FTS5 prefix index. It made bulk inserts (publishing, a first join) about
    // twice as slow at 300,000 rows and many times slower at a million, for a few milliseconds
    // on one-letter searches, so it is gone; the step stays so later steps keep their numbers.
    Migration::Sql(""),
    // How far a staged snapshot's finish has got, so a finish split into short transactions can
    // resume after a crash.
    Migration::Sql(
        "
    ALTER TABLE staged_snapshot ADD COLUMN applied_kind INTEGER;
    ALTER TABLE staged_snapshot ADD COLUMN applied_key BLOB;
    ALTER TABLE staged_snapshot ADD COLUMN applied_second_key BLOB;
    "),
    // Tag usage counts from small indexes: every membership per tag, less those whose page is
    // archived or deleted (a delete waiting for the server keeps its memberships).
    Migration::Sql(
        "
    ALTER TABLE page_tag ADD COLUMN is_page_gone INTEGER NOT NULL DEFAULT 0;
    UPDATE page_tag SET is_page_gone = 1
        WHERE NOT EXISTS (SELECT 1 FROM page WHERE page.id = page_tag.page_id);
    CREATE INDEX page_tag_of_gone_page ON page_tag (tag_id) WHERE is_page_gone;
    CREATE INDEX page_archived ON page (id) WHERE is_archived;
    "),
];

/// Whether the file's schema is behind this build's, so opening it will run migrations.
pub(in crate::store) fn is_behind(conn: &Connection) -> Result<bool, StoreError> {
    let applied_count: usize = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
    Ok(applied_count < MIGRATIONS.len())
}

pub(in crate::store) fn migrate(conn: &mut Connection) -> Result<(), StoreError> {
    let applied_count: usize = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
    for (index, migration) in MIGRATIONS.iter().enumerate().skip(applied_count) {
        let tx = conn.transaction()?;
        match migration {
            Migration::Sql(sql) => tx.execute_batch(sql)?,
            Migration::Code(step) => step(&tx)?,
        }
        tx.pragma_update(None, "user_version", index + 1)?;
        tx.commit()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::id::PageId;
    use crate::operation::{FieldChange, Operation, PageField, RecordId, Transaction};
    use crate::store::encode;
    use crate::store::sync::queue::{QueuedChange, queued_transactions_touching};

    #[test]
    fn the_tree_index_has_the_condition_tree_queries_use() {
        // SQLite uses a partial index only for queries whose condition matches it. If the tree
        // rule changes, the index needs a new migration; this step must stay as it is.
        let tree_index_step = 13;
        let Migration::Sql(sql) = &MIGRATIONS[tree_index_step] else {
            panic!("step {tree_index_step} is not the tree index");
        };
        assert!(sql.contains("CREATE INDEX page_in_tree"));
        assert!(sql.contains(concat!(
            "WHERE ",
            crate::store::kinds::page::in_tree!(),
            ";"
        )));
    }

    #[test]
    fn a_queue_written_before_the_record_index_is_indexed() {
        let mut conn = Connection::open_in_memory().unwrap();
        // Migrations are append-only, so the backfill stays at this index.
        let index_step = 11;
        assert!(matches!(MIGRATIONS[index_step], Migration::Code(_)));
        conn.pragma_update(None, "user_version", 0).unwrap();
        for migration in &MIGRATIONS[..index_step] {
            match migration {
                Migration::Sql(sql) => conn.execute_batch(sql).unwrap(),
                Migration::Code(step) => step(&conn).unwrap(),
            }
        }
        conn.pragma_update(None, "user_version", index_step)
            .unwrap();
        let page = PageId::generate();
        let transaction = Transaction::new(vec![Operation::Set(FieldChange::Page {
            id: page,
            field: PageField::Title("old".into()),
        })]);
        conn.execute(
            "INSERT INTO tx_queue (change_id, change) VALUES (?1, ?2)",
            rusqlite::params![
                transaction.id,
                encode(&QueuedChange::Transaction(transaction.clone())).unwrap()
            ],
        )
        .unwrap();

        migrate(&mut conn).unwrap();
        let touching = queued_transactions_touching(&conn, &[RecordId::Page(page)]).unwrap();
        assert_eq!(touching, vec![transaction]);
    }

    #[test]
    fn migrations_run_once_and_fts5_is_available() {
        let mut conn = Connection::open_in_memory().unwrap();
        migrate(&mut conn).unwrap();
        migrate(&mut conn).unwrap();

        let version: usize = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, MIGRATIONS.len());
        let fts5_compiled: bool = conn
            .query_row(
                "SELECT sqlite_compileoption_used('ENABLE_FTS5')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(fts5_compiled);
    }
}
