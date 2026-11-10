//! How a sync connection is kept alive, and what the server's close codes tell the client to do.

use std::time::Duration;

/// The server pings this often; clients answer pings on their own.
pub const PING_INTERVAL: Duration = Duration::from_secs(30);

/// Either side treats a connection this long without any frame as dead.
pub const SILENCE_LIMIT: Duration = Duration::from_secs(75);

/// The close code ("try again later", 1013) by which the server says: reconnect at once and catch up.
pub const RECONNECT_PROMPTLY: u16 = 1013;

const _: () = assert!(SILENCE_LIMIT.as_secs() > 2 * PING_INTERVAL.as_secs());
