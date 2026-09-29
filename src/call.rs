use crate::{Host, StopToken, WindowHandle};

/// One invocation of an exported function by `/dll`, `$dll()` or `$dllcall()`.
#[derive(Debug)]
pub struct Call {
    pub(crate) data: String,
    pub(crate) main_window: WindowHandle,
    pub(crate) active_window: WindowHandle,
    pub(crate) show: bool,
    pub(crate) no_pause: bool,
    pub(crate) is_dllcall: bool,
    pub(crate) stop: StopToken,
    pub(crate) host: &'static Host,
}

impl Call {
    /// The text passed by the script: `[data]` in `/dll <file> <proc> [data]`.
    pub fn data(&self) -> &str {
        &self.data
    }

    /// Consumes the call, returning [`data`](Self::data).
    pub fn into_data(self) -> String {
        self.data
    }

    /// mIRC's main window.
    pub fn main_window(&self) -> WindowHandle {
        self.main_window
    }

    /// The window the command was issued from. For remote scripts this may not be the
    /// active window.
    pub fn active_window(&self) -> WindowHandle {
        self.active_window
    }

    /// Whether the call was made quietly, with the `.` prefix (`/.dll`).
    pub fn is_quiet(&self) -> bool {
        !self.show
    }

    /// Whether mIRC is in a critical routine. If so, don't do anything that pauses mIRC,
    /// such as opening a dialog.
    pub fn no_pause(&self) -> bool {
        self.no_pause
    }

    /// Whether this call runs on a `$dllcall()` worker thread rather than mIRC's UI thread.
    ///
    /// Blocking is fine here. When the function returns, mIRC runs a
    /// [`Response::Command`](crate::Response::Command), if you returned one, and then
    /// calls the script's alias with the DLL's file name as `$1-`. A
    /// [`Response::Return`](crate::Response::Return) value isn't passed to the alias.
    /// (Verified in mIRC 7.83.)
    ///
    /// When this is `false`, mIRC's UI is frozen until you return.
    pub fn is_dllcall(&self) -> bool {
        self.is_dllcall
    }

    /// Stopped when mIRC exits (or unloads the DLL) while this call is running.
    ///
    /// Only useful in a long-running `$dllcall()`: mIRC never exits or unloads the DLL
    /// during a `$dll()` or `/dll` call. A `$dllcall()` still running at exit gets up to
    /// [`Config::exit_grace`](crate::Config::exit_grace) to notice and return; see
    /// [`worker`](crate::worker#exit-grace-period).
    ///
    /// ```
    /// use std::time::Duration;
    /// use mirust::{Call, Response};
    ///
    /// fn poll(call: Call) -> Response {
    ///     let stop = call.stop_token();
    ///     for _ in 0..60 {
    ///         if stop.wait_timeout(Duration::from_secs(1)) {
    ///             return Response::Halt; // mIRC is exiting
    ///         }
    ///         // poll something
    ///     }
    ///     Response::Continue
    /// }
    /// # mirust::export!(poll);
    /// # fn main() {}
    /// ```
    pub fn stop_token(&self) -> &StopToken {
        &self.stop
    }

    /// The client that loaded the DLL.
    pub fn host(&self) -> &'static Host {
        self.host
    }

    /// Longest response, in code units of [`Host::encoding`], that fits without truncation.
    pub fn max_response_len(&self) -> usize {
        self.host.capacity().saturating_sub(1)
    }
}
