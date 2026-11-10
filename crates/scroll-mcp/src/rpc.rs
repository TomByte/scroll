//! JSON-RPC 2.0 and the few MCP methods the server answers: `initialize`, `ping`, `tools/list`
//! and `tools/call`. Notifications (such as `notifications/initialized`) need no answer.

use serde::Deserialize;
use serde_json::{Value, json};

use crate::tools::{self, ToolError, Workspace};

/// Newest first; a client asking for another version is offered the newest.
const PROTOCOL_VERSIONS: [&str; 2] = ["2025-06-18", "2025-03-26"];

const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;

/// What goes back over HTTP.
pub(crate) enum Reply {
    /// 202 and no body: a notification, or a client's response to us.
    Accepted,
    /// 200 with a JSON-RPC response.
    Answer(Value),
    /// 400 with a JSON-RPC error: the body was not a JSON-RPC message at all.
    Unreadable(Value),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Method {
    Initialize,
    Ping,
    ListTools,
    CallTool,
}

impl Method {
    fn from_name(name: &str) -> Option<Method> {
        match name {
            "initialize" => Some(Method::Initialize),
            "ping" => Some(Method::Ping),
            "tools/list" => Some(Method::ListTools),
            "tools/call" => Some(Method::CallTool),
            _ => None,
        }
    }
}

#[derive(Deserialize)]
struct CallParams {
    name: String,
    #[serde(default)]
    arguments: Value,
}

pub(crate) fn handle(workspace: &Workspace, body: &[u8]) -> Reply {
    let message: Value = match serde_json::from_slice(body) {
        Ok(message) => message,
        Err(error) => {
            return Reply::Unreadable(error_response(
                &Value::Null,
                PARSE_ERROR,
                &error.to_string(),
            ));
        }
    };
    let Some(message) = message.as_object() else {
        return Reply::Unreadable(error_response(
            &Value::Null,
            INVALID_REQUEST,
            "expected one JSON-RPC message as an object",
        ));
    };
    let id = message.get("id").cloned().unwrap_or(Value::Null);
    if message.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Reply::Unreadable(error_response(
            &id,
            INVALID_REQUEST,
            "jsonrpc must be \"2.0\"",
        ));
    }
    let Some(name) = message.get("method").and_then(Value::as_str) else {
        return Reply::Accepted;
    };
    if !message.contains_key("id") {
        return Reply::Accepted;
    }
    let Some(method) = Method::from_name(name) else {
        let reason = format!("there is no method called {name}");
        return Reply::Answer(error_response(&id, METHOD_NOT_FOUND, &reason));
    };
    let params = message.get("params").cloned().unwrap_or(Value::Null);
    Reply::Answer(match answer(workspace, method, params) {
        Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
        Err(reason) => error_response(&id, INVALID_PARAMS, &reason),
    })
}

/// The method's result, or why its parameters are unusable.
fn answer(workspace: &Workspace, method: Method, params: Value) -> Result<Value, String> {
    match method {
        Method::Initialize => Ok(initialize(&params)),
        Method::Ping => Ok(json!({})),
        Method::ListTools => Ok(json!({"tools": tools::definitions(workspace.access())})),
        Method::CallTool => {
            let call: CallParams =
                serde_json::from_value(params).map_err(|error| error.to_string())?;
            call_tool(workspace, call)
        }
    }
}

fn initialize(params: &Value) -> Value {
    let requested = params.get("protocolVersion").and_then(Value::as_str);
    let version = PROTOCOL_VERSIONS
        .into_iter()
        .find(|supported| Some(*supported) == requested)
        .unwrap_or(PROTOCOL_VERSIONS[0]);
    json!({
        "protocolVersion": version,
        "capabilities": {"tools": {}},
        "serverInfo": {"name": "scroll", "version": env!("CARGO_PKG_VERSION")},
    })
}

/// A tool that fails says why in its result, for the model to read; only a tool that does not
/// exist is a protocol error.
fn call_tool(workspace: &Workspace, call: CallParams) -> Result<Value, String> {
    let (text, is_error) = match tools::call(workspace, &call.name, call.arguments) {
        Ok(result) => (pretty(&result), false),
        Err(error @ ToolError::UnknownTool(_)) => return Err(error.to_string()),
        Err(error) => {
            log::debug!("MCP tool {} failed: {error}", call.name);
            (error.to_string(), true)
        }
    };
    Ok(json!({"content": [{"type": "text", "text": text}], "isError": is_error}))
}

fn pretty(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
}

fn error_response(id: &Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}
