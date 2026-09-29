/// What mIRC should do after an exported function returns.
///
/// Strings longer than the host's buffers are truncated at a character boundary; see
/// [`Call::max_response_len`](crate::Call::max_response_len).
///
/// # With `$dllcall()`
///
/// `$dllcall()` evaluates to `$null` at once and the script carries on, so by the time
/// your function returns there is nothing left to halt or return a value to. mIRC then
/// runs a [`Command`](Self::Command), if you returned one, and calls the script's callback
/// alias with the DLL's full path as `$1-`, whatever you returned:
///
/// | Response   | `/dll`, `$dll()`                 | `$dllcall()`                        |
/// |------------|----------------------------------|-------------------------------------|
/// | `Halt`     | halts the calling script         | no effect                           |
/// | `Continue` | carries on; `$dll()` is `$null`  | no effect                           |
/// | `Command`  | runs the command                 | runs the command, then the callback |
/// | `Return`   | `$dll()` evaluates to the value  | no effect; the value is lost        |
///
/// To get a result back from a `$dllcall()`, return a [`Command`](Self::Command) that runs
/// your callback with the result, such as `Response::command_with("fetch_done $1-", value)`,
/// and pass `noop` (from mIRC 6.17; before that, an alias of your own) as the `$dllcall()`
/// callback. (Verified in mIRC 7.83.)
///
/// **Returned commands are delivered reliably, even when `$dllcall()`s overlap.** mIRC
/// keeps each DLL's pending `$dllcall()` result in a single slot, which another call into
/// the same DLL can overwrite before mIRC processes it, losing the command. mirust queues
/// the command itself instead and has mIRC call back into the DLL to run it, so each
/// command runs exactly once, in the order the calls finished. mIRC's own `$dllcall()`
/// callback can still run for the wrong call, or twice, when calls overlap; that's why it
/// should be `noop`. The exception is [`Config::keep_loaded(false)`](crate::Config::keep_loaded),
/// where the DLL doesn't stay loaded long enough to queue anything. See the README for
/// details.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Response {
    /// Halt the calling script, as `/halt` does. mIRC return code 0.
    ///
    /// No effect with `$dllcall()`.
    Halt,
    /// Carry on. `$dll()` evaluates to `$null`. mIRC return code 1.
    Continue,
    /// Run `command` with `parms` available to it as `$1-`. mIRC return code 2.
    ///
    /// With `$dllcall()`, the command runs before the callback alias. An empty `command`
    /// behaves like [`Continue`](Self::Continue).
    Command {
        /// The command to run, such as `echo -a $1-`.
        command: String,
        /// Parameters for the command.
        parms: String,
    },
    /// Make `$dll()` evaluate to this value. mIRC return code 3.
    ///
    /// Lost with `$dllcall()`; use a [`Command`](Self::Command) that stores the value.
    Return(String),
}

impl Response {
    /// Runs `command` with no parameters.
    #[must_use]
    pub fn command(command: impl Into<String>) -> Self {
        Self::Command {
            command: command.into(),
            parms: String::new(),
        }
    }

    /// Runs `command` with `parms` as its `$1-`.
    #[must_use]
    pub fn command_with(command: impl Into<String>, parms: impl Into<String>) -> Self {
        Self::Command {
            command: command.into(),
            parms: parms.into(),
        }
    }

    /// The integer mIRC expects back from the exported function.
    pub(crate) fn code(&self) -> i32 {
        match self {
            Self::Halt => 0,
            Self::Continue => 1,
            Self::Command { .. } => 2,
            Self::Return(_) => 3,
        }
    }
}

/// Types an exported function can return.
///
/// `String` and `&str` become [`Response::Return`], and `()` becomes [`Response::Continue`].
pub trait IntoResponse {
    /// Converts `self` into a [`Response`].
    fn into_response(self) -> Response;
}

impl IntoResponse for Response {
    fn into_response(self) -> Response {
        self
    }
}

impl IntoResponse for String {
    fn into_response(self) -> Response {
        Response::Return(self)
    }
}

impl IntoResponse for &str {
    fn into_response(self) -> Response {
        Response::Return(self.to_owned())
    }
}

impl IntoResponse for () {
    fn into_response(self) -> Response {
        Response::Continue
    }
}
