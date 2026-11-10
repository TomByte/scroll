//! Sizes both ends rely on. The checks below fail the build if they stop fitting together.

/// The largest message the server accepts; it drops a connection that sends a larger one.
pub const MAX_MESSAGE_BYTES: usize = 8 * 1024 * 1024;

/// About this much encoded data goes in each part of a workspace catch-up.
pub const CATCH_UP_PART_BYTES: usize = 1024 * 1024;

/// Messages queued for a client go out gathered in `Batch` frames of about this size.
pub const BATCH_BYTES: usize = 1024 * 1024;

/// The largest frame the app accepts from the server. A relayed transaction can be as large as the largest
/// pushed message plus its envelope, and a catch-up part holds at least one whole record however large.
pub const MAX_SERVER_FRAME_BYTES: usize = 512 * 1024 * 1024;

/// The most databases one connection may have open (`ClientMessage::OpenDatabase`); the server rejects a
/// connection that opens more, so the client closes the one it opened longest ago to stay within it.
pub const MAX_OPEN_DATABASES: usize = 256;

const _: () = assert!(CATCH_UP_PART_BYTES < MAX_MESSAGE_BYTES);
const _: () = assert!(BATCH_BYTES < MAX_MESSAGE_BYTES);
const _: () = assert!(2 * MAX_MESSAGE_BYTES <= MAX_SERVER_FRAME_BYTES);
