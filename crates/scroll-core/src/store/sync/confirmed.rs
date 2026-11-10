//! A record as one value, and the server-confirmed copy kept while local changes to it are pending.
//!
//! While any queued local transaction touches a record, `confirmed_record` holds the record as the
//! server has it (`None` if the server does not have it). The local tables always show that
//! confirmed state with the queued transactions replayed on top, in queue order.

use std::collections::HashMap;

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::id::PropertyId;
use crate::operation::{FieldChange, Operation, Record, RecordId, Transaction};
use crate::record::{Database, Page, PageTag, PropertyValue, Tag, View, replace_value};
use crate::store::content::refresh_search_body;
use crate::store::kinds::database::{read_database, upsert_database};
use crate::store::kinds::page::{read_page, upsert_page};
use crate::store::kinds::page_tag::memberships_depending_on;
use crate::store::kinds::page_tag::{insert_page_tag, page_has_tag};
use crate::store::kinds::tag::{read_tag, upsert_tag};
use crate::store::kinds::value::{read_property_values, replace_property_values};
use crate::store::kinds::view::{read_view, upsert_view};
use crate::store::kinds::{delete_dependants, delete_record};
use crate::store::{StoreError, decode, encode};

/// A page's property values travel with the page: they are created, replaced and dropped with it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(in crate::store) enum RecordState {
    Page {
        page: Page,
        /// In property id order.
        values: Vec<(PropertyId, PropertyValue)>,
    },
    Database(Database),
    View(View),
    Tag(Tag),
    PageTag(PageTag),
}

impl RecordState {
    pub(in crate::store) fn from_record(record: &Record) -> RecordState {
        match record {
            Record::Page(page) => RecordState::Page {
                page: page.clone(),
                values: Vec::new(),
            },
            Record::Database(database) => RecordState::Database(database.clone()),
            Record::View(view) => RecordState::View(view.clone()),
            Record::Tag(tag) => RecordState::Tag(tag.clone()),
            Record::PageTag(membership) => RecordState::PageTag(*membership),
        }
    }
}

/// What applying `operation` to a record in `state` leaves. Operations on other records, or that
/// do not fit the state (a field change to a missing record), leave it unchanged.
pub(in crate::store) fn apply_to_state(
    state: Option<RecordState>,
    operation: &Operation,
) -> Option<RecordState> {
    match operation {
        Operation::Create(record) => state.or_else(|| Some(RecordState::from_record(record))),
        Operation::Delete(_) => None,
        Operation::Set(change) => state.map(|state| apply_change(state, change)),
    }
}

/// A change to a field of another kind of record leaves the state as it is.
fn apply_change(mut state: RecordState, change: &FieldChange) -> RecordState {
    match change {
        FieldChange::Page { field, .. } => {
            if let RecordState::Page { page, .. } = &mut state {
                field.apply_to(page);
            }
        }
        FieldChange::PropertyValue {
            property, value, ..
        } => {
            if let RecordState::Page { values, .. } = &mut state {
                replace_value(values, *property, value.as_ref());
            }
        }
        FieldChange::Database { field, .. } => {
            if let RecordState::Database(database) = &mut state {
                field.apply_to(database);
            }
        }
        FieldChange::View { field, .. } => {
            if let RecordState::View(view) = &mut state {
                field.apply_to(view);
            }
        }
        FieldChange::Tag { field, .. } => {
            if let RecordState::Tag(tag) = &mut state {
                field.apply_to(tag);
            }
        }
    }
    state
}

/// The local tables' current version of the record.
pub(in crate::store) fn read_state(
    conn: &Connection,
    id: RecordId,
) -> Result<Option<RecordState>, StoreError> {
    Ok(match id {
        RecordId::Page(id) => match read_page(conn, id)? {
            Some(page) => Some(RecordState::Page {
                page,
                values: read_property_values(conn, id)?,
            }),
            None => None,
        },
        RecordId::Database(id) => read_database(conn, id)?.map(RecordState::Database),
        RecordId::View(id) => read_view(conn, id)?.map(RecordState::View),
        RecordId::Tag(id) => read_tag(conn, id)?.map(RecordState::Tag),
        RecordId::PageTag { page, tag } => {
            let membership = PageTag { page, tag };
            page_has_tag(conn, membership)?.then_some(RecordState::PageTag(membership))
        }
    })
}

/// Makes the local tables hold `state` for the record, or no record. Page content is kept.
pub(in crate::store) fn write_state(
    conn: &Connection,
    id: RecordId,
    state: Option<&RecordState>,
) -> Result<(), StoreError> {
    match state {
        None => delete_record(conn, id),
        Some(RecordState::Page { page, values }) => {
            upsert_page(conn, page)?;
            replace_property_values(conn, page.id, values)
        }
        Some(RecordState::Database(database)) => upsert_database(conn, database),
        Some(RecordState::View(view)) => upsert_view(conn, view),
        Some(RecordState::Tag(tag)) => upsert_tag(conn, tag),
        Some(RecordState::PageTag(membership)) => insert_page_tag(conn, *membership),
    }
}

/// The record as the server has it; `None` inside means the server does not have it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(in crate::store) struct ServerState(pub(in crate::store) Option<RecordState>);

/// `None` when no pending local change touches the record, so the local tables are the server's state.
pub(in crate::store) fn tracked_server_state(
    conn: &Connection,
    id: RecordId,
) -> Result<Option<ServerState>, StoreError> {
    let bytes: Option<Vec<u8>> = conn
        .prepare_cached("SELECT state FROM confirmed_record WHERE record = ?1")?
        .query_row([encode(&id)?], |row| row.get(0))
        .optional()?;
    bytes.map(|bytes| decode(&bytes)).transpose()
}

pub(in crate::store) fn tracked_records(conn: &Connection) -> Result<Vec<RecordId>, StoreError> {
    let mut statement = conn.prepare_cached("SELECT record FROM confirmed_record")?;
    let rows = statement.query_map([], |row| row.get::<_, Vec<u8>>(0))?;
    rows.map(|bytes| decode(&bytes?)).collect()
}

pub(in crate::store) fn track_server_state(
    conn: &Connection,
    id: RecordId,
    state: &ServerState,
) -> Result<(), StoreError> {
    conn.prepare_cached("INSERT OR REPLACE INTO confirmed_record (record, state) VALUES (?1, ?2)")?
        .execute(params![encode(&id)?, encode(state)?])?;
    Ok(())
}

/// Removes what a record the server no longer has leaves behind (`kinds::delete_dependants`). A
/// membership among them that a pending local change touches keeps a confirmed copy; the server
/// no longer has it either, so that copy becomes "absent".
pub(in crate::store) fn delete_confirmed_dependants(
    conn: &Connection,
    id: RecordId,
) -> Result<(), StoreError> {
    for membership in memberships_depending_on(conn, id)? {
        let membership = RecordId::PageTag {
            page: membership.page,
            tag: membership.tag,
        };
        if tracked_server_state(conn, membership)?.is_some() {
            track_server_state(conn, membership, &ServerState(None))?;
        }
    }
    delete_dependants(conn, id)
}

/// Called once no pending change touches the record any more: the local tables are the server's
/// state again. A page or tag the server no longer has loses what depends on it here.
pub(in crate::store) fn stop_tracking(
    conn: &Connection,
    id: RecordId,
    state: &ServerState,
) -> Result<(), StoreError> {
    conn.prepare_cached("DELETE FROM confirmed_record WHERE record = ?1")?
        .execute([encode(&id)?])?;
    match state.0 {
        None => delete_confirmed_dependants(conn, id),
        Some(_) => Ok(()),
    }
}

/// The queued local operations, grouped by the record they change, in queue order. Built once per
/// batch of rebuilds: a published workspace can have every record pending at once.
pub(in crate::store) struct PendingOperations<'queue> {
    by_record: HashMap<RecordId, Vec<&'queue Operation>>,
}

impl<'queue> PendingOperations<'queue> {
    pub(in crate::store) fn new(queued: &'queue [Transaction]) -> Self {
        let mut by_record: HashMap<RecordId, Vec<&Operation>> = HashMap::new();
        for operation in queued
            .iter()
            .flat_map(|transaction| &transaction.operations)
        {
            by_record
                .entry(operation.record_id())
                .or_default()
                .push(operation);
        }
        PendingOperations { by_record }
    }

    pub(in crate::store) fn on(&self, id: RecordId) -> &[&'queue Operation] {
        self.by_record.get(&id).map_or(&[], Vec::as_slice)
    }
}

/// Recomputes the local version of a tracked record from its server state and the queued
/// operations, writes it, and stops tracking it if no queued operation touches it any more.
pub(in crate::store) fn rebuild(
    conn: &Connection,
    id: RecordId,
    pending: &PendingOperations,
) -> Result<(), StoreError> {
    let Some(server_state) = tracked_server_state(conn, id)? else {
        return Ok(());
    };
    let operations = pending.on(id);
    let local_state = operations
        .iter()
        .fold(server_state.0.clone(), |state, operation| {
            apply_to_state(state, operation)
        });
    let was_absent = read_state(conn, id)?.is_none();
    write_state(conn, id, local_state.as_ref())?;
    if let (RecordId::Page(page), true, Some(_)) = (id, was_absent, &local_state) {
        refresh_search_body(conn, page)?;
    }
    if operations.is_empty() {
        stop_tracking(conn, id, &server_state)?;
    }
    Ok(())
}
