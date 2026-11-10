//! The one small tokio runtime that account requests and browser sign-ins run on, shared by every client in
//! the process, so their futures can be awaited from any executor, including GPUI's.

use std::sync::LazyLock;
use std::time::Duration;

use reqwest::{StatusCode, header};
use tokio::runtime::Runtime;
use tokio::task::JoinError;

/// Created on first use and kept for the life of the process: one thread for all servers.
static RUNTIME: LazyLock<Runtime> = LazyLock::new(|| {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .thread_name("scroll-accounts")
        .enable_all()
        .build()
        .expect("the process can start one more thread")
});

/// A response read whole.
pub(crate) struct Response {
    pub status: StatusCode,
    /// From `Retry-After`; one second when absent.
    pub retry_after: Duration,
    pub body: Vec<u8>,
}

/// Sends the request on the runtime and reads the whole response.
pub(crate) async fn send(
    request: reqwest::RequestBuilder,
) -> Result<Result<Response, reqwest::Error>, JoinError> {
    RUNTIME
        .spawn(async move {
            let response = request.send().await?;
            let status = response.status();
            let retry_after = retry_after(response.headers());
            let body = response.bytes().await?.to_vec();
            Ok(Response {
                status,
                retry_after,
                body,
            })
        })
        .await
}

/// Runs `work` on the runtime, and stops it if the returned future is dropped (the app cancelled).
pub(crate) async fn run_cancellable<T: Send + 'static>(
    work: impl Future<Output = T> + Send + 'static,
) -> Result<T, JoinError> {
    struct AbortOnDrop(tokio::task::AbortHandle);
    impl Drop for AbortOnDrop {
        fn drop(&mut self) {
            self.0.abort();
        }
    }
    let task = RUNTIME.spawn(work);
    let _abort = AbortOnDrop(task.abort_handle());
    task.await
}

fn retry_after(headers: &header::HeaderMap) -> Duration {
    let seconds = headers
        .get(header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse().ok())
        .unwrap_or(1);
    Duration::from_secs(seconds)
}
