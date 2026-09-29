//! Raw FFI: the `LOADINFO` layout and the handful of Win32 functions mirust needs.
//!
//! Declared by hand rather than pulled from `windows`/`windows-sys` so the crate has no
//! dependencies; the surface is small and the signatures are stable Win32 ABI.

use core::ffi::c_void;

/// `LOADINFO` exactly as current mIRC defines it.
///
/// **Never create a reference to this struct.** mIRC grew it over time and older versions
/// allocate only the prefix they know about (5.8 – 6.x pass a 12-byte struct on the stack),
/// so the trailing fields may not exist. Access individual fields through raw pointers,
/// gated on the host version.
#[repr(C)]
pub(crate) struct LoadInfo {
    /// Version: major in the low word, minor in the high word. Added in 5.8.
    pub(crate) m_version: u32,
    /// Main mIRC window. Added in 5.8.
    pub(crate) m_hwnd: *mut c_void,
    /// Set to FALSE to have mIRC unload the DLL after each call. Added in 5.8.
    pub(crate) m_keep: i32,
    /// Set to TRUE to receive UTF-16 strings. Added in 7.0.
    pub(crate) m_unicode: i32,
    /// Public beta number, 0 for releases. Added in 7.51.
    pub(crate) m_beta: u32,
    /// Maximum bytes allowed in `data` / `parms`. Added in 7.64.
    pub(crate) m_bytes: u32,
}

pub(crate) const CP_ACP: u32 = 0;
/// `RT_VERSION`, as an integer resource id.
pub(crate) const RT_VERSION: usize = 16;
/// `VS_VERSION_INFO`, the id of an executable's version resource.
pub(crate) const VS_VERSION_INFO: usize = 1;

pub(crate) const GET_MODULE_HANDLE_EX_FLAG_PIN: u32 = 0x1;
pub(crate) const GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT: u32 = 0x2;
pub(crate) const GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS: u32 = 0x4;

pub(crate) type ThreadProc = unsafe extern "system" fn(param: *mut c_void) -> u32;

#[link(name = "kernel32")]
unsafe extern "system" {
    pub(crate) fn GetCurrentThreadId() -> u32;

    pub(crate) fn GetModuleHandleExW(flags: u32, name: *const u16, module: *mut *mut c_void)
    -> i32;

    pub(crate) fn FreeLibrary(module: *mut c_void) -> i32;

    pub(crate) fn GetProcAddress(module: *mut c_void, name: *const u8) -> *mut c_void;

    pub(crate) fn GetModuleHandleW(name: *const u16) -> *mut c_void;

    pub(crate) fn FindResourceW(
        module: *mut c_void,
        name: *const u16,
        kind: *const u16,
    ) -> *mut c_void;

    pub(crate) fn LoadResource(module: *mut c_void, resource: *mut c_void) -> *mut c_void;

    pub(crate) fn LockResource(loaded: *mut c_void) -> *const c_void;

    pub(crate) fn SizeofResource(module: *mut c_void, resource: *mut c_void) -> u32;

    pub(crate) fn FreeLibraryAndExitThread(module: *mut c_void, exit_code: u32) -> !;

    pub(crate) fn ExitThread(exit_code: u32) -> !;

    pub(crate) fn CreateThread(
        attributes: *const c_void,
        stack_size: usize,
        start: ThreadProc,
        param: *mut c_void,
        flags: u32,
        thread_id: *mut u32,
    ) -> *mut c_void;

    pub(crate) fn CloseHandle(handle: *mut c_void) -> i32;

    pub(crate) fn CreateFileMappingW(
        file: *mut c_void,
        attributes: *const c_void,
        protect: u32,
        size_high: u32,
        size_low: u32,
        name: *const u16,
    ) -> *mut c_void;

    pub(crate) fn MapViewOfFile(
        mapping: *mut c_void,
        access: u32,
        offset_high: u32,
        offset_low: u32,
        bytes: usize,
    ) -> *mut c_void;

    pub(crate) fn UnmapViewOfFile(view: *const c_void) -> i32;

    pub(crate) fn GetLastError() -> u32;

    pub(crate) fn GetTickCount64() -> u64;

    pub(crate) fn GetCurrentProcessId() -> u32;

    pub(crate) fn GetModuleFileNameW(module: *mut c_void, filename: *mut u16, size: u32) -> u32;

    pub(crate) fn MultiByteToWideChar(
        code_page: u32,
        flags: u32,
        multi_byte: *const u8,
        multi_byte_len: i32,
        wide: *mut u16,
        wide_len: i32,
    ) -> i32;

    pub(crate) fn WideCharToMultiByte(
        code_page: u32,
        flags: u32,
        wide: *const u16,
        wide_len: i32,
        multi_byte: *mut u8,
        multi_byte_len: i32,
        default_char: *const u8,
        used_default_char: *mut i32,
    ) -> i32;
}

pub(crate) const WH_CALLWNDPROC: i32 = 4;
pub(crate) const WM_DESTROY: u32 = 0x0002;
pub(crate) const WM_ENDSESSION: u32 = 0x0016;
pub(crate) const SMTO_ABORTIFHUNG: u32 = 0x0002;
pub(crate) const SMTO_BLOCK: u32 = 0x0001;
pub(crate) const SMTO_ERRORONEXIT: u32 = 0x0020;
pub(crate) const WM_USER: u32 = 0x0400;
pub(crate) const PAGE_READWRITE: u32 = 0x04;
pub(crate) const FILE_MAP_WRITE: u32 = 0x0002;
pub(crate) const FILE_MAP_READ: u32 = 0x0004;
pub(crate) const ERROR_ALREADY_EXISTS: u32 = 183;
pub(crate) const ERROR_TIMEOUT: u32 = 1460;
pub(crate) const ERROR_INVALID_WINDOW_HANDLE: u32 = 1400;
/// `INVALID_HANDLE_VALUE`: backs a file mapping with the paging file rather than a file.
pub(crate) const INVALID_HANDLE_VALUE: *mut c_void = -1isize as *mut c_void;

pub(crate) type HookProc =
    unsafe extern "system" fn(code: i32, wparam: usize, lparam: isize) -> isize;

/// `CWPSTRUCT`: a message sent to a window, as seen by a `WH_CALLWNDPROC` hook.
#[repr(C)]
pub(crate) struct CwpStruct {
    pub(crate) lparam: isize,
    pub(crate) wparam: usize,
    pub(crate) message: u32,
    pub(crate) hwnd: *mut c_void,
}

#[link(name = "user32")]
unsafe extern "system" {
    pub(crate) fn GetWindowThreadProcessId(hwnd: *mut c_void, process_id: *mut u32) -> u32;

    pub(crate) fn GetClassNameW(hwnd: *mut c_void, class_name: *mut u16, max_count: i32) -> i32;

    pub(crate) fn SetWindowsHookExW(
        id: i32,
        hook: HookProc,
        module: *mut c_void,
        thread_id: u32,
    ) -> *mut c_void;

    pub(crate) fn UnhookWindowsHookEx(hook: *mut c_void) -> i32;

    pub(crate) fn CallNextHookEx(
        hook: *mut c_void,
        code: i32,
        wparam: usize,
        lparam: isize,
    ) -> isize;

    pub(crate) fn RegisterWindowMessageW(name: *const u16) -> u32;

    pub(crate) fn SendMessageW(
        hwnd: *mut c_void,
        message: u32,
        wparam: usize,
        lparam: isize,
    ) -> isize;

    pub(crate) fn SendMessageTimeoutW(
        hwnd: *mut c_void,
        message: u32,
        wparam: usize,
        lparam: isize,
        flags: u32,
        timeout_ms: u32,
        result: *mut usize,
    ) -> isize;
}
