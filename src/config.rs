use std::time::Duration;

use crate::Host;

/// Settings for the `LoadDll` / `UnloadDll` entry points, given to [`config!`](crate::config).
///
/// Build it in a `const` context:
///
/// ```
/// use mirust::{Config, Host, Unload, UnloadReason};
///
/// fn loaded(host: &Host) { /* start background work */ }
/// fn unloading(reason: UnloadReason) -> Unload { Unload::Allow }
///
/// mirust::config!(Config::new().on_load(loaded).on_unload(unloading));
/// # fn main() {}
/// ```
#[derive(Clone, Copy, Debug)]
pub struct Config {
    pub(crate) keep_loaded: bool,
    pub(crate) unicode: bool,
    pub(crate) on_load: Option<fn(&Host)>,
    pub(crate) on_unload: Option<fn(UnloadReason) -> Unload>,
    pub(crate) pin_module: bool,
    pub(crate) exit_grace: Duration,
}

impl Config {
    /// Defaults: stay loaded between calls, use UTF-16, no hooks, don't pin, and give
    /// [workers](crate::spawn) up to 1 second to finish when mIRC exits.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            keep_loaded: true,
            unicode: true,
            on_load: None,
            on_unload: None,
            pin_module: false,
            exit_grace: Duration::from_secs(1),
        }
    }

    /// Whether mIRC keeps the DLL loaded between calls (`mKeep`). Default `true`.
    ///
    /// With `false`, mIRC unloads the DLL after every call, so statics don't persist.
    #[must_use]
    pub const fn keep_loaded(mut self, keep: bool) -> Self {
        self.keep_loaded = keep;
        self
    }

    /// Whether to ask mIRC 7+ for UTF-16 strings (`mUnicode`). Default `true`.
    ///
    /// Either way your functions see `&str`; this only changes what crosses the boundary.
    /// With `false`, mIRC 7 sends UTF-8. mIRC 6 and earlier always use the ANSI code page.
    #[must_use]
    pub const fn unicode(mut self, unicode: bool) -> Self {
        self.unicode = unicode;
        self
    }

    /// Runs after mirust has read `LOADINFO`, on mIRC's UI thread.
    ///
    /// This is the natural place to start background work with [`spawn`](crate::spawn).
    /// Don't use `std::thread::spawn` for threads that outlive a call: mIRC unmaps the
    /// DLL on unload and such a thread would crash it (see [`crate::worker`]).
    ///
    /// # May run more than once per process
    ///
    /// If the DLL is still in memory when mIRC loads it again (because it is
    /// [pinned](Self::pin_module), or a worker hasn't finished yet), mIRC calls `LoadDll`
    /// again and statics keep their values. Make the hook safe to repeat, for example by
    /// guarding one-time setup with a `static` [`OnceLock`](std::sync::OnceLock).
    #[must_use]
    pub const fn on_load(mut self, hook: fn(&Host)) -> Self {
        self.on_load = Some(hook);
        self
    }

    /// Runs when mIRC is about to unload the DLL.
    ///
    /// The returned [`Unload`] matters only for [`UnloadReason::Idle`]. Without a hook,
    /// idle unloads are refused while [`keep_loaded`](Self::keep_loaded) is set.
    ///
    /// # Not called while a call is running
    ///
    /// mIRC counts the calls in progress for each DLL (including `$dllcall()` workers,
    /// from the moment the worker starts until mIRC handles its completion). While that
    /// count is non-zero, mIRC skips the unload entirely: the DLL stays loaded and this hook
    /// is **not** called. mIRC does not retry later, except as noted:
    ///
    /// - `/dll -u` is dropped. The script has to issue it again.
    /// - [`UnloadReason::Idle`] is retried ten minutes later, as the idle timer restarts.
    /// - [`UnloadReason::Exit`] is skipped, so this hook never runs. [Workers](crate::spawn)
    ///   and the running `$dllcall()` are still told to stop and given the
    ///   [exit grace period](Self::exit_grace), because mirust also watches mIRC's main
    ///   window.
    /// - With [`keep_loaded`](Self::keep_loaded) off, mIRC unloads once the last call ends.
    ///
    /// So don't rely on this hook alone for cleanup that must happen at exit; do it in a
    /// worker or `$dllcall()` that watches its [`StopToken`](crate::StopToken).
    /// (Verified in mIRC 7.85.)
    #[must_use]
    pub const fn on_unload(mut self, hook: fn(UnloadReason) -> Unload) -> Self {
        self.on_unload = Some(hook);
        self
    }

    /// Keeps the DLL in memory until mIRC exits, even after mIRC unloads it. Default `false`.
    ///
    /// Use this when the DLL runs threads that [`spawn`](crate::spawn) doesn't manage and
    /// that you can't stop, such as those of an async runtime or a connection pool. mIRC
    /// would otherwise unmap the DLL's code under them and crash.
    ///
    /// The cost: the DLL file stays locked, so updating it needs an mIRC restart, and
    /// [`on_load`](Self::on_load) runs again each time mIRC reloads the DLL.
    #[must_use]
    pub const fn pin_module(mut self, pin: bool) -> Self {
        self.pin_module = pin;
        self
    }

    /// How long to wait, when mIRC exits, for [workers](crate::spawn) and running
    /// `$dllcall()`s to finish. Default 1 second.
    ///
    /// They are told to stop through their [`StopToken`](crate::StopToken), and the wait
    /// ends as soon as all of them have returned. Afterwards mIRC continues exiting and
    /// Windows ends anything still running. The wait is granted once per process, whether
    /// mIRC reports the exit through `UnloadDll` or not (see
    /// [`worker`](crate::worker#exit-grace-period)); it also applies when Windows ends the
    /// session. Unloads while mIRC keeps running never wait, since that would freeze its UI.
    ///
    /// With several mirust DLLs loaded, their grace periods overlap, so mIRC's exit is
    /// delayed by the longest one, not the sum.
    ///
    /// `Duration::ZERO` disables the wait, and with it the window hook mirust uses to
    /// notice exits that `UnloadDll` misses.
    #[must_use]
    pub const fn exit_grace(mut self, grace: Duration) -> Self {
        self.exit_grace = grace;
        self
    }
}

impl Default for Config {
    fn default() -> Self {
        Self::new()
    }
}

/// Why mIRC is unloading the DLL (`mTimeout`).
///
/// mIRC never unloads, or reports a reason, while one of the DLL's functions is still
/// running; see [`Config::on_unload`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum UnloadReason {
    /// `/dll -u`, or the end of a call with [`Config::keep_loaded`] off. mIRC before 6.3 also
    /// reports exiting this way (tested in 6.03 – 6.21).
    Manual,
    /// The DLL hasn't been used for ten minutes. Return [`Unload::Keep`] to stay loaded.
    Idle,
    /// mIRC is exiting.
    Exit,
    /// A value this version of mirust doesn't know.
    Other(i32),
}

impl UnloadReason {
    pub(crate) fn from_raw(raw: i32) -> Self {
        match raw {
            0 => Self::Manual,
            1 => Self::Idle,
            2 => Self::Exit,
            other => Self::Other(other),
        }
    }
}

/// Answer to an [`UnloadReason::Idle`] unload.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Unload {
    /// Let mIRC unload the DLL.
    Allow,
    /// Stay loaded. Only honoured for [`UnloadReason::Idle`].
    Keep,
}
