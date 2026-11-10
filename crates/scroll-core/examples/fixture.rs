//! Builds a benchmark workspace file.
//!
//! cargo run --release -p scroll-core --example fixture -- PATH [target | ROWS]
//!
//! `target` (the default) is the scale target: 1,000,000 rows and 100,000 pages. A number builds
//! one database of that many rows. Prints the build time and file size.

use std::path::PathBuf;
use std::time::Instant;

use scroll_core::store::fixture::{FixtureSize, generate_workspace};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = std::env::args().skip(1);
    let path = PathBuf::from(
        arguments
            .next()
            .ok_or("usage: fixture PATH [target | ROWS]")?,
    );
    let size = match arguments.next().as_deref() {
        None | Some("target") => FixtureSize::target(),
        Some(rows) => FixtureSize::one_database(rows.parse()?),
    };
    let started = Instant::now();
    let fixture = generate_workspace(&path, &size, 1)?;
    let elapsed = started.elapsed();
    let bytes = std::fs::metadata(&path)?.len();
    let rows: usize = fixture
        .databases
        .iter()
        .map(|database| database.rows.len())
        .sum();
    println!(
        "{rows} rows in {} databases, {} pages: built in {elapsed:.1?}, {:.0} MiB",
        fixture.databases.len(),
        fixture.pages.len(),
        bytes as f64 / (1 << 20) as f64
    );
    Ok(())
}
