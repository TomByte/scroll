use std::str::FromStr;
use std::time::Duration;

use deadpool_postgres::{Manager, Pool, PoolError, Runtime};
use tokio_postgres::NoTls;

/// Applied in order inside one transaction; `schema_migrations` records the versions that have run. Append only.
const MIGRATIONS: &[&str] = &[
    include_str!("../migrations/0001_accounts_and_pages.sql"),
    include_str!("../migrations/0002_page_snapshots.sql"),
    include_str!("../migrations/0003_records.sql"),
    include_str!("../migrations/0004_tags.sql"),
    include_str!("../migrations/0005_sign_in_with_google.sql"),
    include_str!("../migrations/0006_display_names.sql"),
    include_str!("../migrations/0007_page_records.sql"),
];

/// Arbitrary ("SCROLL" in ASCII), but fixed: every server instance must agree on it.
const MIGRATION_LOCK_KEY: i64 = 0x5343_524F_4C4C;

const CONNECTION_WAIT: Duration = Duration::from_secs(5);

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("could not get a database connection: {0}")]
    Pool(#[from] PoolError),
    #[error("database query failed: {0}")]
    Query(#[from] tokio_postgres::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum ConnectError {
    #[error("the database URL is not a valid Postgres connection string: {0}")]
    InvalidUrl(tokio_postgres::Error),
    #[error("could not build the connection pool: {0}")]
    Pool(#[from] deadpool_postgres::BuildError),
}

pub fn connect(database_url: &str) -> Result<Pool, ConnectError> {
    let config =
        tokio_postgres::Config::from_str(database_url).map_err(ConnectError::InvalidUrl)?;
    let pool = Pool::builder(Manager::new(config, NoTls))
        .runtime(Runtime::Tokio1)
        .wait_timeout(Some(CONNECTION_WAIT))
        .create_timeout(Some(CONNECTION_WAIT))
        .build()?;
    Ok(pool)
}

/// One round trip to the database.
pub async fn check(pool: &Pool) -> Result<(), DbError> {
    pool.get().await?.execute("SELECT 1", &[]).await?;
    Ok(())
}

pub async fn migrate(pool: &Pool) -> Result<(), DbError> {
    let mut client = pool.get().await?;
    let tx = client.transaction().await?;
    // Several instances (or parallel test servers) may start at once; the lock makes them take turns.
    tx.execute("SELECT pg_advisory_xact_lock($1)", &[&MIGRATION_LOCK_KEY])
        .await?;
    // Otherwise every start logs a notice that the table below already exists.
    tx.batch_execute("SET LOCAL client_min_messages = warning")
        .await?;
    tx.batch_execute(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
            version    INTEGER PRIMARY KEY,
            applied_at TIMESTAMPTZ NOT NULL DEFAULT now()
        )",
    )
    .await?;
    let latest_applied: i32 = tx
        .query_one(
            "SELECT coalesce(max(version), 0) FROM schema_migrations",
            &[],
        )
        .await?
        .get(0);
    for (version, sql) in (1_i32..)
        .zip(MIGRATIONS)
        .filter(|(version, _)| *version > latest_applied)
    {
        tx.batch_execute(sql).await?;
        tx.execute(
            "INSERT INTO schema_migrations (version) VALUES ($1)",
            &[&version],
        )
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Sequence numbers are BIGINT in the database and `u64` on the wire.
pub fn wire_seq(seq: i64) -> u64 {
    u64::try_from(seq).expect("sequence numbers are never negative")
}

/// A sequence number from a client, for comparing with stored ones. One beyond what a BIGINT holds counts as
/// having seen everything.
pub fn db_seq(seq: u64) -> i64 {
    i64::try_from(seq).unwrap_or(i64::MAX)
}
