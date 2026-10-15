//! Everything file-search MCP tools (rmcp).
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use rmcp::handler::server::wrapper::Parameters;
use rmcp::schemars;
use rmcp::{ServerHandler, tool, tool_handler, tool_router};

use crate::everything_sdk::{
    EVERYTHING_SORT_DATE_MODIFIED_DESCENDING, EverythingError, EverythingSdk, SearchFilesOptions,
    SearchResult, default_dll_path,
};

pub const VALID_PERIODS: [&str; 6] = ["1min", "1hour", "today", "yesterday", "1day", "1week"];

const SEARCH_DESCRIPTION: &str = r#"Search file names across all drives via the Everything engine. Returns one line per match: "full path<TAB>last modified time (local time)"; with include_size=true: "path<TAB>size in bytes<TAB>modified".

query supports Everything search syntax:
- space = AND, | = OR, ! = NOT
- ext:py         filter by extension
- size:>1gb      filter by size (> <, kb/mb/gb)
- dm:today       by modified date (today/yesterday/lastweek, YYYY-MM-DD)
- path:D:\xx     limit to a directory
Example: "ext:py size:>1mb dm:today"

Never throws: on failure returns a single line starting with "Error:"; no matches returns "No matching files found"."#;

const RECENT_DESCRIPTION: &str = r#"Search recently modified files, newest first. Returns one line per match: "full path<TAB>last modified time (local time)".

- period: one of 1min, 1hour (default), today, yesterday, 1day, 1week
- path: optional directory filter; may contain spaces (quoted automatically), must not contain double quotes
- extensions: optional comma-separated extension filter, alphanumeric only, e.g. "py,md" or ".py, md"

Never throws: on failure returns a single line starting with "Error:"; no matches returns "No matching files found"."#;

/// Lazy, retry-on-next-call SDK singleton: DLL load failures surface as
/// `EverythingError` per call instead of failing startup (mirrors the TS
/// `getSdk()`).
static SDK_CELL: OnceLock<Mutex<Option<EverythingSdk>>> = OnceLock::new();

fn with_sdk<T>(
    f: impl FnOnce(&EverythingSdk) -> Result<T, EverythingError>,
) -> Result<T, EverythingError> {
    let cell = SDK_CELL.get_or_init(|| Mutex::new(None));
    let mut guard = cell.lock().unwrap_or_else(|e| e.into_inner());
    if guard.is_none() {
        let dll_path = std::env::var_os("EVERYTHING_SDK_PATH")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(default_dll_path);
        *guard = Some(EverythingSdk::new(&dll_path)?);
    }
    f(guard.as_ref().expect("sdk just initialized"))
}

pub fn format_results(results: &[SearchResult], include_size: bool) -> String {
    if results.is_empty() {
        return "No matching files found".into();
    }
    results
        .iter()
        .map(|r| {
            let modified = r.modified.as_deref().unwrap_or("N/A");
            if include_size {
                format!("{}\t{}\t{}", r.path, r.size, modified)
            } else {
                format!("{}\t{}", r.path, modified)
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn period_minutes(period: &str) -> i64 {
    match period {
        "1min" => 1,
        "1hour" => 60,
        "1day" => 1440,
        _ => 10080, // 1week
    }
}

fn period_to_query(period: &str) -> String {
    if period == "today" || period == "yesterday" {
        return format!("dm:{period}");
    }
    let boundary = chrono::Local::now() - chrono::Duration::minutes(period_minutes(period));
    format!("dm:>{}", boundary.format("%Y-%m-%dT%H:%M:%S"))
}

pub fn build_recent_query(
    period: &str,
    path: Option<&str>,
    extensions: Option<&str>,
) -> Result<String, String> {
    if !VALID_PERIODS.contains(&period) {
        return Err(format!(
            "Invalid period '{period}', allowed values: {}",
            VALID_PERIODS.join(", ")
        ));
    }
    let mut query = period_to_query(period);
    if let Some(p) = path {
        let trimmed = p.trim();
        if !trimmed.is_empty() {
            if trimmed.contains('"') {
                return Err("path must not contain double quotes".into());
            }
            // Everything splits search terms on spaces; a path with spaces must be quoted.
            query += &format!(" path:\"{trimmed}\"");
        }
    }
    if let Some(exts) = extensions {
        let parts: Vec<String> = exts
            .split(',')
            .map(|p| p.trim().trim_start_matches('.').to_string())
            .filter(|p| !p.is_empty())
            .collect();
        let invalid: Vec<&str> = parts
            .iter()
            .map(String::as_str)
            .filter(|p| !p.chars().all(|c| c.is_alphanumeric()))
            .collect();
        if !invalid.is_empty() {
            return Err(format!(
                "Invalid extension '{}', only alphanumeric characters are allowed",
                invalid.join(", ")
            ));
        }
        if !parts.is_empty() {
            // Everything's ext: syntax only accepts semicolons; accept commas for convenience.
            query += &format!(" ext:{}", parts.join(";"));
        }
    }
    Ok(query)
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SearchArgs {
    pub query: String,
    pub top_n: Option<i64>,
    pub include_size: Option<bool>,
    pub match_case: Option<bool>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SearchRecentArgs {
    pub period: Option<String>,
    pub path: Option<String>,
    pub extensions: Option<String>,
    pub top_n: Option<i64>,
}

#[derive(Clone)]
pub struct EverythingTools;

#[tool_router]
impl EverythingTools {
    #[tool(description = SEARCH_DESCRIPTION)]
    fn search(&self, Parameters(args): Parameters<SearchArgs>) -> String {
        if args.query.trim().is_empty() {
            return "Error: query must not be empty".into();
        }
        let opts = SearchFilesOptions {
            query: args.query.trim().to_string(),
            top_n: args.top_n.unwrap_or(20),
            match_case: args.match_case.unwrap_or(false),
            ..Default::default()
        };
        match with_sdk(|sdk| sdk.search_files(opts)) {
            Ok(results) => format_results(&results, args.include_size.unwrap_or(false)),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(description = RECENT_DESCRIPTION)]
    fn search_recent(&self, Parameters(args): Parameters<SearchRecentArgs>) -> String {
        let period = args.period.as_deref().unwrap_or("1hour");
        let query = match build_recent_query(period, args.path.as_deref(), args.extensions.as_deref())
        {
            Ok(q) => q,
            Err(msg) => return format!("Error: {msg}"),
        };
        let opts = SearchFilesOptions {
            query,
            top_n: args.top_n.unwrap_or(20),
            sort_by: EVERYTHING_SORT_DATE_MODIFIED_DESCENDING,
            ..Default::default()
        };
        match with_sdk(|sdk| sdk.search_files(opts)) {
            Ok(results) => format_results(&results, false),
            Err(e) => format!("Error: {e}"),
        }
    }
}

#[tool_handler(name = "everything-search", version = "0.1.0")]
impl ServerHandler for EverythingTools {}
