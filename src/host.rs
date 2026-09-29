use core::ffi::c_void;
use core::ptr;
use std::sync::OnceLock;

use crate::{Encoding, Version, sys};

/// A window handle (`HWND`) passed in by mIRC.
///
/// Convert to your Win32 bindings of choice with [`as_raw`](Self::as_raw); for example
/// `windows::Win32::Foundation::HWND(handle.as_raw())`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct WindowHandle(*mut c_void);

// SAFETY: an HWND is an opaque identifier, not a pointer we dereference. Win32 accepts it
// from any thread.
unsafe impl Send for WindowHandle {}
// SAFETY: as above.
unsafe impl Sync for WindowHandle {}

impl WindowHandle {
    /// Wraps a raw `HWND`.
    pub const fn from_raw(hwnd: *mut c_void) -> Self {
        Self(hwnd)
    }

    /// The raw `HWND`.
    pub const fn as_raw(self) -> *mut c_void {
        self.0
    }

    /// Whether the handle is null.
    pub fn is_null(self) -> bool {
        self.0.is_null()
    }

    /// Whether the calling thread is the thread that owns this window.
    ///
    /// For mIRC's main window this tells you if you are on mIRC's UI thread.
    pub fn is_current_thread(self) -> bool {
        // SAFETY: both calls accept any value; an invalid HWND just yields thread id 0.
        unsafe {
            sys::GetWindowThreadProcessId(self.0, ptr::null_mut()) == sys::GetCurrentThreadId()
        }
    }
}

/// Which client loaded the DLL: mIRC, or AdiIRC, the only other client that loads mIRC
/// DLLs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Client {
    /// mIRC.
    Mirc,
    /// AdiIRC, which imitates mIRC. Since 4.4 it reports itself as mIRC 7.64.
    AdiIrc,
}

impl Client {
    /// Which client owns `window`, judged by its window class.
    ///
    /// Pass mIRC's main window: [`Host::main_window`] or [`Call::main_window`](crate::Call::main_window).
    /// This is a single `GetClassNameW` call. [`Host::client`] holds the answer for the main
    /// window.
    ///
    /// | Window class       | Client            |
    /// |--------------------|-------------------|
    /// | `mIRC`, `mIRC32`   | [`Mirc`](Self::Mirc) (`mIRC32` before 6.0) |
    /// | anything else      | [`AdiIrc`](Self::AdiIrc), whose .NET window classes have generated names |
    pub fn of_window(window: WindowHandle) -> Self {
        let mut class = [0u16; 16];
        // SAFETY: the buffer length is passed in; an invalid or null window just fails.
        let len =
            unsafe { sys::GetClassNameW(window.as_raw(), class.as_mut_ptr(), class.len() as i32) };
        Self::from_class(&class[..len.max(0) as usize])
    }

    fn from_class(class: &[u16]) -> Self {
        let is = |name: &str| class.iter().copied().eq(name.encode_utf16());
        if is("mIRC") || is("mIRC32") {
            Self::Mirc
        } else {
            Self::AdiIrc
        }
    }
}

/// What mirust knows about the client that loaded the DLL: the equivalent of mIRC's
/// `LOADINFO`, valid on every version.
///
/// mIRC added `LOADINFO`'s fields over time, and versions before 5.8 have no `LOADINFO`
/// at all. mirust fills in every field for every version, so each method can be used
/// without checking the version first:
///
/// | `LOADINFO` | `Host` | Before mIRC had the field |
/// |------------|--------|---------------------------|
/// | `mVersion` (5.8) | [`version`](Self::version) | read from the executable's version resource (5.6 – 5.71) |
/// | `mHwnd` (5.8) | [`main_window`](Self::main_window) | the call's `mWnd` argument, which is the same window |
/// | `mKeep` (5.8) | [`keep_loaded`](Self::keep_loaded) | `false`: mIRC unloaded DLLs after every call |
/// | `mUnicode` (7.0) | [`unicode`](Self::unicode), [`encoding`](Self::encoding) | `false`: ANSI only |
/// | `mBeta` (7.51) | [`beta`](Self::beta) | `0`: betas weren't reported |
/// | `mBytes` (7.64) | [`capacity`](Self::capacity) | measured for each version (see [`buffer_capacity`]) |
///
/// Built when mIRC loads the DLL (or first calls it, on mIRC 5.6 – 5.71) and available
/// from [`crate::host`] or [`Call::host`](crate::Call::host).
#[derive(Clone, Debug)]
pub struct Host {
    client: Client,
    version: Version,
    beta: u32,
    main_window: WindowHandle,
    encoding: Encoding,
    capacity: usize,
    keep_loaded: bool,
}

impl Host {
    /// The client that loaded the DLL, from its main window's class; see
    /// [`Client::of_window`].
    pub fn client(&self) -> Client {
        self.client
    }

    /// The mIRC version, corrected for old reporting bugs (see [`Version`]).
    ///
    /// mIRC 5.6 – 5.71 don't call `LoadDll`, so they can't report a version. For them it is
    /// read from the executable's version resource instead, falling back to
    /// [`Version::V5_6`], the lowest version with DLL support. AdiIRC reports the mIRC
    /// version it emulates, not its own.
    pub fn version(&self) -> Version {
        self.version
    }

    /// The public beta number (`mBeta`), or 0 for a release.
    ///
    /// Always 0 before mIRC 7.51, which didn't report betas.
    pub fn beta(&self) -> u32 {
        self.beta
    }

    /// mIRC's main window (`mHwnd`).
    ///
    /// From `LoadDll` on mIRC 5.8+. mIRC 5.6 – 5.71 don't call `LoadDll`, but pass the same
    /// window as the first argument of every call, so it's taken from there, before your
    /// function runs. Either way it is set before any of your code runs. Every call also
    /// carries it in [`Call::main_window`](crate::Call::main_window).
    pub fn main_window(&self) -> WindowHandle {
        self.main_window
    }

    /// Whether strings cross the boundary as UTF-16 (`mUnicode`).
    ///
    /// `true` on mIRC 7+ unless [`Config::unicode(false)`](crate::Config::unicode); always
    /// `false` before 7.0. See [`encoding`](Self::encoding) for the details.
    pub fn unicode(&self) -> bool {
        self.encoding == Encoding::Utf16
    }

    /// How strings are encoded between mIRC and this DLL.
    pub fn encoding(&self) -> Encoding {
        self.encoding
    }

    /// The longest response mIRC accepts, in code units of [`encoding`](Self::encoding)
    /// (bytes, or UTF-16 units), including the NUL terminator: mirust's equivalent of
    /// `mBytes`, available on every version.
    ///
    /// Responses longer than `capacity() - 1` units are truncated at a character boundary.
    /// See [`buffer_capacity`] for where the value comes from.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Whether mIRC will keep the DLL loaded between calls (`mKeep`).
    ///
    /// Always `false` before mIRC 5.8, which unloaded DLLs after every call.
    pub fn keep_loaded(&self) -> bool {
        self.keep_loaded
    }
}

/// Size of the `data`/`parms` buffers, in code units including the NUL terminator.
///
/// This is the longest response mIRC accepts, which can be less than the buffer itself:
/// mIRC also has to handle the string once it has it.
///
/// `reported_bytes` is `mBytes` from `LOADINFO` (mIRC 7.64+). Older versions report nothing,
/// so their values come from disassembling mIRC and from testing it:
///
/// | mIRC          | Capacity    | Source |
/// |---------------|-------------|--------|
/// | 5.6 – 6.31    | 900         | 5.6 docs: *"can each hold 900 chars maximum"* (6.3 actually uses 999). 899-character responses tested in 6.03 – 6.31 (including 6.12, 6.14 – 6.17 and 6.2). |
/// | 6.32 – 7.52   | 4151        | The buffers are 4200 units (6.35, 7.14 – 7.42), but mIRC rejects results over 4150 characters as "line too long". Tested in 6.35, 7.14, 7.52. |
/// | 7.53 – 7.63   | 4200        | 7.53 raised mIRC's string limits; 4199-character responses tested in 7.62. Probably larger, but unmeasured. |
/// | 7.64 – 7.83   | `mBytes / 2`| mIRC reported the UTF-16 byte count even for ANSI DLLs (fixed in 7.84). 10239 characters tested in 7.72 – 7.83. |
/// | 7.84+         | `mBytes`    | 10240 in 7.84 and 7.85, against buffers of 10340+ units. 10239 characters tested. |
///
/// On mIRC 7 the buffers hold the same number of units in both modes (bytes in ANSI mode,
/// UTF-16 units in Unicode mode), so one count covers both.
pub const fn buffer_capacity(version: Version, reported_bytes: Option<u32>) -> usize {
    const DOCUMENTED: usize = 900;
    // mIRC 6.32 – 7.52 reject strings over 4150 characters, below their 4200-unit buffers.
    const LINE_LIMIT: usize = 4151;
    const EXTENDED: usize = 4200;

    let reported = match reported_bytes {
        Some(bytes) if bytes > 0 => Some(bytes as usize),
        _ => None,
    };

    if version.major() > 7 || (version.major() == 7 && version.minor() >= 84) {
        match reported {
            Some(bytes) => bytes,
            None => EXTENDED,
        }
    } else if version.major() == 7 && version.minor() >= 64 {
        match reported {
            Some(bytes) => bytes / 2,
            None => EXTENDED,
        }
    } else if version.major() == 7 && version.minor() >= 53 {
        EXTENDED
    } else if version.major() == 7 || (version.major() == 6 && version.minor() >= 32) {
        LINE_LIMIT
    } else {
        DOCUMENTED
    }
}

/// The version of a host that never calls `LoadDll`: from the executable's `FileVersion`
/// (mIRC 5.6 – 5.71 set it to "5.6", "5.61", "5.7" or "5.71"), or 5.6 if that isn't a
/// version in that range.
fn legacy_version() -> Version {
    host_file_version()
        .filter(|&v| v >= Version::V5_6 && v < Version::V5_8)
        .unwrap_or(Version::V5_6)
}

/// The `FileVersion` string of the host executable's version resource, as a [`Version`].
fn host_file_version() -> Option<Version> {
    parse_version(&host_version_string("FileVersion")?)
}

/// A string from the host executable's version resource, such as `FileVersion`.
fn host_version_string(key: &str) -> Option<String> {
    // SAFETY: a null name means the host executable, which stays loaded. Integer resource
    // ids are passed as pointers, as `MAKEINTRESOURCE` does. The locked resource lives as
    // long as the executable, and `SizeofResource` gives its length.
    let resource = unsafe {
        let module = sys::GetModuleHandleW(ptr::null());
        let found = sys::FindResourceW(
            module,
            sys::VS_VERSION_INFO as *const u16,
            sys::RT_VERSION as *const u16,
        );
        if module.is_null() || found.is_null() {
            return None;
        }
        let data = sys::LockResource(sys::LoadResource(module, found));
        let len = sys::SizeofResource(module, found) as usize;
        if data.is_null() {
            return None;
        }
        core::slice::from_raw_parts(data.cast::<u8>(), len)
    };
    version_resource_string(resource, key)
}

/// Finds the value of the string `key` in a `VS_VERSIONINFO` resource.
///
/// A `String` entry is a small header, its key as NUL-terminated UTF-16, padding to a
/// 4-byte boundary, then its value as NUL-terminated UTF-16. The resource starts 4-byte
/// aligned, so offsets within it can be aligned directly.
fn version_resource_string(resource: &[u8], key: &str) -> Option<String> {
    let key: Vec<u8> = key
        .encode_utf16()
        .chain([0])
        .flat_map(u16::to_le_bytes)
        .collect();
    let at = resource.windows(key.len()).position(|w| w == key)?;
    let value = (at + key.len() + 3) & !3;
    let units: Vec<u16> = resource
        .get(value..)?
        .chunks_exact(2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .take_while(|&u| u != 0)
        .collect();
    Some(String::from_utf16_lossy(&units))
}

/// Parses "5.6", "5.61", "7.85" and the like; the minor part is padded to two digits.
fn parse_version(text: &str) -> Option<Version> {
    let (major, minor) = text.trim().split_once('.')?;
    let minor: String = minor.chars().take_while(char::is_ascii_digit).collect();
    if minor.is_empty() || minor.len() > 2 {
        return None;
    }
    let padded = if minor.len() == 1 {
        format!("{minor}0")
    } else {
        minor
    };
    Some(Version::new(major.parse().ok()?, padded.parse().ok()?))
}

static HOST: OnceLock<Host> = OnceLock::new();

/// The client that loaded this DLL.
///
/// mirust records the host before running any of your code: in `LoadDll` (before
/// `on_load`), or at the start of the first call on mIRC 5.6 – 5.71, which don't call
/// `LoadDll`. So this always describes the real host when your code calls it.
pub fn host() -> &'static Host {
    HOST.get_or_init(|| Host::legacy(None))
}

/// Records the host for mIRC 5.6 – 5.71, which call exported functions without ever
/// calling `LoadDll`. The main window comes from the call.
pub(crate) fn init_legacy(main_window: WindowHandle) -> &'static Host {
    let _ = HOST.set(Host::legacy(Some(main_window)));
    host()
}

/// Reads `LOADINFO`, writes back our settings, and records the result as [`host`].
///
/// # Safety
///
/// `info` must be null or the pointer mIRC passed to `LoadDll`.
pub(crate) unsafe fn init(
    info: *mut sys::LoadInfo,
    keep_loaded: bool,
    unicode: bool,
) -> &'static Host {
    if !info.is_null() {
        // SAFETY: forwarded from the caller.
        let host = unsafe { Host::from_load_info(info, keep_loaded, unicode) };
        // mIRC calls `LoadDll` once per load, and statics start fresh with each load.
        let _ = HOST.set(host);
    }
    host()
}

impl Host {
    /// A host that never called `LoadDll`: mIRC 5.6 – 5.71. They pass ANSI strings, hold
    /// 900 characters, and unload the DLL after every call.
    fn legacy(main_window: Option<WindowHandle>) -> Self {
        let main_window = main_window.unwrap_or(WindowHandle(ptr::null_mut()));
        Self {
            client: Client::of_window(main_window),
            version: legacy_version(),
            beta: 0,
            main_window,
            encoding: Encoding::Ansi,
            capacity: buffer_capacity(Version::V5_6, None),
            keep_loaded: false,
        }
    }

    /// Reads `LOADINFO` and writes back `mKeep` and (on 7.0+) `mUnicode`.
    ///
    /// # Safety
    ///
    /// `info` must point to a `LOADINFO` at least as large as the version it reports.
    unsafe fn from_load_info(info: *mut sys::LoadInfo, keep_loaded: bool, unicode: bool) -> Self {
        // Fields are accessed one at a time through raw pointers: older mIRC versions pass
        // a shorter struct (12 bytes on the stack before 7.0), so a reference to the whole
        // `LoadInfo` would cover memory that isn't ours, and writing `mUnicode` there
        // would corrupt mIRC's stack.
        // SAFETY: `mVersion`, `mHwnd` and `mKeep` exist in every version that calls `LoadDll`.
        let (raw_version, hwnd) = unsafe {
            (&raw mut (*info).m_keep).write(keep_loaded.into());
            (
                (&raw const (*info).m_version).read(),
                (&raw const (*info).m_hwnd).read(),
            )
        };
        let version = Version::from_raw(raw_version);

        let encoding = if version >= Version::V7_0 {
            // SAFETY: `mUnicode` exists from 7.0.
            unsafe { (&raw mut (*info).m_unicode).write(unicode.into()) };
            if unicode {
                Encoding::Utf16
            } else {
                Encoding::Utf8
            }
        } else {
            Encoding::Ansi
        };

        // SAFETY: `mBeta` exists from 7.51.
        let beta =
            (version >= Version::V7_51).then(|| unsafe { (&raw const (*info).m_beta).read() });
        // SAFETY: `mBytes` exists from 7.64.
        let bytes =
            (version >= Version::V7_64).then(|| unsafe { (&raw const (*info).m_bytes).read() });

        Self {
            client: Client::of_window(WindowHandle(hwnd)),
            version,
            beta: beta.unwrap_or(0),
            main_window: WindowHandle(hwnd),
            encoding,
            capacity: buffer_capacity(version, bytes),
            keep_loaded,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn v(major: u16, minor: u16) -> Version {
        Version::new(major, minor)
    }

    #[test]
    fn capacity_by_version() {
        assert_eq!(buffer_capacity(Version::V5_6, None), 900);
        assert_eq!(buffer_capacity(v(5, 80), None), 900);
        assert_eq!(buffer_capacity(v(6, 3), None), 900);
        assert_eq!(buffer_capacity(v(6, 31), None), 900);
        assert_eq!(buffer_capacity(v(6, 32), None), 4151);
        assert_eq!(buffer_capacity(v(7, 0), None), 4151);
        assert_eq!(buffer_capacity(v(7, 52), None), 4151);
        assert_eq!(buffer_capacity(v(7, 53), None), 4200);
        assert_eq!(buffer_capacity(v(7, 63), None), 4200);
    }

    #[test]
    fn capacity_from_mbytes() {
        // 7.64 – 7.83 (and AdiIRC 4.4+, which reports 7.64 / 20480) doubled the value.
        assert_eq!(buffer_capacity(v(7, 64), Some(20480)), 10240);
        assert_eq!(buffer_capacity(v(7, 83), Some(20480)), 10240);
        assert_eq!(buffer_capacity(v(7, 84), Some(10240)), 10240);
        assert_eq!(buffer_capacity(v(7, 85), Some(10240)), 10240);
        assert_eq!(buffer_capacity(v(8, 0), Some(16384)), 16384);
    }

    #[test]
    fn capacity_ignores_missing_mbytes() {
        assert_eq!(buffer_capacity(v(7, 85), Some(0)), 4200);
        assert_eq!(buffer_capacity(v(7, 70), None), 4200);
    }

    #[test]
    fn reads_modern_loadinfo() {
        let mut info = sys::LoadInfo {
            m_version: Version::new(7, 85).to_raw(),
            m_hwnd: ptr::null_mut(),
            m_keep: 1,
            m_unicode: 0,
            m_beta: 0,
            m_bytes: 10240,
        };
        let host = unsafe { Host::from_load_info(&mut info, false, true) };
        assert_eq!(info.m_keep, 0);
        assert_eq!(info.m_unicode, 1);
        assert_eq!(host.version(), v(7, 85));
        assert_eq!(host.beta(), 0);
        assert!(host.unicode());
        assert_eq!(host.encoding(), Encoding::Utf16);
        assert_eq!(host.capacity(), 10240);
        assert!(!host.keep_loaded());
    }

    #[test]
    fn ansi_mode_on_mirc_7_is_utf8() {
        let mut info = sys::LoadInfo {
            m_version: Version::new(7, 42).to_raw(),
            m_hwnd: ptr::null_mut(),
            m_keep: 1,
            m_unicode: 0,
            m_beta: 0,
            m_bytes: 0,
        };
        let host = unsafe { Host::from_load_info(&mut info, true, false) };
        assert_eq!(info.m_unicode, 0);
        assert_eq!(host.encoding(), Encoding::Utf8);
        assert_eq!(host.capacity(), 4151);
    }

    #[test]
    fn touches_only_the_12_byte_prefix_on_mirc_6() {
        #[repr(C)]
        struct Mirc6LoadInfo {
            version: u32,
            hwnd: *mut c_void,
            keep: i32,
            // Stands in for mIRC's saved registers that follow the struct on its stack.
            beyond: [u32; 3],
        }
        let mut info = Mirc6LoadInfo {
            version: 3 << 16, // mIRC 6.03 reports "0.3"
            hwnd: ptr::null_mut(),
            keep: 1,
            beyond: [0xDEAD_BEEF; 3],
        };
        let host = unsafe { Host::from_load_info((&raw mut info).cast(), false, true) };
        assert_eq!(host.version(), v(6, 3));
        assert_eq!(host.encoding(), Encoding::Ansi);
        assert_eq!(host.capacity(), 900);
        assert_eq!(info.keep, 0);
        assert_eq!(info.beyond, [0xDEAD_BEEF; 3]);
    }
}

#[cfg(test)]
mod legacy_tests {
    use super::*;

    #[test]
    fn hosts_without_loaddll_are_conservative_mirc_5_6() {
        let host = Host::legacy(None);
        assert_eq!(host.version(), Version::V5_6);
        assert_eq!(host.encoding(), Encoding::Ansi);
        assert_eq!(host.capacity(), 900);
        assert!(!host.keep_loaded());
        assert!(!host.unicode());
        assert_eq!(host.beta(), 0);
        assert!(host.main_window().is_null());
    }
}

#[cfg(test)]
mod file_version_tests {
    use super::*;

    fn utf16(text: &str) -> Vec<u8> {
        text.encode_utf16().flat_map(u16::to_le_bytes).collect()
    }

    /// A `String` entry as it appears inside a `VS_VERSIONINFO` resource.
    fn string_entry(key: &str, value: &str) -> Vec<u8> {
        let mut entry = vec![0u8; 6]; // wLength, wValueLength, wType
        entry.extend(utf16(key));
        entry.extend([0, 0]);
        while entry.len() % 4 != 0 {
            entry.push(0);
        }
        entry.extend(utf16(value));
        entry.extend([0, 0]);
        entry
    }

    #[test]
    fn finds_file_version_among_other_strings() {
        let mut resource = string_entry("CompanyName", "mIRC Co. Ltd.");
        while resource.len() % 4 != 0 {
            resource.push(0);
        }
        resource.extend(string_entry("FileVersion", "5.61"));
        assert_eq!(
            version_resource_string(&resource, "FileVersion").as_deref(),
            Some("5.61")
        );
        assert_eq!(
            version_resource_string(&resource, "CompanyName").as_deref(),
            Some("mIRC Co. Ltd.")
        );
        assert_eq!(
            version_resource_string(&string_entry("ProductName", "mIRC"), "FileVersion"),
            None
        );
    }

    #[test]
    fn parses_mirc_style_versions() {
        assert_eq!(parse_version("5.6"), Some(Version::new(5, 60)));
        assert_eq!(parse_version("5.61"), Some(Version::new(5, 61)));
        assert_eq!(parse_version("5.7"), Some(Version::new(5, 70)));
        assert_eq!(parse_version("5.71"), Some(Version::new(5, 71)));
        assert_eq!(parse_version("7.85 beta"), Some(Version::new(7, 85)));
        assert_eq!(parse_version("5"), None);
        assert_eq!(parse_version("5.678"), None);
    }

    #[test]
    fn test_runner_is_not_a_legacy_host() {
        // The test executable has no mIRC 5.x version resource: fall back to 5.6.
        assert_eq!(legacy_version(), Version::V5_6);
    }
}

#[cfg(test)]
mod client_tests {
    use super::*;

    fn utf16(text: &str) -> Vec<u16> {
        text.encode_utf16().collect()
    }

    #[test]
    fn mirc_window_classes_are_mirc_and_anything_else_is_adiirc() {
        assert_eq!(Client::from_class(&utf16("mIRC")), Client::Mirc);
        assert_eq!(Client::from_class(&utf16("mIRC32")), Client::Mirc);
        // Exact matches only.
        assert_eq!(Client::from_class(&utf16("mirc")), Client::AdiIrc);
        assert_eq!(Client::from_class(&utf16("mIRC_Channel")), Client::AdiIrc);
        // AdiIRC's .NET windows have generated class names.
        assert_eq!(
            Client::from_class(&utf16("WindowsForms10.Window.8.app.0.141b42a_r6_ad1")),
            Client::AdiIrc
        );
    }
}
