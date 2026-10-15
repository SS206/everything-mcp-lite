//! Everything SDK FFI wrapper for filename search.
//!
//! The DLL keeps query state in process-global variables, mirroring the
//! Python version's `_SDK_LOCK`: a process-level mutex serializes every
//! query, and `Everything_Reset` runs on all exit paths.

use std::fmt;
use std::path::{Path, PathBuf};

use libloading::{Library, Symbol};

pub const EVERYTHING_ERROR_MEMORY: u32 = 1;
pub const EVERYTHING_ERROR_IPC: u32 = 2;
pub const EVERYTHING_ERROR_REGISTERCLASSEX: u32 = 3;
pub const EVERYTHING_ERROR_CREATEWINDOW: u32 = 4;
pub const EVERYTHING_ERROR_CREATETHREAD: u32 = 5;
pub const EVERYTHING_ERROR_INVALIDINDEX: u32 = 6;
pub const EVERYTHING_ERROR_INVALIDCALL: u32 = 7;

pub const EVERYTHING_REQUEST_FULL_PATH_AND_FILE_NAME: u32 = 0x0000_0004;
pub const EVERYTHING_REQUEST_SIZE: u32 = 0x0000_0010;
pub const EVERYTHING_REQUEST_DATE_MODIFIED: u32 = 0x0000_0040;

pub const EVERYTHING_SORT_NAME_ASCENDING: u32 = 1;
pub const EVERYTHING_SORT_SIZE_DESCENDING: u32 = 6;
pub const EVERYTHING_SORT_DATE_MODIFIED_DESCENDING: u32 = 14;

const REQUEST_FLAGS: u32 = EVERYTHING_REQUEST_FULL_PATH_AND_FILE_NAME
    | EVERYTHING_REQUEST_SIZE
    | EVERYTHING_REQUEST_DATE_MODIFIED;

fn error_message(error_code: u32) -> String {
    match error_code {
        EVERYTHING_ERROR_MEMORY => "Memory allocation failed".into(),
        EVERYTHING_ERROR_IPC => {
            "Failed to communicate with Everything, make sure Everything is running".into()
        }
        EVERYTHING_ERROR_REGISTERCLASSEX => "Window class registration failed".into(),
        EVERYTHING_ERROR_CREATEWINDOW => "Window creation failed".into(),
        EVERYTHING_ERROR_CREATETHREAD => "Thread creation failed".into(),
        EVERYTHING_ERROR_INVALIDINDEX => "Invalid index".into(),
        EVERYTHING_ERROR_INVALIDCALL => "Invalid call".into(),
        _ => format!("Unknown error: {error_code}"),
    }
}

#[derive(Debug, Clone)]
pub struct EverythingError {
    pub error_code: u32,
    message: Option<String>,
}

impl EverythingError {
    pub fn new(error_code: u32, message: Option<String>) -> Self {
        Self {
            error_code,
            message,
        }
    }
}

impl fmt::Display for EverythingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(m) = &self.message {
            write!(f, "{m}")
        } else {
            write!(f, "{}", error_message(self.error_code))
        }
    }
}

impl std::error::Error for EverythingError {}

#[derive(Debug, Clone)]
pub struct SearchResult {
    pub path: String,
    pub size: u64,
    pub modified: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SearchFilesOptions {
    pub query: String,
    pub top_n: i64,
    pub match_case: bool,
    pub sort_by: u32,
}

impl Default for SearchFilesOptions {
    fn default() -> Self {
        Self {
            query: String::new(),
            top_n: 20,
            match_case: false,
            sort_by: EVERYTHING_SORT_NAME_ASCENDING,
        }
    }
}

pub fn default_dll_path() -> PathBuf {
    match std::env::var_os("LOCALAPPDATA") {
        Some(local_app_data) if !local_app_data.is_empty() => PathBuf::from(local_app_data)
            .join("Everything-SDK")
            .join("dll")
            .join("Everything64.dll"),
        _ => PathBuf::from("Everything64.dll"),
    }
}

/// Local-time ISO string (second precision) for a Windows FILETIME value.
pub fn filetime_to_local_iso(filetime: u64) -> Option<String> {
    if filetime == 0 {
        return None;
    }
    // Integer-divide in 64-bit first: FILETIME exceeds f64's 2^53 exact range.
    let secs = (filetime / 10_000_000) as i64 - 11_644_473_600;
    let utc = chrono::DateTime::from_timestamp(secs, 0)?;
    let local = utc.with_timezone(&chrono::Local);
    Some(local.format("%Y-%m-%dT%H:%M:%S").to_string())
}

type SetSearchW = unsafe extern "system" fn(*const u16);
type SetFlag = unsafe extern "system" fn(i32);
type SetDword = unsafe extern "system" fn(u32);
type QueryW = unsafe extern "system" fn(i32) -> i32;
type GetNumResults = unsafe extern "system" fn() -> u32;
type GetError = unsafe extern "system" fn() -> u32;
type GetFullPathW = unsafe extern "system" fn(u32, *mut u16, u32) -> u32;
type GetU64 = unsafe extern "system" fn(u32, *mut u64) -> i32;
type ResetFn = unsafe extern "system" fn();

pub struct EverythingSdk {
    // Declared first: struct fields drop in declaration order, so the library
    // outlives every symbol bound from it. Never read directly.
    #[allow(dead_code)]
    lib: Library,
    set_search_w: Symbol<'static, SetSearchW>,
    set_match_path: Symbol<'static, SetFlag>,
    set_match_case: Symbol<'static, SetFlag>,
    set_max: Symbol<'static, SetDword>,
    set_sort: Symbol<'static, SetDword>,
    set_request_flags: Symbol<'static, SetDword>,
    query_w: Symbol<'static, QueryW>,
    get_num_results: Symbol<'static, GetNumResults>,
    get_last_error: Symbol<'static, GetError>,
    get_full_path_w: Symbol<'static, GetFullPathW>,
    get_result_size: Symbol<'static, GetU64>,
    get_result_date_modified: Symbol<'static, GetU64>,
    reset: Symbol<'static, ResetFn>,
}

/// The SDK uses `BOOL` (4-byte int); a Rust `bool` is 1 byte, so all flags are
/// declared as `i32`. Win32 wide strings are UTF-16 with a NUL terminator.
unsafe fn bind<T>(lib: &Library, name: &[u8]) -> Symbol<'static, T> {
    let sym: Symbol<'_, T> = unsafe { lib.get(name) }.unwrap_or_else(|e| {
        panic!(
            "missing export {} in Everything SDK DLL: {e}",
            String::from_utf8_lossy(&name[..name.len() - 1])
        )
    });
    // SAFETY: the symbol is stored in EverythingSdk before `lib` (drop order),
    // so it never outlives the library it points into.
    unsafe { std::mem::transmute(sym) }
}

impl EverythingSdk {
    pub fn new(dll_path: &Path) -> Result<Self, EverythingError> {
        let lib = unsafe { Library::new(dll_path) }.map_err(|_| {
            EverythingError::new(
                EVERYTHING_ERROR_IPC,
                Some(format!(
                    "Failed to load Everything SDK DLL: {}",
                    dll_path.display()
                )),
            )
        })?;
        Ok(Self {
            set_search_w: unsafe { bind(&lib, b"Everything_SetSearchW\0") },
            set_match_path: unsafe { bind(&lib, b"Everything_SetMatchPath\0") },
            set_match_case: unsafe { bind(&lib, b"Everything_SetMatchCase\0") },
            set_max: unsafe { bind(&lib, b"Everything_SetMax\0") },
            set_sort: unsafe { bind(&lib, b"Everything_SetSort\0") },
            set_request_flags: unsafe { bind(&lib, b"Everything_SetRequestFlags\0") },
            query_w: unsafe { bind(&lib, b"Everything_QueryW\0") },
            get_num_results: unsafe { bind(&lib, b"Everything_GetNumResults\0") },
            get_last_error: unsafe { bind(&lib, b"Everything_GetLastError\0") },
            get_full_path_w: unsafe { bind(&lib, b"Everything_GetResultFullPathNameW\0") },
            get_result_size: unsafe { bind(&lib, b"Everything_GetResultSize\0") },
            get_result_date_modified: unsafe { bind(&lib, b"Everything_GetResultDateModified\0") },
            reset: unsafe { bind(&lib, b"Everything_Reset\0") },
            lib,
        })
    }

    fn get_full_path(&self, index: u32) -> String {
        // Two-call buffer pattern: probe the needed length, then allocate
        // needed + 1 and fill. A fixed-size buffer silently truncates long paths.
        let needed =
            unsafe { (self.get_full_path_w)(index, std::ptr::null_mut(), 0) };
        if needed == 0 {
            return String::new();
        }
        let mut buf: Vec<u16> = vec![0; (needed + 1) as usize];
        unsafe { (self.get_full_path_w)(index, buf.as_mut_ptr(), needed + 1) };
        let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        String::from_utf16_lossy(&buf[..len])
    }

    pub fn search_files(
        &self,
        opts: SearchFilesOptions,
    ) -> Result<Vec<SearchResult>, EverythingError> {
        let top_n = opts.top_n.clamp(1, 100) as u32;
        let run = || -> Result<Vec<SearchResult>, EverythingError> {
            let _guard = SDK_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let query_wide: Vec<u16> = opts
                .query
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect();
            unsafe {
                (self.set_search_w)(query_wide.as_ptr());
                (self.set_match_path)(0);
                (self.set_match_case)(if opts.match_case { 1 } else { 0 });
                (self.set_max)(top_n);
                (self.set_sort)(opts.sort_by);
                (self.set_request_flags)(REQUEST_FLAGS);
                if (self.query_w)(1) == 0 {
                    return Err(EverythingError::new((self.get_last_error)(), None));
                }
                let num_results = (self.get_num_results)().min(top_n);
                let mut results = Vec::with_capacity(num_results as usize);
                for i in 0..num_results {
                    let mut size = 0u64;
                    let mut modified = 0u64;
                    (self.get_result_size)(i, &mut size);
                    (self.get_result_date_modified)(i, &mut modified);
                    results.push(SearchResult {
                        path: self.get_full_path(i),
                        size,
                        modified: filetime_to_local_iso(modified),
                    });
                }
                Ok(results)
            }
        };
        let out = run();
        unsafe { (self.reset)() };
        out
    }
}

/// The DLL stores query state in process-global variables; serialize all calls.
static SDK_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
