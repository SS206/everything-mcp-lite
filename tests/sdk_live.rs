//! Integration tests for the FFI wrapper against the live Everything index.
use std::path::{Path, PathBuf};

use everything_mcp_lite::everything_sdk::{
    EVERYTHING_SORT_DATE_MODIFIED_DESCENDING, EverythingSdk, SearchFilesOptions, default_dll_path,
};

fn dll_path() -> PathBuf {
    std::env::var_os("EVERYTHING_SDK_PATH")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(default_dll_path)
}

fn opts(query: &str, top_n: i64) -> SearchFilesOptions {
    SearchFilesOptions {
        query: query.into(),
        top_n,
        ..Default::default()
    }
}

#[test]
fn search_basic_returns_nonempty_paths() {
    let sdk = EverythingSdk::new(&dll_path()).unwrap();
    let results = sdk.search_files(opts("*.py", 5)).unwrap();
    assert!(results.len() <= 5);
    for r in &results {
        assert!(!r.path.is_empty());
    }
}

#[test]
fn top_n_is_clamped_to_100() {
    let sdk = EverythingSdk::new(&dll_path()).unwrap();
    let results = sdk.search_files(opts("*.py", 300)).unwrap();
    assert!(results.len() <= 100);
}

#[test]
fn sort_by_date_modified_descending() {
    let sdk = EverythingSdk::new(&dll_path()).unwrap();
    let mut o = opts("*.py", 10);
    o.sort_by = EVERYTHING_SORT_DATE_MODIFIED_DESCENDING;
    let results = sdk.search_files(o).unwrap();
    let mods: Vec<&str> = results
        .iter()
        .filter_map(|r| r.modified.as_deref())
        .collect();
    let mut sorted: Vec<&str> = mods.clone();
    sorted.sort();
    sorted.reverse();
    assert_eq!(mods, sorted);
}

#[test]
fn bad_dll_path_raises_everything_error() {
    let err = match EverythingSdk::new(&Path::new("C:\\nonexistent").join("Everything64.dll")) {
        Err(e) => e,
        Ok(_) => panic!("expected EverythingError for a bad DLL path"),
    };
    assert!(err.to_string().contains("Failed to load"));
}
