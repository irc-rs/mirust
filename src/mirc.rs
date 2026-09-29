//! Running commands and evaluating identifiers in mIRC, from any thread.
//!
//! mIRC accepts commands and evaluations from DLLs and other programs through its
//! documented SendMessage interface: the text goes in a named shared-memory block (a
//! "mapped file"), and a `WM_MCOMMAND` or `WM_MEVALUATE` message tells mIRC which one to
//! read. This module wraps that interface:
//!
//! ```no_run
//! # fn main() -> Result<(), mirust::mirc::SendError> {
//! use mirust::mirc;
//!
//! mirc::command("echo -a Hello from Rust")?; // runs /echo -a Hello from Rust
//! let version = mirc::evaluate("$version")?;
//!
//! // Options, such as running a command in a particular window:
//! # let window = mirust::host().main_window();
//! mirc::Command::new("say hi").window(window).flood_protection().send()?;
//! # Ok(())
//! # }
//! ```
//!
//! # Guarantees
//!
//! - **Calls don't affect each other.** On mIRC 6.2 and later, every call gets its own
//!   uniquely numbered mapped file (`mIRC<N>`), created exclusively: if another program
//!   already uses a name, the next number is tried. Numbers are never reused while the DLL
//!   is loaded, so even a call that timed out can't be confused with a later one. Calls in
//!   a row, or at the same time from different threads, never share memory. mIRC before
//!   6.2 only reads a mapped file named `mIRC`; there, calls from this DLL take turns.
//! - **Nothing is truncated.** Text that doesn't fit is rejected with
//!   [`SendError::TooLong`] rather than cut short, which could change what a command does.
//! - **Every failure is an error value.** Nothing panics, blocks forever or leaks: the
//!   mapped file is always released.
//!
//! # Threads
//!
//! Any thread may call these functions. On mIRC's UI thread (in `/dll`, `$dll()`,
//! `on_load` or `on_unload`), mIRC runs the request immediately, before the function
//! returns. On any other thread (`$dllcall()`, or a worker from [`spawn`](crate::spawn)),
//! the request waits for mIRC's UI thread, for up to the [timeout](Command::timeout)
//! (5 seconds by default).
//!
//! While mIRC is exiting, requests from other threads fail at once with
//! [`SendError::Exiting`]: mIRC's UI thread is then waiting for background work to finish
//! and couldn't answer.
//!
//! # Versions
//!
//! | mIRC | Support |
//! |------|---------|
//! | before 5.9 | none: [`SendError::Unsupported`] |
//! | 5.9 – 6.17 | commands and evaluations, ANSI text, one mapped file name (`mIRC`) |
//! | 6.2 – 6.35 | a unique mapped file per call |
//! | 7.0 – 7.32 | Unicode text |
//! | 7.33+ | [event context](Command::event_id), and specific errors in [`SendError::Failed`] |
//! | 7.34+ | [`SendError::Disabled`] when SendMessage is turned off |
//!
//! Before 7.33, mIRC reports every failure the same way, and reports some failed commands
//! as successful. mIRC's user can also turn the interface off (Options → Other → "Enable
//! SendMessage Server", or the Lock dialog).

use core::ffi::c_void;
use core::fmt;
use core::ptr;
use std::io;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};
use std::thread;
use std::time::Duration;

use crate::{Client, Encoding, Version, WindowHandle, encoding, host, sys, worker};

const WM_MCOMMAND: u32 = sys::WM_USER + 200;
const WM_MEVALUATE: u32 = sys::WM_USER + 201;

// `cMethod` flags, in the low word of `wParam`.
const METHOD_EDITBOX: u16 = 1;
const METHOD_PLAIN_TEXT: u16 = 2;
const METHOD_FLOOD_PROTECTION: u16 = 4;
const METHOD_UNICODE: u16 = 8;
const METHOD_EXTENDED_ERRORS: u16 = 16;

/// Extended error bit meaning SendMessage is disabled (mIRC 7.34+).
const ERROR_DISABLED: usize = 64;

/// Size of each mapped file. mIRC requires at least 4096 bytes; this holds any command or
/// result current mIRC allows, with room to spare.
const MAP_BYTES: usize = 64 * 1024;

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);

const V5_9: Version = Version::new(5, 90);
const V6_2: Version = Version::new(6, 20);
const V7_33: Version = Version::new(7, 33);

/// Why a request to mIRC failed.
#[derive(Debug)]
#[non_exhaustive]
pub enum SendError {
    /// This mIRC version can't do what was asked: SendMessage needs mIRC 5.9, and an
    /// [event context](Command::event_id) needs 7.33.
    Unsupported(&'static str),
    /// There is no mIRC window to send to, or it was destroyed while the request waited:
    /// for example because mIRC closed. (mIRC before 6.3 reports its exit as an ordinary
    /// unload, so a request made then can end this way instead of with
    /// [`Exiting`](Self::Exiting).)
    NoWindow,
    /// The text contains a NUL character, which would end it early.
    InvalidText,
    /// The text is longer than the mapped file holds.
    TooLong {
        /// Length of the text, in code units (UTF-16 units, or bytes before mIRC 7.0).
        len: usize,
        /// The most the mapped file holds, in the same units.
        max: usize,
    },
    /// No mapped file was free. On mIRC before 6.2, another program was using the single
    /// name those versions allow, `mIRC`. Try again later.
    Busy,
    /// mIRC is exiting, so the request wasn't sent.
    Exiting,
    /// mIRC didn't answer within the timeout, or is not responding.
    Timeout,
    /// SendMessage is turned off in mIRC (Options → Other → "Enable SendMessage Server", or
    /// the Lock dialog). Reported by mIRC 7.34 and later; older versions report it as
    /// [`Failed`](Self::Failed).
    Disabled,
    /// mIRC reported a failure: for example an error in the command or identifier (mIRC
    /// passes unknown commands to the IRC server, so those fail with a server error), or,
    /// before mIRC 7.34, SendMessage being turned off.
    ///
    /// From mIRC 7.33, `code` holds mIRC's detailed error value: 1, combined with 2 for a
    /// bad mapped file name, 4 for a bad mapped file size, 8 for a bad event id, 16 when the
    /// event's server no longer exists, and 32 when its script no longer exists. Just 1
    /// means the command or identifier itself failed. Earlier versions give no detail.
    Failed {
        /// mIRC's detailed error value, on mIRC 7.33 and later.
        code: Option<u32>,
    },
    /// A Windows call failed.
    System(io::Error),
}

impl fmt::Display for SendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported(why) => write!(f, "not supported by this mIRC version: {why}"),
            Self::NoWindow => f.write_str("no mIRC window to send to"),
            Self::InvalidText => f.write_str("text contains a NUL character"),
            Self::TooLong { len, max } => write!(f, "text is {len} units long; the most is {max}"),
            Self::Busy => f.write_str("mIRC's mapped file is in use by another program"),
            Self::Exiting => f.write_str("mIRC is exiting"),
            Self::Timeout => f.write_str("mIRC didn't answer in time"),
            Self::Disabled => f.write_str("SendMessage is disabled in mIRC"),
            Self::Failed { code: Some(code) } => write!(f, "mIRC reported a failure (code {code})"),
            Self::Failed { code: None } => f.write_str("mIRC reported a failure"),
            Self::System(error) => write!(f, "Windows error: {error}"),
        }
    }
}

impl std::error::Error for SendError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::System(error) => Some(error),
            _ => None,
        }
    }
}

/// Runs the command `text` in mIRC, as if typed in the main window's editbox.
///
/// A leading `/` is added if missing, so `"echo -a hi"` runs `/echo -a hi`. Identifiers and
/// variables in the text are not evaluated, just as when typing `/echo $me`; start the text
/// with `//` to have mIRC evaluate them first. Shorthand for `Command::new(text).send()`; see
/// [`Command`] for options.
///
/// # Errors
///
/// See [`SendError`].
pub fn command(text: &str) -> Result<(), SendError> {
    Command::new(text).send()
}

/// Evaluates `text` in mIRC, such as `"$version"` or `"$calc(1 + 2)"`, and returns the
/// result.
///
/// Shorthand for `Evaluate::new(text).send()`; see [`Evaluate`] for options.
///
/// # Errors
///
/// See [`SendError`].
pub fn evaluate(text: &str) -> Result<String, SendError> {
    Evaluate::new(text).send()
}

/// A command to run in mIRC, with options.
///
/// The text runs as if typed in a window's editbox. mirust adds a leading `/` if it is
/// missing, so `"echo -a hi"` runs `/echo -a hi`. As when typing, a single `/` leaves
/// identifiers and variables unevaluated (`/echo $me` prints `$me`), which keeps text that
/// includes user input from being evaluated by accident. Start the text with `//` to have
/// mIRC evaluate them first (`//echo $me` prints your nickname).
///
/// ```no_run
/// # fn main() -> Result<(), mirust::mirc::SendError> {
/// # let window = mirust::host().main_window();
/// use std::time::Duration;
/// use mirust::mirc::Command;
///
/// Command::new("say Hello")
///     .window(window)
///     .flood_protection()
///     .timeout(Duration::from_secs(1))
///     .send()?;
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug)]
#[must_use = "a Command does nothing until you call send()"]
pub struct Command {
    text: String,
    options: Options,
    plain_text: bool,
    flood_protection: bool,
}

impl Command {
    /// A command, run as if typed in the main window's editbox.
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            options: Options::default(),
            plain_text: false,
            flood_protection: false,
        }
    }

    /// Runs the command in `window` (a channel, query or other mIRC window) instead of the
    /// main window, as if typed in its editbox.
    pub fn window(mut self, window: WindowHandle) -> Self {
        self.options.window = Some(window);
        self
    }

    /// Sends the text as a plain message to the window's channel or query instead of
    /// running it as a command, even if it starts with `/`. No `/` is added.
    pub fn plain_text(mut self) -> Self {
        self.plain_text = true;
        self
    }

    /// Applies mIRC's flood protection, if the user has it turned on.
    pub fn flood_protection(mut self) -> Self {
        self.flood_protection = true;
        self
    }

    /// Runs the command in the context of the remote event whose `$eventid` this is, so
    /// that identifiers describing the event (`$nick`, `$chan`, `$signal` and so on) refer
    /// to it. The handler's own parameters (`$1-`) aren't part of the context; pass them in
    /// the text. The id is only valid while the event is running, so use it from the
    /// handler (for example in a `$dll()` it calls). Needs mIRC 7.33.
    pub fn event_id(mut self, event_id: u16) -> Self {
        self.options.event_id = Some(event_id);
        self
    }

    /// How long to wait for mIRC when called from a thread other than mIRC's UI thread.
    /// Default 5 seconds.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.options.timeout = timeout;
        self
    }

    /// Sends the command to mIRC.
    ///
    /// # Errors
    ///
    /// See [`SendError`].
    pub fn send(&self) -> Result<(), SendError> {
        let mut method = METHOD_EDITBOX;
        if self.plain_text {
            method |= METHOD_PLAIN_TEXT;
        }
        if self.flood_protection {
            method |= METHOD_FLOOD_PROTECTION;
        }
        let text = command_text(&self.text, self.plain_text);
        send(WM_MCOMMAND, method, &text, &self.options, false).map(drop)
    }
}

/// The text to send for a command: with a leading `/` unless it has one, or is plain text.
fn command_text(text: &str, plain_text: bool) -> std::borrow::Cow<'_, str> {
    if plain_text || text.starts_with('/') {
        text.into()
    } else {
        format!("/{text}").into()
    }
}

/// An evaluation in mIRC, with options.
#[derive(Clone, Debug)]
#[must_use = "an Evaluate does nothing until you call send()"]
pub struct Evaluate {
    text: String,
    options: Options,
}

impl Evaluate {
    /// Evaluates `text`, such as `"$version"`, in the main window.
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            options: Options::default(),
        }
    }

    /// Evaluates in `window` instead of the main window, which affects identifiers such as
    /// `$active` and `$chan`.
    pub fn window(mut self, window: WindowHandle) -> Self {
        self.options.window = Some(window);
        self
    }

    /// Evaluates in the context of the remote event whose `$eventid` this is, so that
    /// identifiers describing the event (`$nick`, `$chan`, `$signal` and so on) refer to it.
    /// `$1-` isn't part of the context. The id is only valid while the event is running.
    /// Needs mIRC 7.33.
    pub fn event_id(mut self, event_id: u16) -> Self {
        self.options.event_id = Some(event_id);
        self
    }

    /// How long to wait for mIRC when called from a thread other than mIRC's UI thread.
    /// Default 5 seconds.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.options.timeout = timeout;
        self
    }

    /// Sends the evaluation to mIRC and returns the result.
    ///
    /// # Errors
    ///
    /// See [`SendError`].
    pub fn send(&self) -> Result<String, SendError> {
        send(WM_MEVALUATE, 0, &self.text, &self.options, true).map(Option::unwrap_or_default)
    }
}

#[derive(Clone, Debug)]
struct Options {
    window: Option<WindowHandle>,
    event_id: Option<u16>,
    timeout: Duration,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            window: None,
            event_id: None,
            timeout: DEFAULT_TIMEOUT,
        }
    }
}

/// Sends one request and, for evaluations, reads the result.
fn send(
    message: u32,
    method: u16,
    text: &str,
    options: &Options,
    read_result: bool,
) -> Result<Option<String>, SendError> {
    let host = host();
    let version = host.version();
    if version < V5_9 {
        return Err(SendError::Unsupported(
            "SendMessage needs mIRC 5.9 or later",
        ));
    }
    if options.event_id.is_some() && version < V7_33 {
        return Err(SendError::Unsupported(
            "an event context needs mIRC 7.33 or later",
        ));
    }
    let window = options.window.unwrap_or(host.main_window());
    if window.is_null() {
        return Err(SendError::NoWindow);
    }
    // mIRC's UI thread handles a request from itself immediately; from another thread,
    // the request would wait for a UI thread that, while exiting, is waiting on us.
    if !window.is_current_thread() && worker::is_exiting() {
        return Err(SendError::Exiting);
    }

    let unicode = version >= Version::V7_0;
    let text_encoding = if unicode {
        Encoding::Utf16
    } else {
        Encoding::Ansi
    };
    let capacity = MAP_BYTES / text_encoding.unit_size();
    check_text(text, text_encoding, capacity)?;

    let extended_errors = version >= V7_33 && host.client() == Client::Mirc;
    let mut method = method;
    if unicode {
        method |= METHOD_UNICODE;
    }
    if extended_errors {
        method |= METHOD_EXTENDED_ERRORS;
    }

    let (mapping, index) = Mapping::acquire(version >= V6_2)?;
    // SAFETY: the view is `MAP_BYTES` long, which is `capacity` units, and `check_text`
    // made sure the text and its terminator fit.
    unsafe { encoding::write(text, mapping.view, capacity, text_encoding) };

    let reply = mapping.send(
        window,
        message,
        wparam(method, options.event_id),
        index,
        options.timeout,
    )?;
    outcome(reply, extended_errors)?;

    // SAFETY: mIRC writes a NUL-terminated result within the view, which is `capacity`
    // units long; the read never goes past it.
    Ok(read_result.then(|| unsafe { encoding::read(mapping.view, capacity, text_encoding) }))
}

/// Rejects text that would be cut short: containing NUL, or too long for the mapped file.
fn check_text(text: &str, text_encoding: Encoding, capacity: usize) -> Result<(), SendError> {
    if text.contains('\0') {
        return Err(SendError::InvalidText);
    }
    let len = encoding::encoded_len(text, text_encoding).ok_or(SendError::InvalidText)?;
    let max = capacity - 1; // room for the terminator
    if len > max {
        return Err(SendError::TooLong { len, max });
    }
    Ok(())
}

/// `wParam`: `cMethod` in the low word, the event id in the high word.
fn wparam(method: u16, event_id: Option<u16>) -> usize {
    (u32::from(event_id.unwrap_or(0)) << 16 | u32::from(method)) as usize
}

/// Interprets mIRC's reply. With extended errors (7.33+) 0 is success; otherwise 1 is.
fn outcome(reply: usize, extended_errors: bool) -> Result<(), SendError> {
    if extended_errors {
        match reply {
            0 => Ok(()),
            code if code & ERROR_DISABLED != 0 => Err(SendError::Disabled),
            code => Err(SendError::Failed {
                code: Some(code as u32),
            }),
        }
    } else if reply != 0 {
        Ok(())
    } else {
        Err(SendError::Failed { code: None })
    }
}

/// Mapped file numbers for this load of the DLL. Each number is used at most once.
fn next_index() -> u32 {
    static SEED: OnceLock<u32> = OnceLock::new();
    static NEXT: AtomicU32 = AtomicU32::new(0);
    // Start somewhere different for each process and each load of the DLL, so that
    // separate DLLs (and separate mIRCs) rarely try the same names first.
    let seed = *SEED.get_or_init(|| {
        // SAFETY: plain queries with no arguments.
        let (ticks, pid) = unsafe { (sys::GetTickCount64(), sys::GetCurrentProcessId()) };
        (ticks as u32) ^ pid.rotate_left(16) ^ (ptr::from_ref(&NEXT) as usize as u32)
    });
    index_from(seed.wrapping_add(NEXT.fetch_add(1, Ordering::Relaxed)))
}

/// Maps any number onto 1 ..= 2^31 - 1: positive, and never 0, which means `mIRC` itself.
fn index_from(n: u32) -> u32 {
    1 + n % (i32::MAX as u32)
}

/// Serialises requests from this DLL on mIRC before 6.2, which only read `mIRC`.
static SINGLE_NAME: Mutex<()> = Mutex::new(());

/// An exclusively created mapped file, released on drop.
struct Mapping {
    handle: *mut c_void,
    view: *mut c_void,
    _turn: Option<MutexGuard<'static, ()>>,
}

impl Mapping {
    /// Creates a mapped file nobody else is using, and the `lParam` that names it.
    fn acquire(numbered: bool) -> Result<(Self, u32), SendError> {
        if numbered {
            for _ in 0..64 {
                let index = next_index();
                if let Some(mapping) = Self::create_new(&format!("mIRC{index}"))? {
                    return Ok((mapping, index));
                }
            }
            return Err(SendError::Busy);
        }
        let turn = SINGLE_NAME.lock().unwrap_or_else(PoisonError::into_inner);
        for attempt in 0..50 {
            if let Some(mut mapping) = Self::create_new("mIRC")? {
                mapping._turn = Some(turn);
                return Ok((mapping, 0));
            }
            if attempt < 49 {
                thread::sleep(Duration::from_millis(10));
            }
        }
        Err(SendError::Busy)
    }

    /// Creates the mapped file `name`, or returns `None` if it already exists (another
    /// program is using it) or belongs to someone else.
    fn create_new(name: &str) -> Result<Option<Self>, SendError> {
        let wide: Vec<u16> = name.encode_utf16().chain([0]).collect();
        // SAFETY: a paging-file-backed mapping with a NUL-terminated name. `GetLastError`
        // is read straight away, before anything else can change it.
        let (handle, error) = unsafe {
            let handle = sys::CreateFileMappingW(
                sys::INVALID_HANDLE_VALUE,
                ptr::null(),
                sys::PAGE_READWRITE,
                0,
                MAP_BYTES as u32,
                wide.as_ptr(),
            );
            (handle, sys::GetLastError())
        };
        const ERROR_ACCESS_DENIED: u32 = 5;
        if handle.is_null() {
            return if error == ERROR_ACCESS_DENIED {
                Ok(None)
            } else {
                Err(SendError::System(io::Error::from_raw_os_error(
                    error as i32,
                )))
            };
        }
        if error == sys::ERROR_ALREADY_EXISTS {
            // SAFETY: a handle we own and don't use again.
            unsafe { sys::CloseHandle(handle) };
            return Ok(None);
        }
        // SAFETY: a mapping we just created, `MAP_BYTES` long.
        let view = unsafe {
            sys::MapViewOfFile(
                handle,
                sys::FILE_MAP_READ | sys::FILE_MAP_WRITE,
                0,
                0,
                MAP_BYTES,
            )
        };
        if view.is_null() {
            let error = io::Error::last_os_error();
            // SAFETY: a handle we own and don't use again.
            unsafe { sys::CloseHandle(handle) };
            return Err(SendError::System(error));
        }
        Ok(Some(Self {
            handle,
            view,
            _turn: None,
        }))
    }

    /// Sends `message` to `window`, naming this mapped file, and returns mIRC's reply.
    fn send(
        &self,
        window: WindowHandle,
        message: u32,
        wparam: usize,
        index: u32,
        timeout: Duration,
    ) -> Result<usize, SendError> {
        let timeout_ms = u32::try_from(timeout.as_millis()).unwrap_or(u32::MAX);
        let mut reply = 0usize;
        // SAFETY: a plain message send. mIRC reads (and for evaluations writes) the mapped
        // file named by `index` before replying; it stays alive until after this returns.
        let sent = unsafe {
            sys::SendMessageTimeoutW(
                window.as_raw(),
                message,
                wparam,
                index as isize,
                sys::SMTO_BLOCK | sys::SMTO_ABORTIFHUNG | sys::SMTO_ERRORONEXIT,
                timeout_ms,
                &mut reply,
            )
        };
        if sent == 0 {
            // SAFETY: reads this thread's last error.
            let error = unsafe { sys::GetLastError() };
            return Err(match error {
                0 | sys::ERROR_TIMEOUT => SendError::Timeout,
                sys::ERROR_INVALID_WINDOW_HANDLE => SendError::NoWindow,
                _ => SendError::System(io::Error::from_raw_os_error(error as i32)),
            });
        }
        Ok(reply)
    }
}

impl Drop for Mapping {
    fn drop(&mut self) {
        // SAFETY: a view and handle we own, released once. mIRC has already replied, and
        // holds its own handle if it is still reading.
        unsafe {
            sys::UnmapViewOfFile(self.view);
            sys::CloseHandle(self.handle);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn wparam_puts_the_event_id_in_the_high_word() {
        assert_eq!(wparam(METHOD_EDITBOX, None), 1);
        assert_eq!(
            wparam(METHOD_EDITBOX | METHOD_UNICODE, Some(0x1234)),
            0x1234_0009
        );
        assert_eq!(wparam(0, Some(u16::MAX)), 0xFFFF_0000);
    }

    #[test]
    fn commands_get_a_leading_slash() {
        assert_eq!(command_text("echo -a hi", false), "/echo -a hi");
        assert_eq!(command_text("/echo -a hi", false), "/echo -a hi");
        assert_eq!(command_text("//echo -a $me", false), "//echo -a $me");
        // Plain text is sent as a message: never prefixed.
        assert_eq!(command_text("hello there", true), "hello there");
    }

    #[test]
    fn replies_are_read_by_version() {
        // Before 7.33: 1 is success, 0 is failure, with no detail.
        assert!(outcome(1, false).is_ok());
        assert!(matches!(
            outcome(0, false),
            Err(SendError::Failed { code: None })
        ));
        // 7.33+: 0 is success, anything else describes the failure.
        assert!(outcome(0, true).is_ok());
        assert!(matches!(
            outcome(1, true),
            Err(SendError::Failed { code: Some(1) })
        ));
        assert!(matches!(
            outcome(1 | 8, true),
            Err(SendError::Failed { code: Some(9) })
        ));
        assert!(matches!(outcome(1 | 64, true), Err(SendError::Disabled)));
    }

    #[test]
    fn text_that_would_be_cut_short_is_rejected() {
        assert!(check_text("echo hi", Encoding::Utf16, 100).is_ok());
        assert!(matches!(
            check_text("a\0b", Encoding::Utf16, 100),
            Err(SendError::InvalidText)
        ));
        assert!(check_text(&"a".repeat(99), Encoding::Utf16, 100).is_ok());
        assert!(matches!(
            check_text(&"a".repeat(100), Encoding::Utf16, 100),
            Err(SendError::TooLong { len: 100, max: 99 })
        ));
        // A surrogate pair takes two UTF-16 units.
        assert!(matches!(
            check_text(&"🦀".repeat(50), Encoding::Utf16, 100),
            Err(SendError::TooLong { len: 100, max: 99 })
        ));
    }

    #[test]
    fn mapped_file_numbers_are_positive_and_unique() {
        assert_eq!(index_from(0), 1);
        assert_eq!(index_from(i32::MAX as u32 - 1), i32::MAX as u32);
        assert_eq!(index_from(i32::MAX as u32), 1);
        let indexes: HashSet<u32> = (0..10_000).map(|_| next_index()).collect();
        assert_eq!(indexes.len(), 10_000);
        assert!(indexes.iter().all(|&n| (1..=i32::MAX as u32).contains(&n)));
    }

    #[test]
    fn mapped_files_are_created_exclusively() {
        // SAFETY: a plain query.
        let name = format!("mirust-test-{}", unsafe { sys::GetCurrentProcessId() });
        let first = Mapping::create_new(&name)
            .unwrap()
            .expect("name should be free");
        assert!(
            Mapping::create_new(&name).unwrap().is_none(),
            "second create must see it in use"
        );
        drop(first);
        assert!(
            Mapping::create_new(&name).unwrap().is_some(),
            "free again once released"
        );
    }

    #[test]
    fn unsupported_without_sendmessage() {
        // The test runner isn't mIRC: mirust treats it as mIRC 5.6, which predates
        // SendMessage.
        assert!(matches!(command("echo hi"), Err(SendError::Unsupported(_))));
        assert!(matches!(
            evaluate("$version"),
            Err(SendError::Unsupported(_))
        ));
    }
}
