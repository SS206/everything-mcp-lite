//! End-to-end check: real stdio subprocess, tool list + all tools (mirrors the
//! TS e2e test).
use rmcp::model::CallToolRequestParams;
use rmcp::transport::TokioChildProcess;
use rmcp::{ServiceExt, service::RunningService};
use serde_json::{Value, json};

fn extract_text(result: &rmcp::model::CallToolResult) -> String {
    for block in &result.content {
        if let rmcp::model::ContentBlock::Text(t) = block {
            return t.text.clone();
        }
    }
    String::new()
}

#[tokio::test(flavor = "multi_thread")]
async fn e2e_stdio_subprocess_exact_tool_list_all_tools_callable() {
    let exe = env!("CARGO_BIN_EXE_everything-mcp-lite");
    let transport =
        TokioChildProcess::new(tokio::process::Command::new(exe)).expect("spawn server");
    let client: RunningService<rmcp::service::RoleClient, ()> =
        ().serve(transport).await.expect("client serve");

    let tools = client.list_all_tools().await.expect("tools/list");
    let mut names: Vec<String> = tools.iter().map(|t| t.name.to_string()).collect();
    names.sort();
    println!("[1] tools/list -> {names:?}");
    assert_eq!(names, vec!["search", "search_recent"]);

    let home = std::env::var("USERPROFILE").unwrap_or_default();
    let checks: Vec<(&str, Value)> = vec![
        ("search", json!({ "query": "*.py", "top_n": 5 })),
        (
            "search",
            json!({ "query": "*.py", "top_n": 3, "include_size": true }),
        ),
        ("search", json!({ "query": "" })),
        (
            "search",
            json!({ "query": "size:0", "top_n": 3, "include_size": true }),
        ),
        ("search_recent", json!({})),
        ("search_recent", json!({ "period": "decade" })),
        (
            "search_recent",
            json!({ "period": "1week", "path": "C:\\Program Files" }),
        ),
        (
            "search_recent",
            json!({ "period": "1day", "path": home, "extensions": "py,md", "top_n": 5 }),
        ),
        (
            "search",
            json!({ "query": "ext:py size:>1mb dm:today", "top_n": 3 }),
        ),
    ];

    for (i, (tool, args)) in checks.iter().enumerate() {
        let params =
            CallToolRequestParams::new(*tool).with_arguments(args.as_object().unwrap().clone());
        let result = client.call_tool(params).await.expect("tools/call");
        let text = extract_text(&result);
        println!(
            "[{}] {} {} -> {}",
            i + 2,
            tool,
            args,
            text.chars().take(200).collect::<String>()
        );
        assert!(!text.is_empty(), "[{}] returned empty text", i + 2);
    }

    let _ = client.cancel().await;
    // Give the spawned server process a moment to observe the closed stdio
    // and exit; TokioChildProcess reaps it on drop.
}
