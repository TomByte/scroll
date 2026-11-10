# scroll-server

Accounts, workspaces and live sync (page content and records) for Scroll. One binary; Postgres for storage. It
speaks plain HTTP and WebSocket, so put a TLS-terminating reverse proxy (such as Caddy) in front of it.

## Settings

Environment variables; an empty one counts as unset. The server reads them all in `src/config.rs`, where the
defaults below live. With Docker Compose, put them in `crates/scroll-server/.env`; the compose file passes them
on (the database URL and listen address are set for you there).

| Variable                    | Default            | Meaning                                                                       |
|-----------------------------|--------------------|-------------------------------------------------------------------------------|
| `DATABASE_URL`              | required           | Postgres connection string, e.g. `postgres://user@host/db`                    |
| `POSTGRES_PASSWORD`         | required (Compose) | Password the Compose Postgres is created with; keep it                        |
| `BIND_ADDR`                 | `127.0.0.1:8080`   | Address and port to listen on; the Docker image sets `0.0.0.0:8080`           |
| `SERVER_NAME`               | `Scroll`           | The name apps show for this server                                            |
| `SIGN_UP`                   | `invite`           | `invite`: an invite code is needed to make an account; `open`: anyone can     |
| `GOOGLE_CLIENT_ID`          | unset              | Google sign-in: the OAuth client's ID (set all three Google settings or none) |
| `GOOGLE_CLIENT_SECRET`      | unset              | Google sign-in: the OAuth client's secret                                     |
| `PUBLIC_URL`                | unset              | Google sign-in: the server's `https://` address as people reach it            |
| `DOMAIN`                    | unset (Compose)    | With the Compose `tls` profile: the domain Caddy gets a certificate for       |
| `CLIENT_IP_HEADER`          | unset              | Header the proxy puts the client address in (see below)                       |
| `PAGE_COMPACTION_THRESHOLD` | `1000`             | Updates after a page's snapshot before it is compacted                        |
| `RUST_LOG`                  | `info`             | Log filter, e.g. `scroll_server=debug`                                        |

Migrations run automatically on startup. A half-set Google configuration stops the server at startup.

### Sign-in and sign-up

People sign in with email and password, or with Google when it is set up; `GET /server-info` tells apps which
(`sign_in_methods`) and whether sign-up needs an invite (`sign_up`).

- Email addresses of password accounts are not verified: the server sends no email. On a server open to the
  public, keep `SIGN_UP=invite` until email verification exists, or anyone can register someone else's address.
- When Google proves an address that a password account already uses, that account becomes the Google
  account's: its password is cleared and its sessions end, so whoever registered the address first loses it.
- Google sign-in: in the Google Cloud console, create an OAuth client of type **Web application**, add the
  **Authorized redirect URI** `<PUBLIC_URL>/auth/google/callback` (for example
  `https://scroll.example.com/auth/google/callback`), and give the consent screen the scopes `openid`, `email`
  and `profile`. The app opens the sign-in in the system browser and receives the result on its own loopback
  address; nothing else needs registering.

### Client addresses and rate limits

Sign-in and sign-up attempts are limited in memory by each server process, with two rules (the numbers are in
`src/auth/rate_limit.rs`):

- Every attempt counts against the client's address.
- Only failed sign-ins count against an email, and only for the address they came from.

So guessing a password is slowed per address, but nobody can lock a user out by failing to sign in as them
from somewhere else: the user's own address is never refused because of other people's failures. Over a
limit the server answers `429` with `Retry-After`.

Behind a proxy, set `CLIENT_IP_HEADER` to the header the proxy appends the client's address to, for example
`x-forwarded-for` with Caddy. The server takes the **last** address in that header, because that is the one the
proxy added; anything before it was sent by the client and could be forged. Only set it when every request
reaches the server through the proxy. Without it, the server uses the TCP peer address.

### Page compaction

When a page has `PAGE_COMPACTION_THRESHOLD` updates after its snapshot, the server folds them into a new Loro
snapshot in the background, so clients opening the page download one snapshot instead of every update.

### Checking a server address

`GET /server-info` needs no account and tells an app it has found a Scroll server it can talk to:
`{"product": "scroll-server", "name": ..., "version": ..., "protocol_version": ..., "sign_up": "invite",
"sign_in_methods": ["password", "google"], "sign_up_requires_invite": true}` (the last kept for older apps).
It is limited per client address, more generously than sign-in.

## Self-hosting with Docker

From the repository root, with a domain pointing at the machine and ports 80 and 443 open:

```sh
printf 'POSTGRES_PASSWORD=%s\nDOMAIN=%s\n' "$(openssl rand -hex 24)" scroll.example.com > crates/scroll-server/.env
docker compose -f crates/scroll-server/docker-compose.yml --profile tls up -d --build
docker compose -f crates/scroll-server/docker-compose.yml exec scroll-server scroll-server create-invite
```

That runs Postgres, the server and Caddy, which gets a certificate for `DOMAIN` and serves the server at
`https://DOMAIN`. The last command prints a one-time invite code, which expires if unused. In the app, add the server
by its address, sign up with that code, then create or publish a workspace; owners invite everyone else from
the app. To change `SIGN_UP`, the Google settings (with `PUBLIC_URL=https://DOMAIN`) or `SERVER_NAME`, add
them to the same `.env` and run the `up` command again. Keep `.env` out of version control.

Behind a proxy of your own instead, leave out `--profile tls` and `DOMAIN`: the server then listens on
`127.0.0.1:8080` only, and `Caddyfile.example` shows a matching Caddy setup. Either way the server takes the
client's address from `X-Forwarded-For` (`CLIENT_IP_HEADER` in the compose file). `GET /healthz` answers
`200 ok` when the database is reachable and `503` otherwise.

Without Docker, the same invite comes from `DATABASE_URL=... scroll-server create-invite`.

## Run locally for development

```sh
createdb scroll_dev
export DATABASE_URL=postgres://postgres@localhost:5432/scroll_dev
cargo run -p scroll-server -- create-invite
cargo run -p scroll-server
```

The server stops cleanly on Ctrl-C or `SIGTERM`.

## Load and scale tools

Two tools in `examples/` drive a running server through its public API, so they also work against a hosted one.
They only need a sign-up invite. They write test data, so they refuse a server whose `SERVER_NAME` does not contain
"test" (and, for `--database-url`, a database whose name does not) unless given `--allow-any-server`.

```sh
SERVER_NAME="Scroll load test" DATABASE_URL=.../scroll_load_test scroll-server &
cargo run --release -p scroll-server --example load_test -- --url http://127.0.0.1:8080 \
    --invite "$(scroll-server create-invite)" --connections 300 --seconds 60
cargo run --release -p scroll-server --example scale_test -- --url http://127.0.0.1:8080 \
    --invite "$(scroll-server create-invite)" --database-url .../scroll_load_test
```

`load_test` opens many sync connections doing a realistic mix of typing, cell edits, presence and page switches,
and reports latency percentiles, throughput and errors. `scale_test` builds a workspace of 100,000 pages and
1,000,000 database rows and measures publishing, joining, edit latency under load and transactions per second.
The app's side at the same scale, with the real sync engine and workspace files, is
`cargo run --release -p scroll-sync --example engine_scale -- --url ... --invite ...`.

## How changes reach connections

A workspace's committed transactions go to every connection that has it open, in sequence order, but not all at
once: a transaction that only changes rows of one database wakes only the connections that have that database open
(`OpenDatabase`); the others receive it with their next delivery, within a second (`Settings::deferred_delivery`)
or just before the next change they need at once. Messages that queue up for a connection go out together in
`Batch` frames. Transactions pushed to a workspace at the same time commit together, sharing one lock and one
flush to disk, each still all or nothing; small ones take turns with bulk ones rather than waiting behind them.

## Tests

The integration tests start real servers against a `scroll_test` database:

```sh
createdb scroll_test
cargo test -p scroll-server
```

They connect to `postgres://postgres@localhost:5432/scroll_test` unless `SCROLL_TEST_DATABASE_URL` is set.
