//! Times store operations on a benchmark workspace built by the `fixture` example.
//!
//! cargo run --release -p scroll-core --example scale -- COMMAND FIXTURE [DIRECTORY]
//!
//! - `reads`: opening, the page tree, search, tags and table views, twice (first and repeat).
//! - `writes`: the edits a person makes on the biggest database, local-only and linked.
//! - `link`: linking the workspace to a server, alone (for its peak memory).
//! - `publish`: linking the workspace to a server, then acknowledging every queued transaction.
//! - `export`: exporting the whole workspace as one snapshot.
//! - `split FIXTURE DIRECTORY`: writes the snapshot as numbered parts of about 1 MiB.
//! - `join DIRECTORY`: a new linked store applies those parts as a first join would, while another
//!   connection writes every 20 ms and notes its longest wait. With `SCALE_RAW_WRITER` set, that
//!   connection is a plain SQLite one using SQLite's own busy timeout.
//!
//! Each works on a copy of the fixture. Run under `/usr/bin/time -l` for peak memory.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use scroll_core::id::{PageId, PropertyId, TransactionId, WorkspaceId};
use scroll_core::operation::{
    DatabaseField, FieldChange, Operation, Record, RecordId, Transaction,
};
use scroll_core::position::Position;
use scroll_core::record::{Database, Page, Property, PropertyType, PropertyValue};
use scroll_core::store::{QueuedChange, WorkspaceStore};
use scroll_core::sync::WorkspaceSnapshot;

type Outcome = Result<(), Box<dyn std::error::Error>>;

fn main() -> Outcome {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let [command, source, rest @ ..] = arguments.as_slice() else {
        return Err("usage: scale COMMAND FIXTURE [DIRECTORY]".into());
    };
    let source = PathBuf::from(source);
    if command == "join" {
        return join(&source);
    }
    let path = source.with_extension("copy");
    copy_workspace(&source, &path)?;
    let (store, opened) = timed(|| WorkspaceStore::open(&path));
    let mut store = store?;
    report("open (copy, first time)", opened);
    let result = match command.as_str() {
        "reads" => reads(&path, &store),
        "writes" => writes(&mut store),
        "publish" => publish(&mut store),
        "link" => link(&mut store),
        "export" => export(&store),
        "split" => split(
            &store,
            Path::new(rest.first().ok_or("split needs a DIRECTORY")?),
        ),
        _ => Err(format!("unknown command {command}").into()),
    };
    drop(store);
    remove_workspace(&path);
    result
}

fn reads(path: &Path, store: &WorkspaceStore) -> Outcome {
    let (reopened, elapsed) = timed(|| WorkspaceStore::open(path));
    drop(reopened?);
    report("open again", elapsed);
    let (optimized, elapsed) = timed(|| store.optimize());
    optimized?;
    report("optimize (first time on this file)", elapsed);
    for pass in ["first", "repeat"] {
        println!("-- {pass}");
        measure_reads(store)?;
        measure_tables(store)?;
    }
    Ok(())
}

fn measure_reads(store: &WorkspaceStore) -> Outcome {
    let (children, elapsed) = timed(|| store.page_children(None, None, 200));
    let children = children?;
    report(
        &format!("page_children top level, first 200 of {}", children.len()),
        elapsed,
    );
    if let Some(busiest) = children.iter().max_by_key(|child| child.child_count) {
        let (grandchildren, elapsed) =
            timed(|| store.page_children(Some(busiest.page.id), None, 200));
        report(
            &format!("page_children busiest ({})", grandchildren?.len()),
            elapsed,
        );
    }
    for query in ["p", "pl", "plan", "quarterly report", "zzz"] {
        let (hits, elapsed) = timed(|| store.search(query, 20));
        report(&format!("search {query:?} ({} hits)", hits?.len()), elapsed);
    }
    let (tags, elapsed) = timed(|| store.tags());
    let tags = tags?;
    report(&format!("tags with counts ({})", tags.len()), elapsed);
    let popular: Vec<_> = tags.iter().take(2).map(|usage| usage.tag.id).collect();
    let (pages, elapsed) = timed(|| store.pages_with_tags(&popular, 100));
    report(
        &format!("pages_with_tags, 2 tags ({})", pages?.len()),
        elapsed,
    );
    Ok(())
}

fn measure_tables(store: &WorkspaceStore) -> Outcome {
    for (title, database) in databases(store)? {
        for view in store.views(database.id)? {
            let (ids, elapsed) = timed(|| store.view_row_ids(view.id));
            let ids = ids?;
            report(
                &format!(
                    "view_row_ids {title} {:?} ({} shown)",
                    view.name,
                    ids.rows.len()
                ),
                elapsed,
            );
            let middle = ids.rows.len() / 2;
            let window = &ids.rows[middle..(middle + 60).min(ids.rows.len())];
            let (rows, elapsed) = timed(|| store.table_rows(window));
            report(&format!("  table_rows window of {}", rows?.len()), elapsed);
        }
    }
    Ok(())
}

fn databases(
    store: &WorkspaceStore,
) -> Result<Vec<(String, Database)>, Box<dyn std::error::Error>> {
    let mut found = Vec::new();
    for child in store.page_children(None, None, 100_000)? {
        if let Some(database) = store.database_hosted_by(child.page.id)? {
            found.push((child.page.title, database));
        }
    }
    found.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(found)
}

fn writes(store: &mut WorkspaceStore) -> Outcome {
    println!("-- local-only");
    measure_writes(store)?;
    let (linked, elapsed) =
        timed(|| store.link_to_server("https://scroll.test", WorkspaceId::generate()));
    linked?;
    report("link_to_server", elapsed);
    println!("-- linked (everything still queued)");
    measure_writes(store)?;
    measure_remote(store)
}

/// The biggest database, its first view's rows, and its number property.
fn biggest(
    store: &WorkspaceStore,
) -> Result<(Database, Vec<PageId>, Property), Box<dyn std::error::Error>> {
    let (_, database) = databases(store)?.into_iter().next().ok_or("no database")?;
    let view = store.views(database.id)?.remove(0);
    let rows = store.view_row_ids(view.id)?.rows;
    let points = database
        .properties
        .iter()
        .find(|property| property.name == "Points")
        .ok_or("no Points property")?
        .clone();
    Ok((database, rows, points))
}

fn set_value(page: PageId, property: PropertyId, value: f64) -> Operation {
    Operation::Set(FieldChange::PropertyValue {
        page,
        property,
        value: Some(PropertyValue::Number(value)),
    })
}

fn measure_writes(store: &mut WorkspaceStore) -> Outcome {
    let (database, rows, points) = biggest(store)?;
    let middle = rows[rows.len() / 2];
    let edit = Transaction::new(vec![set_value(middle, points.id, 7.0)]);
    let (result, elapsed) = timed(|| store.apply_local_transaction(&edit));
    result?;
    report("cell edit", elapsed);

    let last = store.last_row_position(database.id)?;
    let row = Page {
        id: PageId::generate(),
        parent: Some(database.host_page),
        title: String::new(),
        icon: None,
        position: Position::between(last.as_ref(), None)?,
        row_of: Some(database.id),
        is_archived: false,
    };
    let add = Transaction::new(vec![Operation::Create(Record::Page(row))]);
    let (result, elapsed) = timed(|| store.apply_local_transaction(&add));
    result?;
    report("add row", elapsed);

    let properties: Vec<&Property> = database.properties.iter().collect();
    let paste: Vec<Operation> = rows[..1_000]
        .iter()
        .flat_map(|row| {
            properties.iter().map(|property| {
                Operation::Set(FieldChange::PropertyValue {
                    page: *row,
                    property: property.id,
                    value: Some(pasted_value(property)),
                })
            })
        })
        .collect();
    let (result, elapsed) = timed(|| store.apply_local_transaction(&Transaction::new(paste)));
    result?;
    report("paste 1,000 x 10", elapsed);

    let added = Property {
        id: PropertyId::generate(),
        name: "Added".into(),
        property_type: PropertyType::Text,
        options: vec![],
    };
    let add_property = Transaction::new(vec![Operation::Set(FieldChange::Database {
        id: database.id,
        field: DatabaseField::Property(added.clone()),
    })]);
    let (result, elapsed) = timed(|| store.apply_local_transaction(&add_property));
    result?;
    report("add property", elapsed);
    let remove_property = Transaction::new(vec![Operation::Set(FieldChange::Database {
        id: database.id,
        field: DatabaseField::RemovedProperty(added.id),
    })]);
    let (result, elapsed) = timed(|| store.apply_local_transaction(&remove_property));
    result?;
    report("delete property", elapsed);

    let doomed: Vec<Operation> = rows[rows.len() - 10_000..]
        .iter()
        .map(|row| Operation::Delete(RecordId::Page(*row)))
        .collect();
    let (result, elapsed) = timed(|| store.apply_local_transaction(&Transaction::new(doomed)));
    result?;
    report("delete 10,000 rows", elapsed);
    Ok(())
}

fn pasted_value(property: &Property) -> PropertyValue {
    match property.property_type {
        PropertyType::Number => PropertyValue::Number(1.0),
        PropertyType::Checkbox => PropertyValue::Checkbox(true),
        PropertyType::Select => PropertyValue::Select(property.options[0].id),
        PropertyType::MultiSelect => PropertyValue::MultiSelect(vec![property.options[0].id]),
        PropertyType::Date => PropertyValue::Date(scroll_core::record::Date {
            year: 2026,
            month: 1,
            day: 2,
        }),
        PropertyType::Url => PropertyValue::Url("https://example.com".into()),
        PropertyType::Text => PropertyValue::Text("pasted".into()),
    }
}

fn measure_remote(store: &mut WorkspaceStore) -> Outcome {
    let (_, rows, points) = biggest(store)?;
    // Rows whose publishing is still queued, as during a large publish, and rows that are not.
    let remote = Transaction::new(
        rows[5_000..7_000]
            .iter()
            .map(|row| set_value(*row, points.id, 3.0))
            .collect(),
    );
    let seq = store.last_seen_workspace_seq()? + 1;
    let (result, elapsed) = timed(|| store.apply_remote_transaction(seq, &remote));
    result?;
    report("remote transaction, 2,000 rows (all pending)", elapsed);
    Ok(())
}

fn publish(store: &mut WorkspaceStore) -> Outcome {
    let (linked, elapsed) =
        timed(|| store.link_to_server("https://scroll.test", WorkspaceId::generate()));
    linked?;
    report("link_to_server", elapsed);
    let (count, elapsed) = timed(|| store.queued_change_count());
    report(&format!("queued_change_count ({})", count?), elapsed);
    let (batch, elapsed) = timed(|| store.queued_changes_after(0, 50));
    report(
        &format!("queued_changes_after, 50 ({})", batch?.len()),
        elapsed,
    );
    // Acknowledged 50 at a time, reading the queue in bounded steps, as the sync engine can.
    let started = Instant::now();
    let mut slowest = Duration::ZERO;
    let mut position = 0;
    let mut seq = 0;
    loop {
        let batch = store.queued_changes_after(position, 50)?;
        let Some(last) = batch.last() else {
            break;
        };
        position = last.position;
        let acknowledgements: Vec<(TransactionId, u64)> = batch
            .into_iter()
            .filter_map(|entry| match entry.change {
                QueuedChange::Transaction(transaction) => {
                    seq += 1;
                    Some((transaction.id, seq))
                }
                QueuedChange::PageUpdate(_) => None,
            })
            .collect();
        let (result, elapsed) = timed(|| store.acknowledge_transactions(&acknowledgements));
        result?;
        slowest = slowest.max(elapsed);
    }
    report(
        "acknowledge every publishing transaction",
        started.elapsed(),
    );
    report("  slowest call (50 acknowledgements)", slowest);
    report_file_size("file after the queue drained", store)?;
    if let Err(error) = store.reclaim_free_space(RECLAIM_PAGES) {
        println!("not reclaiming: {error}");
        return Ok(());
    }
    let started = Instant::now();
    let mut slowest = Duration::ZERO;
    let mut calls = 1;
    loop {
        let (remaining, elapsed) = timed(|| store.reclaim_free_space(RECLAIM_PAGES));
        slowest = slowest.max(elapsed);
        calls += 1;
        if remaining? == 0 {
            break;
        }
    }
    report(
        &format!("reclaim free space ({calls} calls of {RECLAIM_PAGES} pages)"),
        started.elapsed(),
    );
    report("  slowest call", slowest);
    report_file_size("file after reclaiming", store)
}

fn link(store: &mut WorkspaceStore) -> Outcome {
    let (linked, elapsed) =
        timed(|| store.link_to_server("https://scroll.test", WorkspaceId::generate()));
    linked?;
    report("link_to_server", elapsed);
    Ok(())
}

/// 8 MiB at a time.
const RECLAIM_PAGES: usize = 2_048;

/// Main file plus its write-ahead log, which holds pages until they are checkpointed.
fn report_file_size(what: &str, store: &WorkspaceStore) -> Outcome {
    let path = store.path().ok_or("the store has no file")?;
    let mut wal = path.as_os_str().to_owned();
    wal.push("-wal");
    let main = std::fs::metadata(&path)?.len();
    let log = std::fs::metadata(&wal).map_or(0, |metadata| metadata.len());
    println!(
        "{what:<55} {:>10.0} MiB (+ {:.0} MiB log)",
        main as f64 / (1 << 20) as f64,
        log as f64 / (1 << 20) as f64
    );
    Ok(())
}

fn export(store: &WorkspaceStore) -> Outcome {
    let (snapshot, elapsed) = timed(|| store.export_workspace_snapshot());
    let snapshot = snapshot?;
    report(
        &format!(
            "export_workspace_snapshot ({} records, {} values)",
            snapshot.records.len(),
            snapshot.property_values.len()
        ),
        elapsed,
    );
    Ok(())
}

/// About a megabyte of records or values per part, like the server's.
const PART_BYTES: usize = 1 << 20;

fn split(store: &WorkspaceStore, directory: &Path) -> Outcome {
    let snapshot = store.export_workspace_snapshot()?;
    std::fs::create_dir_all(directory)?;
    let mut part = empty_part(snapshot.seq);
    let mut part_bytes = 0;
    let mut written = 0;
    let mut flush = |part: &mut WorkspaceSnapshot, bytes: &mut usize| -> Outcome {
        let encoded = postcard::to_stdvec(&*part)?;
        std::fs::write(directory.join(format!("{written:06}.part")), encoded)?;
        written += 1;
        *part = empty_part(part.seq);
        *bytes = 0;
        Ok(())
    };
    for record in snapshot.records {
        part_bytes += postcard::to_stdvec(&record)?.len();
        part.records.push(record);
        if part_bytes > PART_BYTES {
            flush(&mut part, &mut part_bytes)?;
        }
    }
    for value in snapshot.property_values {
        part_bytes += postcard::to_stdvec(&value)?.len();
        part.property_values.push(value);
        if part_bytes > PART_BYTES {
            flush(&mut part, &mut part_bytes)?;
        }
    }
    flush(&mut part, &mut part_bytes)?;
    println!("wrote {written} parts to {}", directory.display());
    Ok(())
}

fn empty_part(seq: u64) -> WorkspaceSnapshot {
    WorkspaceSnapshot {
        seq,
        records: Vec::new(),
        property_values: Vec::new(),
    }
}

fn join(directory: &Path) -> Outcome {
    let path = directory.join("joined.scroll");
    remove_workspace(&path);
    let mut store = WorkspaceStore::open(&path)?;
    store.link_to_server("https://scroll.test", WorkspaceId::generate())?;
    let mut parts: Vec<PathBuf> = std::fs::read_dir(directory)?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<_, _>>()?;
    parts.retain(|part| {
        part.extension()
            .is_some_and(|extension| extension == "part")
    });
    parts.sort();
    let started = Instant::now();
    let mut seq = None;
    for part in &parts {
        let part: WorkspaceSnapshot = postcard::from_bytes(&std::fs::read(part)?)?;
        if seq.is_none() {
            store.begin_workspace_snapshot(part.seq)?;
            seq = Some(part.seq);
        }
        store.stage_workspace_snapshot_part(&part)?;
    }
    report(&format!("stage {} parts", parts.len()), started.elapsed());
    let seq = seq.ok_or("no parts")?;
    // Another connection writes every 20 ms during the finish, as the app would, and notes the
    // longest it waited for the write lock.
    let finished = Arc::new(AtomicBool::new(false));
    let writer = std::thread::spawn({
        let (path, finished) = (path.clone(), finished.clone());
        move || -> Result<Duration, String> {
            let mut app = WorkspaceStore::open(&path).map_err(|error| error.to_string())?;
            if std::env::var_os("SCALE_RAW_WRITER").is_some() {
                drop(app);
                return raw_writer(&path, &finished);
            }
            let mut longest = Duration::ZERO;
            while !finished.load(Ordering::Relaxed) {
                let (result, waited) = timed(|| app.set_workspace_name("written meanwhile"));
                result.map_err(|error| error.to_string())?;
                longest = longest.max(waited);
                std::thread::sleep(Duration::from_millis(20));
            }
            Ok(longest)
        }
    });
    let (result, elapsed) = timed(|| store.finish_workspace_snapshot(seq));
    finished.store(true, Ordering::Relaxed);
    result?;
    report("finish_workspace_snapshot", elapsed);
    report("first join in total", started.elapsed());
    let longest = writer.join().map_err(|_| "the writer thread panicked")??;
    report("  longest wait of another connection's write", longest);
    drop(store);
    remove_workspace(&path);
    Ok(())
}

/// A plain connection that never checkpoints, to tell waiting for the lock apart from doing a
/// checkpoint's work.
fn raw_writer(path: &Path, finished: &AtomicBool) -> Result<Duration, String> {
    let conn = rusqlite::Connection::open(path).map_err(|error| error.to_string())?;
    conn.busy_timeout(Duration::from_secs(60))
        .map_err(|error| error.to_string())?;
    conn.execute_batch("PRAGMA wal_autocheckpoint = 0;")
        .map_err(|error| error.to_string())?;
    let mut longest = Duration::ZERO;
    while !finished.load(Ordering::Relaxed) {
        let (result, waited) =
            timed(|| conn.execute("UPDATE workspace SET name = 'written meanwhile'", []));
        result.map_err(|error| error.to_string())?;
        longest = longest.max(waited);
        std::thread::sleep(Duration::from_millis(20));
    }
    Ok(longest)
}

fn timed<T>(work: impl FnOnce() -> T) -> (T, Duration) {
    let started = Instant::now();
    let result = work();
    (result, started.elapsed())
}

fn report(what: &str, elapsed: Duration) {
    println!("{what:<55} {:>10.1} ms", elapsed.as_secs_f64() * 1000.0);
}

fn copy_workspace(from: &Path, to: &Path) -> std::io::Result<()> {
    remove_workspace(to);
    std::fs::copy(from, to)?;
    Ok(())
}

/// Removes the file and its WAL companions. Any of them may not exist, which is fine here.
fn remove_workspace(path: &Path) {
    for suffix in ["", "-wal", "-shm"] {
        let mut file = path.as_os_str().to_owned();
        file.push(suffix);
        if let Err(error) = std::fs::remove_file(&file)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            eprintln!("could not remove {}: {error}", Path::new(&file).display());
        }
    }
}
