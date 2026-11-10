//! The app's side at the target scale, against a running server: a generated workspace of a million rows is
//! published by a real `SyncEngine`, then joined by a second one. Reports times, this process's peak memory,
//! the upload queue as it drains, and the size of each workspace file.
//!
//! cargo run --release -p scroll-sync --example engine_scale -- --url http://127.0.0.1:8080 --invite CODE \
//!     [--directory /path/for/files] [--rows 0 for the full target, else one database of that many rows] \
//!     [--fixture-cache /path/fixture.scroll]
//!
//! Generating the full target takes a long time; with `--fixture-cache` the generated file is kept there and
//! copied for later runs of the same `--rows`.
//!
//! The invite is a sign-up invite (`scroll-server create-invite`). It fills the server with a million rows, so it
//! refuses a server whose `SERVER_NAME` does not contain "test" unless given `--allow-any-server`.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use scroll_core::store::WorkspaceStore;
use scroll_core::store::fixture::{FixtureSize, generate_workspace};
use scroll_server::test_support::tools::{
    MemoryWatch, argument, refuse_unless_test_target, resident_megabytes,
};
use scroll_sync::{
    AccountClient, NewAccount, SyncEngine, SyncEvent, SyncEvents, create_linked_workspace_file,
    publish_workspace,
};
use uuid::Uuid;

const PASSWORD: &str = "engine scale password";

fn own_resident_megabytes() -> f64 {
    resident_megabytes(std::process::id()).unwrap_or(0.0)
}

/// The file with its write-ahead log, in megabytes.
fn file_megabytes(path: &Path) -> f64 {
    let size = |path: PathBuf| std::fs::metadata(path).map_or(0, |metadata| metadata.len());
    let mut wal = path.as_os_str().to_owned();
    wal.push("-wal");
    (size(path.to_path_buf()) + size(PathBuf::from(wal))) as f64 / 1_048_576.0
}

/// Space SQLite holds for reuse but does not give back to the file system until a vacuum.
fn free_megabytes(path: &Path) -> f64 {
    let connection = rusqlite::Connection::open(path).expect("the file opens");
    let pages: (i64, i64) = connection
        .query_row(
            "SELECT freelist_count, page_size FROM pragma_freelist_count, pragma_page_size",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("the pragmas answer");
    (pages.0 * pages.1) as f64 / 1_048_576.0
}

/// Waits for events until `done` says so, printing a line every ten seconds from `progress`.
async fn follow(
    events: &mut SyncEvents,
    mut done: impl FnMut(&SyncEvent) -> bool,
    mut progress: impl FnMut() -> String,
) {
    let mut next_report = Instant::now() + Duration::from_secs(10);
    loop {
        while let Some(event) = events.try_next() {
            if let SyncEvent::Error(error) = &event {
                panic!("the engine reported an error: {error}");
            }
            if done(&event) {
                return;
            }
        }
        if Instant::now() >= next_report {
            println!("  {}", progress());
            next_report += Duration::from_secs(10);
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::main]
async fn main() {
    let url = argument("--url", "http://127.0.0.1:8080");
    let invite = argument("--invite", "");
    assert!(!invite.is_empty(), "--invite CODE is required");
    let rows: usize = argument("--rows", "0").parse().expect("--rows is a number");
    let directory = PathBuf::from(argument(
        "--directory",
        &std::env::temp_dir().to_string_lossy(),
    ))
    .join(format!("engine-scale-{}", Uuid::now_v7()));
    std::fs::create_dir_all(&directory).expect("the directory can be made");
    let size = match rows {
        0 => FixtureSize::target(),
        rows => FixtureSize::one_database(rows),
    };

    refuse_unless_test_target(&url, "").await;
    let accounts = AccountClient::new(&url, None).expect("a valid server URL");
    let email = format!("engine-scale-{}@example.com", Uuid::now_v7());
    accounts
        .sign_up(NewAccount {
            email: email.clone(),
            password: PASSWORD.into(),
            invite_code: Some(invite),
        })
        .await
        .expect("sign up");
    let session = accounts.sign_in(&email, PASSWORD).await.expect("sign in");

    let laptop_path = directory.join("laptop.scroll");
    let fixture_cache = argument("--fixture-cache", "");
    let started = Instant::now();
    if fixture_cache.is_empty() || !Path::new(&fixture_cache).exists() {
        let fixture = generate_workspace(&laptop_path, &size, 7).expect("the fixture is generated");
        let row_count: usize = fixture
            .databases
            .iter()
            .map(|database| database.rows.len())
            .sum();
        println!(
            "generated {row_count} rows and {} pages in {:.1?}, file {:.0} MB",
            fixture.pages.len(),
            started.elapsed(),
            file_megabytes(&laptop_path)
        );
        if !fixture_cache.is_empty() {
            // Folds the write-ahead log into the file, so the one file is the whole workspace.
            drop(WorkspaceStore::open(&laptop_path).expect("the file opens"));
            std::fs::copy(&laptop_path, &fixture_cache).expect("the fixture is cached");
        }
    } else {
        std::fs::copy(&fixture_cache, &laptop_path).expect("the cached fixture is copied");
        println!(
            "copied the cached fixture, file {:.0} MB",
            file_megabytes(&laptop_path)
        );
    }

    let memory = MemoryWatch::start(std::process::id());
    let mut laptop = WorkspaceStore::open(&laptop_path).expect("the file opens");
    laptop.set_workspace_name("Engine scale").expect("named");
    let started = Instant::now();
    let membership = publish_workspace(&accounts, &session.session_token, &mut laptop)
        .await
        .expect("published");
    let queued = laptop.queued_change_count().expect("counted");
    println!(
        "queued {queued} changes for publishing in {:.1?}, file {:.0} MB",
        started.elapsed(),
        file_megabytes(&laptop_path)
    );
    let started = Instant::now();
    let (engine, mut events) = SyncEngine::start(&laptop_path, session.session_token.clone(), None)
        .expect("the engine starts");
    let mut remaining = queued;
    follow(
        &mut events,
        |event| {
            if let SyncEvent::UploadProgress { remaining: now } = event {
                remaining = *now;
            }
            remaining == 0
        },
        || {
            format!(
                "{:.0?}: {} changes left to upload, {:.0} MB resident",
                started.elapsed(),
                laptop.queued_change_count().unwrap_or(0),
                own_resident_megabytes()
            )
        },
    )
    .await;
    let publish_time = started.elapsed();
    engine.shut_down();
    let peak_megabytes = memory.peak_megabytes().await;
    println!(
        "published in {publish_time:.1?}; peak memory {:.0} MB; file {:.0} MB, of which {:.0} MB free pages",
        peak_megabytes,
        file_megabytes(&laptop_path),
        free_megabytes(&laptop_path)
    );
    drop(laptop);

    let desktop_path = directory.join("desktop.scroll");
    create_linked_workspace_file(&desktop_path, &url, &membership).expect("the file is made");
    let memory = MemoryWatch::start(std::process::id());
    let started = Instant::now();
    let (engine, mut events) =
        SyncEngine::start(&desktop_path, session.session_token.clone(), None)
            .expect("the engine starts");
    let mut first_part = None;
    let mut downloaded = None;
    let parts = std::cell::Cell::new(0);
    follow(
        &mut events,
        |event| match event {
            SyncEvent::DownloadProgress {
                received_parts,
                is_complete,
            } => {
                first_part.get_or_insert(started.elapsed());
                parts.set(*received_parts);
                if *is_complete {
                    downloaded = Some(started.elapsed());
                }
                false
            }
            // Sent once the staged snapshot has replaced the records.
            SyncEvent::AllRecordsChanged => true,
            _ => false,
        },
        || {
            format!(
                "{:.0?}: {} parts, {:.0} MB resident",
                started.elapsed(),
                parts.get(),
                own_resident_megabytes()
            )
        },
    )
    .await;
    let join_time = started.elapsed();
    engine.shut_down();
    let peak_megabytes = memory.peak_megabytes().await;
    println!(
        "joined in {join_time:.1?} ({} parts, the first after {:.1?}, the last after {:.1?}); peak memory {:.0} MB; file {:.0} MB",
        parts.get(),
        first_part.unwrap_or_default(),
        downloaded.unwrap_or_default(),
        peak_megabytes,
        file_megabytes(&desktop_path)
    );

    let laptop = WorkspaceStore::open(&laptop_path).expect("the file opens");
    let desktop = WorkspaceStore::open(&desktop_path).expect("the file opens");
    let laptop_snapshot = laptop.export_workspace_snapshot().expect("exported");
    let desktop_snapshot = desktop.export_workspace_snapshot().expect("exported");
    println!(
        "laptop: {} records, {} values; desktop: {} records, {} values; equal: {}",
        laptop_snapshot.records.len(),
        laptop_snapshot.property_values.len(),
        desktop_snapshot.records.len(),
        desktop_snapshot.property_values.len(),
        laptop_snapshot.records.len() == desktop_snapshot.records.len()
            && laptop_snapshot.property_values.len() == desktop_snapshot.property_values.len()
    );
    std::fs::remove_dir_all(&directory).expect("the files are removed");
}
