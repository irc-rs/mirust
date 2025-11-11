use windows::core::BOOL;

use crate::loadinfo::{LOADINFO, set_loadinfo};
use crate::version::fix_m_version;

#[allow(dead_code)]
#[repr(i32)]
enum UnloadReason {
    ManualUnload, // /dll -u or mKeep = false
    Timeout,      // DLL unused for 10 minutes
    Exit,         // mIRC is exiting
}

#[unsafe(no_mangle)]
extern "system" fn LoadDll(loadinfo: *mut LOADINFO) -> i32 {
    // SAFETY: mIRC guarantees that loadinfo is a valid pointer to a LOADINFO struct.
    let loadinfo = unsafe { &mut *loadinfo };

    let m_version = loadinfo.m_version;
    let m_version = fix_m_version(m_version);
    let m_version_major = m_version & 0xFFFF; // Low word
    let m_version_minor = (m_version >> 16) & 0xFFFF; // High word

    if m_version_major >= 7 {
        loadinfo.m_unicode = BOOL(1);
    }

    // Create an owned copy to modify and reference later
    let mut loadinfo = *loadinfo;
    // Update our copy's version to the corrected value
    loadinfo.m_version = m_version;

    // mUnicode added in 7.0
    if m_version_major < 7 {
        loadinfo.m_unicode = false.into(); // Unicode added in 7.0
    }

    // mBeta added in 7.51
    if m_version_major < 7 || (m_version_major == 7 && m_version_minor < 51) {
        loadinfo.m_beta = 0; // No beta versioning before mIRC 7.51
    }

    // mBytes added in 7.64
    if m_version_major < 7 || (m_version_major == 7 && m_version_minor < 64) {
        loadinfo.m_bytes = 900; // Buffer size as of mIRC 5.6 (when DLL support was added)
    }

    set_loadinfo(loadinfo).expect("LOADINFO was already set");
    0
}

#[unsafe(no_mangle)]
extern "system" fn UnloadDll(m_timeout: UnloadReason) -> i32 {
    match m_timeout {
        UnloadReason::ManualUnload => {
            // Either /dll -u was called or loadinfo.m_keep was set to false
        }
        UnloadReason::Timeout => {
            // We've set mKeep to true, so we return 0 to prevent unloading
            return 0;
        }
        UnloadReason::Exit => {
            // mIRC is exiting, perform cleanup
        }
    }
    0
}
