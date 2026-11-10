//! The real server on a free port, against a workspace file of its own, spoken to over plain
//! HTTP/1.1 as an MCP client would.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use scroll_core::id::{DatabaseId, PageId, PropertyId, SelectOptionId, ViewId, WorkspaceId};
use scroll_core::operation::{Operation, Record, RecordId, Transaction};
use scroll_core::position::Position;
use scroll_core::record::{Database, Page, Property, PropertyType, SelectOption, View, ViewType};
use scroll_core::store::WorkspaceStore;
use scroll_mcp::{Access, McpConfig, McpEvent, McpEvents, McpServer, StartError, generate_token};
use serde_json::{Value, json};

struct Running {
    server: Option<McpServer>,
    events: McpEvents,
    token: String,
    path: PathBuf,
}

impl Running {
    fn start(access: Access) -> Running {
        Running::start_at(temporary_path(), access)
    }

    fn start_at(path: PathBuf, access: Access) -> Running {
        let token = generate_token().unwrap();
        let (server, events) = McpServer::start(McpConfig {
            workspace_path: path.clone(),
            port: 0,
            token: token.clone(),
            access,
        })
        .unwrap();
        Running {
            server: Some(server),
            events,
            token,
            path,
        }
    }

    fn port(&self) -> u16 {
        self.server.as_ref().unwrap().address().port()
    }

    /// Sends `body` with the token and the headers an MCP client sends.
    fn post(&self, body: &Value) -> (u16, String) {
        let authorization = format!("Bearer {}", self.token);
        self.request(
            "POST",
            &[("Authorization", &authorization)],
            &body.to_string(),
        )
    }

    fn request(&self, method: &str, headers: &[(&str, &str)], body: &str) -> (u16, String) {
        let host = format!("127.0.0.1:{}", self.port());
        let mut all_headers = vec![
            ("Host", host.as_str()),
            ("Content-Type", "application/json"),
            ("Accept", "application/json, text/event-stream"),
        ];
        all_headers.extend_from_slice(headers);
        raw_request(self.port(), method, &all_headers, body)
    }

    fn rpc(&self, method: &str, params: Value) -> Value {
        let (status, body) =
            self.post(&json!({"jsonrpc": "2.0", "id": 7, "method": method, "params": params}));
        assert_eq!(status, 200, "{body}");
        let answer: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(answer["id"], json!(7));
        answer
    }

    /// The tool's result, parsed; fails the test if the tool failed.
    fn call(&self, tool: &str, arguments: Value) -> Value {
        let result =
            &self.rpc("tools/call", json!({"name": tool, "arguments": arguments}))["result"];
        let text = result["content"][0]["text"].as_str().unwrap();
        assert_eq!(result["isError"], json!(false), "{tool} failed: {text}");
        serde_json::from_str(text).unwrap()
    }

    /// Why the tool failed; fails the test if it did not.
    fn call_failing(&self, tool: &str, arguments: Value) -> String {
        let result =
            &self.rpc("tools/call", json!({"name": tool, "arguments": arguments}))["result"];
        assert_eq!(result["isError"], json!(true), "{tool} should have failed");
        result["content"][0]["text"].as_str().unwrap().to_string()
    }

    fn next_event(&mut self) -> McpEvent {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        let next = runtime.block_on(async {
            tokio::time::timeout(Duration::from_secs(5), self.events.next()).await
        });
        next.unwrap().unwrap()
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        if let Some(server) = self.server.take() {
            server.shut_down();
        }
        for suffix in ["", "-wal", "-shm"] {
            let mut path = self.path.clone().into_os_string();
            path.push(suffix);
            std::fs::remove_file(path).ok();
        }
    }
}

fn temporary_path() -> PathBuf {
    std::env::temp_dir().join(format!("scroll-mcp-{}.sqlite", PageId::generate().0))
}

fn raw_request(port: u16, method: &str, headers: &[(&str, &str)], body: &str) -> (u16, String) {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    let mut request = format!("{method} /mcp HTTP/1.1\r\n");
    for (name, value) in headers {
        request.push_str(&format!("{name}: {value}\r\n"));
    }
    request.push_str(&format!(
        "Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    ));
    stream.write_all(request.as_bytes()).unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    let status = response[9..12].parse().unwrap();
    let body = response
        .split_once("\r\n\r\n")
        .map_or(String::new(), |(_, body)| body.to_string());
    (status, body)
}

fn id(value: &Value) -> String {
    value.as_str().unwrap().to_string()
}

/// A "Tasks" page hosting a database with a select, a number and a date, written by another
/// connection as the app would.
fn tasks_database(path: &Path) -> (PageId, DatabaseId) {
    let mut store = WorkspaceStore::open(path).unwrap();
    let host = Page {
        id: PageId::generate(),
        parent: None,
        title: "Tasks".into(),
        icon: None,
        position: Position::between(None, None).unwrap(),
        row_of: None,
        is_archived: false,
    };
    let option = |name: &str| SelectOption {
        id: SelectOptionId::generate(),
        name: name.into(),
    };
    let property = |name: &str, property_type, options| Property {
        id: PropertyId::generate(),
        name: name.into(),
        property_type,
        options,
    };
    let database = Database {
        id: DatabaseId::generate(),
        host_page: host.id,
        properties: vec![
            property(
                "Status",
                PropertyType::Select,
                vec![option("Todo"), option("Done")],
            ),
            property("Estimate", PropertyType::Number, Vec::new()),
            property("Due", PropertyType::Date, Vec::new()),
        ],
    };
    let view = View {
        id: ViewId::generate(),
        database: database.id,
        name: "All".into(),
        view_type: ViewType::Table,
        filter: Vec::new(),
        sort: Vec::new(),
        visible_columns: database
            .properties
            .iter()
            .map(|property| property.id)
            .collect(),
        column_widths: Vec::new(),
    };
    let ids = (host.id, database.id);
    store
        .apply_local_transaction(&Transaction::new(vec![
            Operation::Create(Record::Page(host)),
            Operation::Create(Record::Database(database)),
            Operation::Create(Record::View(view)),
        ]))
        .unwrap();
    ids
}

#[test]
fn a_client_reads_writes_and_queries_a_workspace() {
    let mut running = Running::start(Access::ReadWrite);

    let initialized = running.rpc(
        "initialize",
        json!({"protocolVersion": "2025-03-26", "capabilities": {}, "clientInfo": {"name": "test", "version": "1"}}),
    );
    assert_eq!(
        initialized["result"]["protocolVersion"],
        json!("2025-03-26")
    );
    assert_eq!(initialized["result"]["capabilities"], json!({"tools": {}}));
    let (status, body) =
        running.post(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
    assert_eq!((status, body.as_str()), (202, ""));
    let listed = running.rpc("tools/list", json!({}));
    let names: Vec<&str> = listed["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "list_pages",
            "get_page",
            "search",
            "query_database",
            "create_page",
            "update_page",
            "append_blocks",
            "update_block",
            "delete_block",
            "create_row",
            "update_row"
        ]
    );

    // A page with content, reported to the app as a record change and a content update.
    let created = running.call(
        "create_page",
        json!({
            "title": "Garden plan",
            "icon": "🌱",
            "blocks": [
                {"kind": "heading_1", "text": "Beds"},
                {"kind": "to_do", "text": "Water tomatoes"},
            ],
        }),
    );
    let page = id(&created["page_id"]);
    let page_id = PageId(page.parse().unwrap());
    let (heading, to_do) = (id(&created["block_ids"][0]), id(&created["block_ids"][1]));
    match running.next_event() {
        McpEvent::RecordsChanged(changes) => {
            assert_eq!(changes.records, vec![RecordId::Page(page_id)]);
            assert_eq!(changes.child_lists, vec![None]);
        }
        other => panic!("expected the new page's record, got {other:?}"),
    }
    let McpEvent::PageContentChanged {
        page_id: changed,
        update,
    } = running.next_event()
    else {
        panic!("expected the new page's content");
    };
    assert_eq!(changed, page_id);
    assert!(!update.is_empty());

    let appended = running.call(
        "append_blocks",
        json!({"page_id": page, "blocks": [{"text": "Basil by the door"}], "after_block_id": heading}),
    );
    assert_eq!(appended["block_ids"].as_array().unwrap().len(), 1);
    running.call(
        "update_block",
        json!({"page_id": page, "block_id": to_do, "text": "Water tomatoes daily", "checked": true}),
    );

    let read = running.call("get_page", json!({"page_id": page}));
    assert_eq!(read["title"], json!("Garden plan"));
    assert_eq!(read["icon"], json!("🌱"));
    let blocks: Vec<(&str, &str)> = read["blocks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|block| {
            (
                block["kind"].as_str().unwrap(),
                block["text"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        blocks,
        [
            ("heading_1", "Beds"),
            ("paragraph", "Basil by the door"),
            ("to_do", "Water tomatoes daily")
        ]
    );
    assert_eq!(read["blocks"][2]["checked"], json!(true));

    // Another connection, as the app's, sees the content the server wrote.
    let app_store = WorkspaceStore::open(&running.path).unwrap();
    let texts: Vec<String> = app_store
        .load_page_doc(page_id)
        .unwrap()
        .blocks()
        .unwrap()
        .iter()
        .map(|block| block.text.clone())
        .collect();
    assert_eq!(texts, ["Beds", "Basil by the door", "Water tomatoes daily"]);

    let found = running.call("search", json!({"query": "basil"}));
    assert_eq!(found["results"][0]["page_id"], json!(page));

    // A database made by the app, queried and filled in by the client.
    let (tasks, database) = tasks_database(&running.path);
    let listed = running.call("list_pages", json!({}));
    let titles: Vec<&str> = listed["pages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|page| page["title"].as_str().unwrap())
        .collect();
    assert_eq!(titles, ["Garden plan", "Tasks"]);
    assert_eq!(
        listed["pages"][1]["database_id"],
        json!(database.0.to_string())
    );
    let schema = running.call("get_page", json!({"page_id": tasks.0.to_string()}));
    assert_eq!(
        schema["database"]["properties"][0]["options"],
        json!(["Todo", "Done"])
    );

    for (title, status, estimate) in [("Dig", "Todo", 3), ("Sow", "Done", 1), ("Weed", "Todo", 5)] {
        running.call(
            "create_row",
            json!({
                "database_id": database.0.to_string(),
                "title": title,
                "values": {"Status": status, "Estimate": estimate, "Due": "2026-03-01"},
            }),
        );
    }
    let todo = running.call(
        "query_database",
        json!({
            "database_id": database.0.to_string(),
            "filter": [{"property": "status", "operator": "equals", "value": "Todo"}],
            "sort": [{"property": "Estimate", "direction": "descending"}],
        }),
    );
    assert_eq!(todo["total"], json!(2));
    let rows = todo["rows"].as_array().unwrap();
    assert_eq!(rows[0]["title"], json!("Weed"));
    assert_eq!(
        rows[0]["values"],
        json!({"Status": "Todo", "Estimate": 5.0, "Due": "2026-03-01"})
    );
    assert_eq!(rows[1]["title"], json!("Dig"));

    running.call(
        "update_row",
        json!({"page_id": rows[0]["id"], "values": {"Status": "Done", "Due": null}}),
    );
    let row = running.call("get_page", json!({"page_id": rows[0]["id"]}));
    assert_eq!(
        row["row"]["values"],
        json!({"Status": "Done", "Estimate": 5.0, "Due": null})
    );
    let still_todo = running.call(
        "query_database",
        json!({
            "database_id": database.0.to_string(),
            "filter": [{"property": "Status", "operator": "equals", "value": "Todo"}],
            "limit": 1,
        }),
    );
    assert_eq!(still_todo["total"], json!(1));
    assert_eq!(still_todo["rows"][0]["title"], json!("Dig"));

    let refused = running.call_failing(
        "create_row",
        json!({"database_id": database.0.to_string(), "values": {"Status": "Blocked"}}),
    );
    assert_eq!(
        refused,
        "\"Status\" has no option called \"Blocked\"; its options are: Todo, Done"
    );

    running.call(
        "delete_block",
        json!({"page_id": page, "block_id": heading}),
    );
    running.call("update_page", json!({"page_id": page, "archived": true}));
    let blocks = running.call("get_page", json!({"page_id": page}))["blocks"].clone();
    assert_eq!(blocks.as_array().unwrap().len(), 2);
    let listed = running.call("list_pages", json!({}));
    assert_eq!(listed["pages"].as_array().unwrap().len(), 1);
}

#[test]
fn requests_without_the_token_or_from_a_browser_are_refused() {
    let running = Running::start(Access::ReadWrite);
    let ping = json!({"jsonrpc": "2.0", "id": 1, "method": "ping"}).to_string();
    let bearer = format!("Bearer {}", running.token);

    assert_eq!(running.request("POST", &[], &ping).0, 401);
    assert_eq!(
        running
            .request("POST", &[("Authorization", "Bearer not-the-token")], &ping)
            .0,
        401
    );
    assert_eq!(
        running
            .request(
                "POST",
                &[
                    ("Authorization", &bearer),
                    ("Origin", "https://evil.example")
                ],
                &ping
            )
            .0,
        403
    );
    let rebound = raw_request(
        running.port(),
        "POST",
        &[("Host", "evil.example"), ("Authorization", &bearer)],
        &ping,
    );
    assert_eq!(rebound.0, 403);
    assert_eq!(
        running.request("GET", &[("Authorization", &bearer)], "").0,
        405
    );
    assert_eq!(
        running
            .request("POST", &[("Authorization", &bearer)], &ping)
            .0,
        200
    );
}

#[test]
fn a_read_only_server_offers_and_runs_no_tool_that_writes() {
    let running = Running::start(Access::ReadOnly);

    let listed = running.rpc("tools/list", json!({}));
    let names: Vec<&str> = listed["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        ["list_pages", "get_page", "search", "query_database"]
    );
    assert_eq!(
        running.call_failing("create_page", json!({"title": "Sneaky"})),
        "this workspace is read-only for AI tools"
    );
    assert_eq!(running.call("list_pages", json!({}))["pages"], json!([]));
}

#[test]
fn malformed_messages_get_json_rpc_errors() {
    let running = Running::start(Access::ReadWrite);
    let error_code = |(status, body): (u16, String)| {
        let answer: Value = serde_json::from_str(&body).unwrap();
        (status, answer["error"]["code"].as_i64().unwrap())
    };
    let bearer = format!("Bearer {}", running.token);
    let send = |body: &str| running.request("POST", &[("Authorization", &bearer)], body);

    assert_eq!(error_code(send("{not json")), (400, -32700));
    assert_eq!(error_code(send("[]")), (400, -32600));
    assert_eq!(
        error_code(send(r#"{"id": 1, "method": "ping"}"#)),
        (400, -32600)
    );
    assert_eq!(
        error_code(running.post(&json!({"jsonrpc": "2.0", "id": 1, "method": "resources/list"}))),
        (200, -32601)
    );
    assert_eq!(
        error_code(running.post(&json!({
            "jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": {"name": "drop_tables", "arguments": {}}
        }))),
        (200, -32602)
    );
    assert!(
        running
            .call_failing("get_page", json!({"page": "typo"}))
            .starts_with("the arguments do not fit the tool's schema")
    );
    let offered = running.rpc("initialize", json!({"protocolVersion": "1999-01-01"}));
    assert_eq!(offered["result"]["protocolVersion"], json!("2025-06-18"));
}

#[test]
fn a_port_in_use_is_reported_not_fatal() {
    let running = Running::start(Access::ReadWrite);
    let second = McpServer::start(McpConfig {
        workspace_path: running.path.clone(),
        port: running.port(),
        token: generate_token().unwrap(),
        access: Access::ReadWrite,
    });
    assert!(matches!(second, Err(StartError::Listen { .. })));
}

#[test]
fn in_a_linked_workspace_every_write_waits_for_the_server_and_is_searchable_meanwhile() {
    let path = temporary_path();
    WorkspaceStore::open(&path)
        .unwrap()
        .link_to_server("https://scroll.example", WorkspaceId::generate())
        .unwrap();
    let running = Running::start_at(path, Access::ReadWrite);

    running.call(
        "create_page",
        json!({"title": "Shared", "blocks": [{"text": "compost rota"}]}),
    );

    let app_store = WorkspaceStore::open(&running.path).unwrap();
    assert_eq!(app_store.queued_change_count().unwrap(), 2);
    let found = running.call("search", json!({"query": "compost"}));
    assert_eq!(found["results"][0]["title"], json!("Shared"));
}
