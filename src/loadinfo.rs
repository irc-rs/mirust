use std::sync::OnceLock;

use windows::Win32::Foundation::HWND;
use windows::core::BOOL;

use crate::win_utils::get_mirc32_hwnd;

// The global LOADINFO storage. Kept crate-private; access via helpers.
pub(crate) static LOADINFO: OnceLock<LOADINFO> = OnceLock::new();

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct LOADINFO {
    pub m_version: u32,  // mIRC version number (low/high words). Added in mIRC 5.8
    pub m_hwnd: HWND,    // Handle to main mIRC window. Added in mIRC 5.8
    pub m_keep: BOOL,    // set to TRUE to keep DLL loaded after call. Added in mIRC 5.8.
    pub m_unicode: BOOL, // set to TRUE to have mIRC use UTF-16 strings. Added in mIRC 7.0.
    pub m_beta: u32,     // Beta version number (if applicable). Added in mIRC 7.51.
    pub m_bytes: u32,    // Max bytes allowed in data/parms buffers. Added in mIRC 7.64.
}

// SAFETY: LOADINFO is a plain C-compatible struct with only Copy types.
// HWND is stable and valid for the DLL's lifetime inside the host process.
// m_keep and m_unicode may be mutated during setup, but not shared unsafely across threads.
unsafe impl Send for LOADINFO {}
unsafe impl Sync for LOADINFO {}

/// Return the global LOADINFO stored in the OnceLock.
/// If it hasn't been set by `LoadDll`, create a reasonable default,
/// store it in the static and return a reference to it.
pub fn get_loadinfo() -> &'static LOADINFO {
    LOADINFO.get_or_init(|| {
        // Initialize a conservative default representing mIRC v5.6
        let default_version: u32 = (60 << 16) | 5; // v5.60

        let loadinfo = LOADINFO {
            m_version: default_version,
            m_hwnd: get_mirc32_hwnd(),
            m_keep: BOOL(0),
            m_unicode: BOOL(0),
            m_beta: 0,
            m_bytes: 900,
        };

        println!(
            "Initializing LOADINFO with default mIRC version v5.6 (packed = {})",
            default_version
        );

        loadinfo
    })
}

/// Set the global LOADINFO. Mirrors previous behavior of calling `LOADINFO.set(...)`.
/// Returns Ok(()) on success, Err(()) if it was already set.
pub(crate) fn set_loadinfo(li: LOADINFO) -> Result<(), ()> {
    LOADINFO.set(li).map_err(|_| ())
}
