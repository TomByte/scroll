//! What tests and tools of the server and of its clients need: the test database, a fake Google, and the tools'
//! shared helpers. Behind the `test-support` feature, so it never ships.

pub mod fake_google;
pub mod tools;

use deadpool_postgres::Pool;

const DEFAULT_TEST_DATABASE_URL: &str = "postgres://postgres@localhost:5432/scroll_test";

/// Tests share one database, `scroll_test` unless `SCROLL_TEST_DATABASE_URL` says otherwise; every test makes its
/// own users and workspaces, so they can run in parallel.
pub fn test_database_url() -> String {
    std::env::var("SCROLL_TEST_DATABASE_URL")
        .unwrap_or_else(|_| DEFAULT_TEST_DATABASE_URL.to_owned())
}

/// A pool on the test database, migrated.
pub async fn test_pool() -> Pool {
    let pool = crate::connect(&test_database_url()).expect("the test database URL is valid");
    crate::migrate(&pool)
        .await
        .expect("the test database migrates");
    pool
}
