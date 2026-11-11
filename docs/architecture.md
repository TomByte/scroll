# Scroll — architecture

Scroll is a Notion-like document and database app for macOS, written in Rust with GPUI. It works on its own with a local file, and can connect to any number of Scroll servers for team workspaces with live co-editing. The server is one binary: hosted centrally, or run by anyone on their own machine or network.

Status (2026-11-10): the local app, the server and live sync are built. Google sign-in has only been run against a fake provider. Billing is not built. Nothing has been released.

## Decisions

| Decision | Choice |
|---|---|
| UI framework | GPUI, the official `gpui` crate (0.2.2) |
| UI kit | Our own, in the `scroll-ui` crate |
| Client storage | SQLite (`rusqlite`, bundled), one file per workspace |
| Server | `axum` 0.8, one WebSocket per client, binary frames |
| Server storage | Postgres (`tokio-postgres` with a `deadpool-postgres` pool) |
| Page content | One Loro CRDT document per page (block tree and rich text) |
| Everything else | Plain records changed by operations, last write wins per field, ordered by the server |
| Collaboration | Team workspaces, live co-editing including two people in the same block |
| Local mode | The app is fully usable with no server and no account |
| Servers | The app connects to any number of Scroll servers, added by URL, each with its own account; anyone can host one |
| Sign-in | Email and password, or Google; the app never asks for a token |
| Hosting | One central paid server run by us, and free self-hosting from the same binary (Dockerfile and Compose file in `crates/scroll-server`) |
| Scale target | 1,000,000 database rows per workspace, 500,000 in one database, 100,000 pages, at 120 Hz |

We write our own UI kit because `gpui-component` 0.9.1 does not build against the official `gpui` crate: it pins `gpui-pre` at exactly 0.3.12, a third-party republish of unreleased GPUI.

The server uses `tokio-postgres` rather than `sqlx` because `sqlx` pulls in a second SQLite link that conflicts with `rusqlite` in `scroll-core`.

## What we take from Notion

- **Everything is a block, and pages are blocks.** Kept. A database row is a page with property values.
- **Edits are operations batched into transactions.** The client applies them locally, queues them in SQLite, then sends them; the server validates and commits all or nothing. Kept for records.
- **Clients subscribe to records over a WebSocket.** Notion's socket only announces new versions and the client then fetches them. Changed: our socket carries the change itself.
- **Children stored as an ordered array on the parent.** Changed: each page stores its parent and a position, so a move is one field write and concurrent inserts cannot conflict.
- **Last write wins for text.** Notion later moved offline pages to a CRDT model. We start with a CRDT for page content.

## Data model

Two kinds of data, with different conflict rules.

**Page content (CRDT).** Each page has one Loro document holding its block tree and each block's rich text with marks. Concurrent typing, formatting and block moves merge without a server decision. Loro also supplies stable cursors and per-user undo.

**Records (operations).** Everything that needs querying or permission checks. The record kinds are:

- `page`: parent page, title, icon, position, archived flag; a row also names the database it belongs to and carries its property values.
- `database`: the page that hosts it and its property schema (name, type, options per property).
- `view`: database, name, filter, sort, visible columns and column widths.
- `tag`: a workspace-level name and colour.
- `page_tag`: one record per (page, tag), so concurrent tagging merges.

A record change is an operation: create, set a field, or delete. Operations are grouped into a transaction. The server gives every committed transaction the next sequence number for its workspace; the higher sequence wins per field. The client keeps the last server-confirmed state of each record it has pending changes for, so a rejected transaction rolls back locally.

IDs are UUIDv7 generated on the client. Sibling order uses fractional index strings.

### Workspace file (SQLite)

The app's local store, one file per workspace, with a small settings file beside it (last page, expanded pages).

| Tables | Purpose |
|---|---|
| `page`, `database`, `database_property`, `property_value`, `view`, `tag`, `page_tag` | Current record state |
| `page_doc`, `page_update` | Latest Loro snapshot per page, and updates since |
| `tx_queue`, `queued_record`, `confirmed_record` | Changes not yet acknowledged, and the server-confirmed state they sit on |
| `staged_snapshot`, `staged_record`, `staged_value` | A first download being received and applied in resumable batches |
| `sync_state`, `workspace` | The link to a server and the workspace's name |
| `search` | FTS5 index over titles, block text and tag names |

The table view opens its own read and write connections to this file beside the app's storage thread; SQLite's immediate transactions and a short busy retry keep them correct.

### Server (Postgres)

The server keeps the same records in its own tables keyed by workspace (`pages`, `databases`, `views`, `tags`, `page_tags`, `property_values`), page content in `pages` and `page_updates`, the committed log in `record_transactions`, and accounts in `users`, `identities`, `sessions`, `memberships`, `invites`, `workspaces`, plus short-lived browser sign-in state.

## Sync

The app always reads and writes its local file. In standalone mode that is the whole story. When a workspace is linked to a server, the sync engine runs alongside.

1. **Local edit.** Apply and repaint in the same frame; write to SQLite and to the queue.
2. **Push.** Send the queue in order. The server checks membership and role, validates, commits waiting transactions together, assigns sequence numbers and acknowledges.
3. **Fan out.** Page content is relayed at once to everyone with the page open. Record changes go at once to connections that have the affected database open (the app tells the server which tables and row pages are on screen); everyone else receives them within about a second, batched, in the same order.
4. **Catch up.** On connect the client sends its last seen sequences. The server replies with the log since then, or, for a first join or a long absence, a snapshot sent in parts that the client stages and applies in short transactions.
5. **Presence.** Cursors, selections and names travel over the same socket and are never stored.

For page content the server does not interpret updates: it checks the sender may edit, appends the bytes, relays them, and periodically folds updates into a new snapshot. For records the server applies the same pure rules as the client (`scroll-core`'s validation), with its own lookups in Postgres.

**Transport.** WebSocket over TLS with binary frames, encoded with `postcard`. HTTPS is used for server info, sign-up, sign-in, workspaces and invites.

**Accounts and permissions.** People sign in with email and password (Argon2id) or with Google. Google sign-in opens the system browser; the server holds the Google client secret and hands the app a one-time code on a loopback address. Either way the app receives a session token, stored in the macOS Keychain and never shown. A server offers Google only when its host configures it. Sign-up is invite only or open, set per server. Workspace roles: owner, editor, viewer. TLS is terminated by a reverse proxy such as Caddy.

**Local and server workspaces.** A workspace is either local or linked to one server. Publishing a local workspace sends its records and page documents through the normal sync path.

## Code layout

One Cargo workspace, nine crates. Dependencies point one way. `scroll-core` is used by every crate except `scroll-ui` and `scroll-settings`. `scroll-api` is used by `scroll-server` and `scroll-sync`. `scroll-ui` is used by `scroll-table` and `scroll-app`. `scroll-app` uses `scroll-table`, `scroll-sync`, `scroll-settings` and `scroll-mcp`, and never the server (only `scroll-sync`'s tests start one).

- `scroll-core` — record kinds, operations, validation rules, the SQLite store, Loro page documents, search. No GPUI and no networking. Everything about a record kind sits in `record/`, `validation/` and `store/kinds/`.
- `scroll-api` — everything on the wire: HTTP request and response types, error codes, paths, the WebSocket protocol, size limits and connection timing. Used by the server, the client, tools and tests.
- `scroll-ui` — the GPUI component kit: theme tokens and components. Knows nothing about Scroll's data. Adding a component is one file (component, gallery story, test), a line in its group's `mod.rs`, one export line and one line in the gallery; the recipe is at the top of `src/lib.rs`. `cargo run -p scroll-ui --example gallery` shows every component in light and dark. Every animation goes through `motion/`: an entrance (`Motion`) or a change (`Transition`), with durations and easings in one place and nothing animated under reduced motion.
- `scroll-settings` — preferences: a typed `Setting` declared as a `const` beside the code that uses it, kept in `preferences.json`, and listed on the settings page by category. GPUI only; the recipe is at the top of `src/lib.rs`.
- `scroll-mcp` — the Model Context Protocol server AI tools use to read, search and change the open workspace. No GPUI. It runs inside the app while its setting is on, on its own thread with its own connection to the workspace file (as the table view and sync engine have), listens on 127.0.0.1 only and requires a token made at runtime and kept in the data directory. Its writes go through `apply_local_transaction` and `record_local_page_update`, and are reported to the app as events so the window and the sync engine follow.
- `scroll-table` — the database table view and the one home of property-value display and parsing (`values`).
- `scroll-sync` — the client: account calls per server, browser sign-in, and the engine that syncs one workspace file. No GPUI.
- `scroll-server` — the axum server, by feature: `auth/`, `workspaces/`, `http/`, `pages/`, `records/`, `realtime/`; every setting is read in `config.rs`.
- `scroll-app` — the app, by feature: `app/`, `window/`, `overlays/`, `accounts/`, `workspace/`, `sidebar/`, `page/`, `database/`, `editor/`, `commands/`, `palette/`, `settings/` (the settings page), `mcp/` (starting and stopping the MCP server), plus a few single-purpose files at the root (sync status wording, title-bar and page layout sizes, JSON file load and save, logging). Each feature declares its own commands; one registry feeds the palette, the menu bar, page menus and key bindings.

Tools: `scroll-core` examples `fixture` and `scale`; `scroll-table` example `table`; `scroll-app` examples `scale_probe`, `editor_perf` and `demo` (the README's GIF, recorded by `scripts/record-demo.sh`); `scroll-server` examples `load_test` and `scale_test`; `scroll-mcp` example `serve`; `scroll-sync` example `engine_scale`.

## Performance targets

- Cold start to a first frame: under 200 ms.
- Keystroke to paint: within one frame at 120 Hz.
- Smooth scrolling on a 500,000-row table.
- Remote edit visible to someone viewing the same page or table on the same network: under 50 ms.

## Risks and limits

- **Own UI kit and block editor.** Every component, including text input with IME, is ours. Text selection is within a block plus whole-block selection.
- **GPUI is pre-1.0.** Releases on crates.io lag Zed's main branch and can break APIs. We pin the version and upgrade deliberately. GPUI shapes one font size per line.
- **One server process.** Relays, rate limits and live sessions are in memory. Running several processes behind a load balancer needs Postgres `LISTEN/NOTIFY` or similar; a design exists, nothing is built.
- **Merging blocks** can lose a peer's concurrent typing in the merged block. The fix is one rich-text container per page, a stored-format change.

## Open questions

- The central server needs open sign-up, which needs email verification for password accounts (not built).
- Paid plans on the central server: design only (plans per workspace, limits enforced in the server, a billing webhook); no billing code or plan data exists.
- Should local workspaces be encrypted at rest? Assumed no.
- Later features: relations, rollups and formulas; board, calendar and gallery views; images and files; per-page sharing; version history; import and export; a signed app bundle; other platforms.
