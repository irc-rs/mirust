use core::ffi::c_void;
use core::ptr;

use crate::sys;

/// How mIRC encodes the strings it exchanges with the DLL.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Encoding {
    /// UTF-16, used when the DLL sets `mUnicode` (mIRC 7.0+). The default with mirust.
    Utf16,
    /// UTF-8 bytes. mIRC 7+ sends this to DLLs that stay in ANSI mode: any string with a
    /// non-ASCII character is UTF-8 encoded, and bytes coming back are UTF-8 decoded.
    Utf8,
    /// The system ANSI code page (`CP_ACP`), used by mIRC 6 and earlier.
    Ansi,
}

impl Encoding {
    /// Size in bytes of one code unit: 2 for [`Utf16`](Self::Utf16), 1 otherwise.
    pub const fn unit_size(self) -> usize {
        match self {
            Self::Utf16 => 2,
            Self::Utf8 | Self::Ansi => 1,
        }
    }
}

/// Reads a NUL-terminated string from an mIRC buffer.
///
/// Stops at the first NUL or after `capacity` code units, whichever comes first, and never
/// reads past the terminator.
///
/// # Safety
///
/// `ptr` must be null, or valid for reads up to its NUL terminator or `capacity` code units
/// of `encoding`, whichever is shorter.
pub(crate) unsafe fn read(ptr: *const c_void, capacity: usize, encoding: Encoding) -> String {
    if ptr.is_null() {
        return String::new();
    }
    match encoding {
        Encoding::Utf16 => {
            // SAFETY: forwarded from the caller.
            let units = unsafe { terminated_slice(ptr.cast::<u16>(), capacity) };
            String::from_utf16_lossy(units)
        }
        Encoding::Utf8 => {
            // SAFETY: forwarded from the caller.
            decode_utf8(unsafe { terminated_slice(ptr.cast::<u8>(), capacity) })
        }
        Encoding::Ansi => {
            // SAFETY: forwarded from the caller.
            decode_ansi(unsafe { terminated_slice(ptr.cast::<u8>(), capacity) })
        }
    }
}

/// Writes `s` into an mIRC buffer as a NUL-terminated string, truncating it at a character
/// boundary so that it fits in `capacity` code units including the terminator.
///
/// # Safety
///
/// `ptr` must be null, or valid for writes of `capacity` code units of `encoding`.
pub(crate) unsafe fn write(s: &str, ptr: *mut c_void, capacity: usize, encoding: Encoding) {
    if ptr.is_null() || capacity == 0 {
        return;
    }
    let max_len = capacity - 1;
    match encoding {
        Encoding::Utf16 => {
            let units = encode_utf16_truncated(s, max_len);
            // SAFETY: `units.len() <= max_len`, so the copy plus terminator fits in `capacity`.
            unsafe { copy_terminated(&units, ptr.cast::<u16>()) }
        }
        Encoding::Utf8 => {
            let bytes = truncate_utf8(s, max_len).as_bytes();
            // SAFETY: as above.
            unsafe { copy_terminated(bytes, ptr.cast::<u8>()) }
        }
        Encoding::Ansi => {
            let bytes = encode_ansi_truncated(s, max_len);
            // SAFETY: as above.
            unsafe { copy_terminated(&bytes, ptr.cast::<u8>()) }
        }
    }
}

/// # Safety
///
/// `ptr` must be valid for reads up to its NUL terminator or `max` elements.
unsafe fn terminated_slice<'a, T: Copy + Default + PartialEq>(
    ptr: *const T,
    max: usize,
) -> &'a [T] {
    let nul = T::default();
    let mut len = 0;
    // SAFETY: we read one element at a time and stop at the terminator, so we never touch
    // memory beyond what the caller vouched for.
    while len < max && unsafe { ptr.add(len).read() } != nul {
        len += 1;
    }
    // SAFETY: the `len` elements just read are initialised and readable.
    unsafe { core::slice::from_raw_parts(ptr, len) }
}

/// # Safety
///
/// `dst` must be valid for writes of `src.len() + 1` elements.
unsafe fn copy_terminated<T: Copy + Default>(src: &[T], dst: *mut T) {
    // SAFETY: forwarded from the caller; `src` is a Rust slice and cannot overlap mIRC's buffer.
    unsafe {
        ptr::copy_nonoverlapping(src.as_ptr(), dst, src.len());
        dst.add(src.len()).write(T::default());
    }
}

/// Decodes bytes the way mIRC 7 does: as UTF-8 when valid, otherwise byte-for-byte as
/// Latin-1 so that no input is lost.
fn decode_utf8(bytes: &[u8]) -> String {
    match core::str::from_utf8(bytes) {
        Ok(s) => s.to_owned(),
        Err(_) => bytes.iter().map(|&b| char::from(b)).collect(),
    }
}

fn truncate_utf8(s: &str, max_len: usize) -> &str {
    if s.len() <= max_len {
        return s;
    }
    let end = (0..=max_len)
        .rev()
        .find(|&i| s.is_char_boundary(i))
        .unwrap_or(0);
    &s[..end]
}

fn encode_utf16_truncated(s: &str, max_len: usize) -> Vec<u16> {
    let mut units = Vec::with_capacity(s.len().min(max_len));
    let mut buf = [0u16; 2];
    for c in s.chars() {
        let encoded = c.encode_utf16(&mut buf);
        if units.len() + encoded.len() > max_len {
            break;
        }
        units.extend_from_slice(encoded);
    }
    units
}

fn decode_ansi(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        return String::new();
    }
    let Ok(len) = i32::try_from(bytes.len()) else {
        return String::new();
    };
    // SAFETY: two-pass conversion; the first call only measures, the second writes into a
    // buffer of exactly the measured size.
    unsafe {
        let needed =
            sys::MultiByteToWideChar(sys::CP_ACP, 0, bytes.as_ptr(), len, ptr::null_mut(), 0);
        if needed <= 0 {
            return String::new();
        }
        let mut wide = vec![0u16; needed as usize];
        let written = sys::MultiByteToWideChar(
            sys::CP_ACP,
            0,
            bytes.as_ptr(),
            len,
            wide.as_mut_ptr(),
            needed,
        );
        wide.truncate(written.max(0) as usize);
        String::from_utf16_lossy(&wide)
    }
}

/// Converts UTF-16 to the ANSI code page. Returns `None` if Windows rejects the input.
fn wide_to_ansi(wide: &[u16]) -> Option<Vec<u8>> {
    if wide.is_empty() {
        return Some(Vec::new());
    }
    let len = i32::try_from(wide.len()).ok()?;
    // SAFETY: two-pass conversion as in `decode_ansi`. Unmappable characters become the
    // code page's default character.
    unsafe {
        let needed = sys::WideCharToMultiByte(
            sys::CP_ACP,
            0,
            wide.as_ptr(),
            len,
            ptr::null_mut(),
            0,
            ptr::null(),
            ptr::null_mut(),
        );
        if needed <= 0 {
            return None;
        }
        let mut bytes = vec![0u8; needed as usize];
        let written = sys::WideCharToMultiByte(
            sys::CP_ACP,
            0,
            wide.as_ptr(),
            len,
            bytes.as_mut_ptr(),
            needed,
            ptr::null(),
            ptr::null_mut(),
        );
        bytes.truncate(written.max(0) as usize);
        Some(bytes)
    }
}

fn encode_ansi_truncated(s: &str, max_len: usize) -> Vec<u8> {
    let wide: Vec<u16> = s.encode_utf16().collect();
    let Some(bytes) = wide_to_ansi(&wide) else {
        return Vec::new();
    };
    if bytes.len() <= max_len {
        return bytes;
    }
    // Too long. Convert one character at a time so a multi-byte (DBCS) character is never
    // split. The ANSI code pages are stateless, so per-character output concatenates to the
    // whole-string output.
    let mut out = Vec::with_capacity(max_len);
    let mut buf = [0u16; 2];
    for c in s.chars() {
        let Some(encoded) = wide_to_ansi(c.encode_utf16(&mut buf)) else {
            break;
        };
        if out.len() + encoded.len() > max_len {
            break;
        }
        out.extend_from_slice(&encoded);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(s: &str, capacity: usize, encoding: Encoding) -> String {
        let mut buf = vec![0xAAu8; capacity * encoding.unit_size() + 8];
        let ptr = buf.as_mut_ptr().cast::<c_void>();
        unsafe {
            write(s, ptr, capacity, encoding);
            read(ptr, capacity, encoding)
        }
    }

    #[test]
    fn round_trips_within_capacity() {
        for encoding in [Encoding::Utf16, Encoding::Utf8, Encoding::Ansi] {
            assert_eq!(round_trip("hello world", 64, encoding), "hello world");
            assert_eq!(round_trip("", 64, encoding), "");
        }
        assert_eq!(round_trip("héllo 🦀", 64, Encoding::Utf16), "héllo 🦀");
        assert_eq!(round_trip("héllo 🦀", 64, Encoding::Utf8), "héllo 🦀");
    }

    #[test]
    fn truncates_to_capacity_including_terminator() {
        for encoding in [Encoding::Utf16, Encoding::Utf8, Encoding::Ansi] {
            assert_eq!(round_trip("abcdef", 4, encoding), "abc");
            assert_eq!(round_trip("abcdef", 1, encoding), "");
        }
    }

    #[test]
    fn never_writes_past_capacity() {
        for encoding in [Encoding::Utf16, Encoding::Utf8, Encoding::Ansi] {
            let unit = encoding.unit_size();
            let mut buf = vec![0xAAu8; 16 * unit];
            unsafe {
                write(
                    "abcdefghijklmnopqrstuvwxyz",
                    buf.as_mut_ptr().cast(),
                    8,
                    encoding,
                )
            };
            assert!(buf[8 * unit..].iter().all(|&b| b == 0xAA), "{encoding:?}");
            assert!(
                buf[7 * unit..8 * unit].iter().all(|&b| b == 0),
                "{encoding:?}"
            );
        }
    }

    #[test]
    fn does_not_split_characters() {
        // 🦀 is a surrogate pair in UTF-16 and 4 bytes in UTF-8.
        assert_eq!(round_trip("a🦀", 3, Encoding::Utf16), "a");
        assert_eq!(round_trip("a🦀", 4, Encoding::Utf16), "a🦀");
        assert_eq!(round_trip("a🦀", 5, Encoding::Utf8), "a");
        assert_eq!(round_trip("a🦀", 6, Encoding::Utf8), "a🦀");
    }

    #[test]
    fn reads_stop_at_capacity_without_terminator() {
        let buf = *b"abcdef";
        let s = unsafe { read(buf.as_ptr().cast(), 3, Encoding::Utf8) };
        assert_eq!(s, "abc");
    }

    #[test]
    fn invalid_utf8_falls_back_to_latin1() {
        assert_eq!(decode_utf8(b"caf\xe9"), "café");
    }

    #[test]
    fn null_pointers_are_ignored() {
        unsafe {
            assert_eq!(read(ptr::null(), 10, Encoding::Utf16), "");
            write("x", ptr::null_mut(), 10, Encoding::Utf16);
        }
    }
}
