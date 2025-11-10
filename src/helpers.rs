use windows::{Win32::Globalization::{CP_ACP, MB_ERR_INVALID_CHARS, MultiByteToWideChar, WideCharToMultiByte}};

pub fn pwstr_to_string(ptr: *const u16, maxlen: usize) -> String {
    if ptr.is_null() {
        return String::new();
    }

    let maxlen = maxlen & !1 / 2; // Convert byte length to number of u16 characters
    
    // Make sure we don't return the null terminator
    let slice = unsafe { std::slice::from_raw_parts(ptr, maxlen) };
    let len = slice.iter().position(|&c| c == 0).unwrap_or(maxlen);
    String::from_utf16_lossy(&slice[..len])
}

pub fn pstr_to_string(ptr: *const u8, maxlen: usize) -> String {
    if ptr.is_null() {
        return String::new();
    }

    let wide_vec = convert_ansi_to_wide_string(ptr, maxlen);
    pwstr_to_string(wide_vec.as_ptr(), wide_vec.len() * 2)
}

pub fn string_to_pwstr(s: &str, ptr: *const u16, maxlen: usize) {
    if ptr.is_null() {
        return;
    }

    let wide: Vec<u16> = s.encode_utf16().collect();
    let len = wide.len().min(maxlen / 2 - 1); // Leave space for null terminator

    unsafe {
        std::ptr::copy_nonoverlapping(wide.as_ptr(), ptr as *mut u16, len);
        *((ptr as *mut u16).add(len)) = 0; // null-terminate
    }
}

pub fn string_to_pstr(s: &str, ptr: *const u8, maxlen: usize) {
    if ptr.is_null() {
        return;
    }

    let wide: Vec<u16> = s.encode_utf16().collect(); // Null-terminated UTF-16
    let ansi_vec = convert_wide_to_ansi_string(wide.as_ptr(), wide.len());

    // We need to copy up to maxlen - 1 bytes to leave space for null terminator
    let len = ansi_vec.len().min(maxlen - 1);
    unsafe {
        std::ptr::copy_nonoverlapping(ansi_vec.as_ptr(), ptr as *mut u8, len);
        *((ptr as *mut u8).add(len)) = 0; // null-terminate
    }
}

/// Converts a null-terminated ANSI string (CP_ACP) to a new, null-terminated UTF-16 string.
///
/// This function uses the two-pass `MultiByteToWideChar` pattern to safely allocate
/// the exact buffer size required for the new wide string.
///
/// # Parameters
/// * `ansi_str_ptr`: A C-style pointer to a null-terminated ANSI string.
/// * `max_ansi_bytes`: A safety limit. The function will not read past this many
///   bytes from `ansi_str_ptr`.
///
/// # Returns
/// A `Vec<u16>` containing the UTF-16 representation of the string, guaranteed
/// to be terminated with a `0u16` null character.
///
/// If `ansi_str_ptr` is null, or if the conversion fails, this function
/// returns a `Vec<u16>` containing only a single null terminator (`vec![0]`).
fn convert_ansi_to_wide_string(
    ansi_str_ptr: *const u8,
    max_ansi_bytes: usize,
) -> Vec<u16> {
    // A null-terminated empty string is the safest default return.
    let null_terminated_empty_wide = || vec![0u16];

    if ansi_str_ptr.is_null() {
        return null_terminated_empty_wide();
    }

    // This operation remains unsafe as we are trusting the caller's pointer
    let ansi_buffer_slice =
        unsafe { std::slice::from_raw_parts(ansi_str_ptr, max_ansi_bytes) };

    // Find the length of the *content* (excluding the null terminator).
    let ansi_len_no_null = ansi_buffer_slice
        .iter()
        .position(|&byte| byte == 0)
        .unwrap_or(max_ansi_bytes);

    // Get a slice of only the content bytes.
    let ansi_content_slice = &ansi_buffer_slice[..ansi_len_no_null];

    if ansi_content_slice.is_empty() {
        // This handles the case where max_ansi_bytes is 0
        // or the input pointer is an empty string.
        return null_terminated_empty_wide();
    }

    // PASS 1: Determine the required buffer size (in u16s)
    let required_wide_chars = unsafe {
        MultiByteToWideChar(
            CP_ACP,
            MB_ERR_INVALID_CHARS,
            ansi_content_slice,
            None, // lpWideCharStr = None
        )
    };

    if required_wide_chars <= 0 {
        // This indicates an error.
        return null_terminated_empty_wide();
    }

    // Allocate the exact buffer size: content + 1 for the null.
    let mut wide_buffer: Vec<u16> =
        vec![0u16; (required_wide_chars + 1) as usize];

    // PASS 2: Perform the actual conversion.
    let chars_written = unsafe {
        MultiByteToWideChar(
            CP_ACP,
            MB_ERR_INVALID_CHARS,
            ansi_content_slice,
            Some(&mut wide_buffer), // Pass the mutable slice
        )
    };

    if chars_written <= 0 {
        // The conversion failed on the second pass.
        return null_terminated_empty_wide();
    }

    // Ensure null-termination (should already be null due to initialization).
    wide_buffer[chars_written as usize] = 0;

    // Truncate the vec to the exact final size: content + null.
    wide_buffer.truncate((chars_written + 1) as usize);

    wide_buffer
}

/// Converts a null-terminated UTF-16 string to a new, null-terminated ANSI string (CP_ACP).
///
/// This function uses the two-pass `WideCharToMultiByte` pattern to safely allocate
/// the exact buffer size required for the new ANSI string.
///
/// # Parameters
/// * `wide_str_ptr`: A C-style pointer to a null-terminated UTF-16 string.
/// * `max_wide_chars`: A safety limit. The function will not read past this many
///   `u16` characters from `wide_str_ptr`.
///
/// # Returns
/// A `Vec<u8>` containing the ANSI (CP_ACP) representation of the string, guaranteed
/// to be terminated with a `0u8` null character.
///
/// If `wide_str_ptr` is null, or if the conversion fails, this function
/// returns a `Vec<u8>` containing only a single null terminator (`vec![0]`).
pub fn convert_wide_to_ansi_string(
    wide_str_ptr: *const u16,
    max_wide_chars: usize,
) -> Vec<u8> {
    // A null-terminated empty string is the safest default return.
    let null_terminated_empty_ansi = || vec![0u8];

    if wide_str_ptr.is_null() {
        return null_terminated_empty_ansi();
    }

    // This operation remains unsafe as we are trusting the caller's pointer
    let wide_buffer_slice =
        unsafe { std::slice::from_raw_parts(wide_str_ptr, max_wide_chars) };

    // Find the length of the *content* (excluding the null terminator).
    let wide_len_no_null = wide_buffer_slice
        .iter()
        .position(|&wide_char| wide_char == 0)
        .unwrap_or(max_wide_chars);

    // Get a slice of only the content bytes.
    let wide_content_slice = &wide_buffer_slice[..wide_len_no_null];

    if wide_content_slice.is_empty() {
        // This handles the case where max_wide_chars is 0
        // or the input pointer is an empty string.
        return null_terminated_empty_ansi();
    }

    // PASS 1: Determine the required buffer size (in u8s)
    let required_ansi_bytes = unsafe {
        WideCharToMultiByte(
            CP_ACP,
            0,
            wide_content_slice,
            None, // lpMultiByteStr = None
            None, // lpDefaultChar = None
            None, // lpUsedDefaultChar = None
        )
    };

    if required_ansi_bytes <= 0 {
        // This indicates an error.
        return null_terminated_empty_ansi();
    }

    // Allocate the exact buffer size: content + 1 for the null.
    // We fill with 0s, so the buffer is already null-terminated.
    let mut ansi_buffer: Vec<u8> =
        vec![0u8; (required_ansi_bytes + 1) as usize];

    // PASS 2: Perform the actual conversion.
    // It will not write its own null, as we passed an explicit length.
    let bytes_written = unsafe {
        WideCharToMultiByte(
            CP_ACP,
            0,
            wide_content_slice,
            Some(&mut ansi_buffer), // Pass the mutable slice
            None, // lpDefaultChar = None
            None, // lpUsedDefaultChar = None
        )
    };

    if bytes_written <= 0 {
        // The conversion failed on the second pass.
        return null_terminated_empty_ansi();
    }

    // Explicitly set the null terminator after the written content.
    ansi_buffer[bytes_written as usize] = 0;

    // Truncate the vec to the exact final size: content + null.
    ansi_buffer.truncate((bytes_written + 1) as usize);

    ansi_buffer
}