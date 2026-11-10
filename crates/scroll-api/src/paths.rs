//! The server's endpoints. Paths with a parameter come as functions for callers and as route patterns for the
//! server's router.

pub const SERVER_INFO: &str = "/server-info";
pub const HEALTH: &str = "/healthz";
pub const ACCOUNTS: &str = "/accounts";
pub const SESSIONS: &str = "/sessions";
pub const CURRENT_SESSION: &str = "/sessions/current";
pub const SESSION_EXCHANGE: &str = "/sessions/exchange";
pub const WORKSPACES: &str = "/workspaces";
pub const WORKSPACE_INVITES_ROUTE: &str = "/workspaces/{workspace_id}/invites";
pub const ACCEPT_INVITE: &str = "/invites/accept";
pub const GOOGLE_START: &str = "/auth/google/start";
pub const GOOGLE_CALLBACK: &str = "/auth/google/callback";
pub const SYNC_SOCKET: &str = "/ws";

pub fn workspace_invites(workspace_id: scroll_core::id::WorkspaceId) -> String {
    format!("/workspaces/{}/invites", workspace_id.0)
}

/// The sync socket of the server at `server_url`: `https://host` becomes `wss://host/ws`. `None` for a URL that is
/// not `http` or `https`.
pub fn socket_url(server_url: &str) -> Option<String> {
    let server_url = server_url.trim_end_matches('/');
    let socket_base = if let Some(rest) = server_url.strip_prefix("https://") {
        format!("wss://{rest}")
    } else {
        format!("ws://{}", server_url.strip_prefix("http://")?)
    };
    Some(format!("{socket_base}{SYNC_SOCKET}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_urls_map_to_socket_urls() {
        assert_eq!(
            socket_url("https://scroll.example/").as_deref(),
            Some("wss://scroll.example/ws")
        );
        assert_eq!(
            socket_url("http://127.0.0.1:8080").as_deref(),
            Some("ws://127.0.0.1:8080/ws")
        );
        assert_eq!(socket_url("ftp://scroll.example"), None);
    }
}
