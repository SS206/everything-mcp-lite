//! Tests mirroring the TS suite: 8 pure query-builder + 8 live MCP integration
//! (in-process duplex transport).
use rmcp::model::{CallToolRequestParams, CallToolResult, ContentBlock};
use rmcp::{ServiceExt, service::RunningService};
use serde_json::{Value, json};

use everything_mcp_lite::server::{EverythingTools, build_recent_query};

// ---------- A. pure query-builder (no Everything dependency) ----------

#[test]
fn period_today_is_a_literal() {
    assert_eq!(build_recent_query("today", None, None).unwrap(), "dm:today");
}

#[test]
fn fixed_periods_use_a_computed_dm_boundary() {
    let q = build_recent_query("1hour", None, None).unwrap();
    assert!(q.starts_with("dm:>"));
}

#[test]
fn invalid_period_raises_with_options_listed() {
    let err = build_recent_query("decade", None, None).unwrap_err();
    assert!(err.contains("1min"));
}

#[test]
fn path_with_spaces_is_trimmed_and_quoted() {
    let q = build_recent_query("1week", Some(" C:\\Program Files "), None).unwrap();
    assert!(q.contains("path:\"C:\\Program Files\""));
}

#[test]
fn blank_path_is_ignored() {
    let q = build_recent_query("1week", Some("   "), None).unwrap();
    assert!(!q.contains("path:"));
}

#[test]
fn path_with_a_quote_is_rejected() {
    assert!(build_recent_query("1week", Some("C:\\Has\"Quote"), None).is_err());
}

#[test]
fn extensions_are_normalized_to_semicolons() {
    let q = build_recent_query("1week", None, Some(" .py , md ,")).unwrap();
    assert!(q.contains("ext:py;md"));
}

#[test]
fn invalid_extensions_are_rejected() {
    assert!(build_recent_query("1week", None, Some("py, m d")).is_err());
}

// ---------- B. live MCP integration (in-process duplex transport) ----------

fn extract_text(result: &CallToolResult) -> String {
    for block in &result.content {
        if let ContentBlock::Text(t) = block {
            return t.text.clone();
        }
    }
    String::new()
}

type TestClient = RunningService<rmcp::service::RoleClient, ()>;

async fn call_tool(tool: &str, args: Value) -> String {
    let (server_transport, client_transport) = tokio::io::duplex(4096);
    let server_handle: tokio::task::JoinHandle<Result<(), Box<dyn std::error::Error + Send + Sync>>> =
        tokio::spawn(async move {
            let service = EverythingTools
                .serve(server_transport)
                .await
                .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { Box::new(e) })?;
            service
                .waiting()
                .await
                .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { Box::new(e) })?;
            Ok(())
        });

    let client: TestClient = ().serve(client_transport).await.expect("client serve");
    let params =
        CallToolRequestParams::new(tool.to_string()).with_arguments(
            args.as_object().cloned().unwrap_or_default(),
        );
    let result = client.call_tool(params).await;
    let _ = client.cancel().await;
    let _ = server_handle.await;
    match result {
        Ok(r) => extract_text(&r),
        Err(e) => format!("ServiceError: {e}"),
    }
}

#[tokio::test]
async fn search_basic_returns_tab_separated_lines() {
    let out = call_tool("search", json!({ "query": "*.py", "top_n": 5 })).await;
    assert!(!out.is_empty());
    assert!(out.contains('\t'));
    let lines: Vec<&str> = out.trim().split('\n').collect();
    assert!(lines.len() <= 5);
}

#[tokio::test]
async fn search_include_size_yields_three_columns() {
    let out = call_tool(
        "search",
        json!({ "query": "*.py", "top_n": 3, "include_size": true }),
    )
    .await;
    assert!(!out.is_empty());
    for line in out.trim().split('\n') {
        assert_eq!(line.split('\t').count(), 3);
    }
}

#[tokio::test]
async fn empty_query_returns_error_line() {
    let out = call_tool("search", json!({ "query": "" })).await;
    assert!(out.contains("Error"));
}

#[tokio::test]
async fn search_recent_with_defaults_returns_output() {
    let out = call_tool("search_recent", json!({})).await;
    assert!(!out.is_empty());
}

#[tokio::test]
async fn invalid_period_returns_error_mentioning_1min() {
    let out = call_tool("search_recent", json!({ "period": "decade" })).await;
    assert!(out.contains("Error"));
    assert!(out.contains("1min"));
}

#[tokio::test]
async fn search_recent_with_path_and_extensions() {
    let home = std::env::var("USERPROFILE").unwrap_or_default();
    let out = call_tool(
        "search_recent",
        json!({ "period": "1week", "path": home, "extensions": "py,md" }),
    )
    .await;
    assert!(!out.is_empty());
    assert!(!out.contains("Error"));
    assert!(out.contains('\t'));
}

#[tokio::test]
async fn zero_size_files_keep_three_columns_with_size_0() {
    let out = call_tool(
        "search",
        json!({ "query": "size:0", "top_n": 5, "include_size": true }),
    )
    .await;
    assert!(!out.contains("Error"));
    assert!(!out.trim().is_empty());
    assert_ne!(out.trim(), "No matching files found");
    for line in out.trim().split('\n') {
        let cols: Vec<&str> = line.split('\t').collect();
        assert_eq!(cols.len(), 3);
        assert_eq!(cols[1], "0");
    }
}

#[tokio::test]
async fn search_recent_path_with_spaces_scopes_results() {
    let out = call_tool(
        "search_recent",
        json!({ "period": "1week", "path": "C:\\Program Files" }),
    )
    .await;
    assert!(!out.is_empty());
    assert!(!out.contains("Error"));
    if out.trim() == "No matching files found" {
        return;
    }
    // path: is a contains-match covering both C:\Program Files\ and
    // C:\Program Files (x86)\
    for line in out.trim().split('\n') {
        assert!(line.to_lowercase().starts_with("c:\\program files"));
    }
}
