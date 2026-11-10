//! What the load and scale tools of the server and the sync client share: command-line arguments, the refusal to
//! fill a server that is not a test one, and watching a process's memory.

use std::time::Duration;

use scroll_api::accounts::ServerInfoResponse;
use scroll_api::paths;

/// `--name value` from the command line, or `default`.
pub fn argument(name: &str, default: &str) -> String {
    let arguments: Vec<String> = std::env::args().collect();
    arguments
        .iter()
        .position(|argument| argument == name)
        .and_then(|index| arguments.get(index + 1).cloned())
        .unwrap_or_else(|| default.to_owned())
}

/// The tools fill a server with test data. Unless `--allow-any-server` is given, they refuse a server whose name
/// (`SERVER_NAME`, shown by `/server-info`) does not contain "test", and a database whose name does not.
/// `database_url` is empty when the tool does not use the database.
pub async fn refuse_unless_test_target(url: &str, database_url: &str) {
    if std::env::args().any(|argument| argument == "--allow-any-server") {
        return;
    }
    let server_info = format!("{}{}", url.trim_end_matches('/'), paths::SERVER_INFO);
    let answer: ServerInfoResponse = reqwest::get(server_info)
        .await
        .expect("the server is reachable")
        .json()
        .await
        .expect("the server answers /server-info");
    let name = answer.info.name;
    assert!(
        name.to_lowercase().contains("test"),
        "refusing to write test data to the server {name:?}: run it with SERVER_NAME containing \"test\", or pass --allow-any-server"
    );
    let database = database_url.rsplit('/').next().unwrap_or_default();
    assert!(
        database_url.is_empty() || database.contains("test"),
        "refusing to use the database {database:?}: its name must contain \"test\", or pass --allow-any-server"
    );
}

/// The resident memory of a process, from `ps`, in megabytes.
pub fn resident_megabytes(pid: u32) -> Option<f64> {
    let output = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    let kilobytes: f64 = String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse()
        .ok()?;
    Some(kilobytes / 1024.0)
}

/// Samples a process's memory every 100 ms until stopped, keeping the peak.
pub struct MemoryWatch {
    stop: tokio::sync::oneshot::Sender<()>,
    peak: tokio::task::JoinHandle<f64>,
}

impl MemoryWatch {
    pub fn start(pid: u32) -> Self {
        let (stop, mut stopped) = tokio::sync::oneshot::channel();
        let peak = tokio::spawn(async move {
            let mut peak: f64 = 0.0;
            loop {
                if let Some(megabytes) = resident_megabytes(pid) {
                    peak = peak.max(megabytes);
                }
                tokio::select! {
                    _ = &mut stopped => return peak,
                    () = tokio::time::sleep(Duration::from_millis(100)) => {}
                }
            }
        });
        Self { stop, peak }
    }

    pub async fn peak_megabytes(self) -> f64 {
        // The watch ends either way: a send fails only if it already has.
        let _ = self.stop.send(());
        self.peak.await.unwrap_or(0.0)
    }
}
