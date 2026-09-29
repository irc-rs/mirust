//! Background threads that survive mIRC unloading the DLL, and a grace period at exit.
//!
//! # Why not `std::thread::spawn`?
//!
//! mIRC doesn't know about threads a DLL starts itself. When it unloads the DLL (`/dll -u`,
//! the end of a call with [`Config::keep_loaded`](crate::Config::keep_loaded) off, an idle
//! unload, or exit) it calls `UnloadDll` and then `FreeLibrary`, which unmaps the DLL's
//! code. A thread still running that code then crashes mIRC with an access violation the
//! next time it wakes up.
//!
//! [`spawn`] avoids this. Each worker holds its own reference to the DLL, so mIRC's
//! `FreeLibrary` leaves the code mapped until the last worker has finished. The worker then
//! drops that reference and exits in one step (`FreeLibraryAndExitThread`), so it never
//! returns into unmapped code. If it was the last user, the DLL is fully unloaded and its
//! file can be replaced.
//!
//! # Stopping
//!
//! mirust never forcibly kills a thread; that would leave locks (including the process
//! heap lock shared with mIRC) held forever. Workers are asked to stop through their
//! [`StopToken`] and are expected to return soon after.
//!
//! | Event                        | Workers signalled? | mirust waits for them?                       |
//! |------------------------------|--------------------|----------------------------------------------|
//! | `/dll -u`, or end of call with `keep_loaded(false)` ([`Manual`]) | yes | no, `UnloadDll` returns at once |
//! | Idle for 10 minutes ([`Idle`]) | only if unloading is allowed | no                             |
//! | mIRC exiting, or Windows ending the session | yes     | up to [`Config::exit_grace`](crate::Config::exit_grace) (1 s by default) |
//!
//! Not waiting on `/dll -u` and idle unloads is deliberate: `UnloadDll` runs on mIRC's UI
//! thread, so any wait would freeze mIRC, and a worker blocked in a call to mIRC (such as
//! `SendMessage` to its window) would deadlock it. A worker that ignores its token doesn't
//! hurt mIRC; it just keeps the DLL in memory (and its file locked) until it returns.
//!
//! # Exit grace period
//!
//! On exit mIRC is closing anyway, so mirust gives background work a short, bounded chance
//! to finish (for example to flush a file) before Windows ends the process and kills
//! whatever is still running. The grace period covers workers started with [`spawn`] and
//! `$dllcall()` functions still running, which can watch
//! [`Call::stop_token`](crate::Call::stop_token). The wait ends as soon as they have all
//! returned.
//!
//! mIRC doesn't always call `UnloadDll` when it exits: not if the DLL was already unloaded
//! with `/dll -u` (while a worker kept running), not if a `$dllcall()` is still running, and
//! mIRC before 6.3 reports the exit as an ordinary [`Manual`] unload. So
//! mirust also watches mIRC's main window. While the DLL is loaded, a `WH_CALLWNDPROC` hook
//! on mIRC's UI thread looks for the main window's `WM_DESTROY`, or `WM_ENDSESSION` when
//! Windows logs off or shuts down, and grants the grace period there if `UnloadDll` didn't
//! already. The grace period is granted at most once.
//!
//! The hook costs one comparison per message sent to mIRC's UI thread. It is removed when
//! the DLL is unloaded, and never installed with `exit_grace(Duration::ZERO)`.
//!
//! Several mirust DLLs share the wait instead of taking turns. mIRC tells its DLLs about
//! the exit one at a time (each `UnloadDll`, each hook in turn), so the first mirust DLL to
//! notice sends a registered window message that makes every mirust DLL stop its work
//! immediately. Each DLL's grace period then runs from that moment, so the waits overlap:
//! mIRC's exit is delayed by the longest `exit_grace`, not the sum.
//!
//! # Your responsibilities
//!
//! - Check the token regularly, and use [`StopToken::wait_timeout`] instead of
//!   `thread::sleep` so a sleeping worker reacts at once.
//! - Avoid blocking indefinitely (for example network calls without a timeout). A worker
//!   that never returns can't be stopped; it only keeps the DLL in memory.
//! - **Once the token is stopped, don't make blocking calls into mIRC**, such as
//!   `SendMessage` to its window to run a command. During the exit grace period mIRC's UI
//!   thread is inside mirust's wait and can't answer, so the call hangs until the grace
//!   period runs out and the worker never gets to finish. If you must notify mIRC, use
//!   `SendMessageTimeout` with a short timeout, or `PostMessage`.
//! - Make [`Config::on_load`](crate::Config::on_load) safe to run more than once. If a
//!   worker is still finishing when mIRC reloads the DLL, the DLL was never unmapped, so
//!   `LoadDll` runs again with statics intact.
//!
//! Threads you don't create, such as an async runtime's, can't be managed this way; use
//! [`Config::pin_module`](crate::Config::pin_module) for those.
//!
//! # Tested behaviour
//!
//! In mIRC 7.83 (and 6.03 where noted); mIRC exited cleanly in every case:
//!
//! | Scenario | Result |
//! |---|---|
//! | `/dll -u`, worker stops promptly | `/dll -u` returns in under 16 ms; the DLL fully unloads (its file can be deleted) |
//! | `/dll -u`, no workers | the DLL fully unloads |
//! | `/dll -u`, worker ignores its token | `/dll -u` still returns at once; mIRC keeps running; the DLL stays loaded |
//! | Reload while a worker is still finishing | `on_load` runs again; the old worker stays stopped; the new one runs |
//! | `keep_loaded(false)`: several calls, one starting a worker | each call loads and unloads the DLL; the worker finishes; the DLL fully unloads |
//! | Two mirust DLLs, `/dll -u` on one | only that DLL unloads (fully); the other keeps its workers and exit grace |
//! | Exit, worker needs 300 ms to flush | mIRC waits for it, then exits |
//! | Exit, worker ignores its token | mIRC exits about 1.2 s later (one grace period, not two) |
//! | Exit, three mirust DLLs with workers ignoring their tokens | mIRC exits 1.1 – 1.4 s later on every exit path, not ~3 s |
//! | Exit, three mirust DLLs each flushing for 300 ms | all three stop at once and flush in parallel |
//! | Exit after `/dll -u`, worker still draining | mIRC waits for the drain, via the main window |
//! | Exit while a `$dllcall()` is running | its stop token fires; mIRC waits for it and for workers |
//! | Exit on mIRC 6.03 (reported as `Manual`) | mIRC waits for the worker's flush, via the main window |
//!
//! Not tested: `WM_ENDSESSION` (Windows logging off or shutting down while mIRC runs).
//!
//! [`Manual`]: crate::UnloadReason::Manual
//! [`Idle`]: crate::UnloadReason::Idle

use core::ffi::c_void;
use core::ptr;
use std::io;
use std::mem::ManuallyDrop;
use std::panic::{self, AssertUnwindSafe};
use std::sync::atomic::{AtomicPtr, AtomicU32, Ordering};
use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use crate::{WindowHandle, sys};

/// Tells background work when mirust wants it to stop.
///
/// Workers started with [`spawn`] receive one; exported functions can get one from
/// [`Call::stop_token`](crate::Call::stop_token). A token is stopped when mIRC unloads the
/// DLL or exits (see the [module docs](self#stopping)). Once stopped it stays stopped, even
/// if mIRC loads the DLL again while the work is still winding down.
#[derive(Clone)]
pub struct StopToken {
    registry: &'static Registry,
    /// The generation this token belongs to; `None` if it was born stopped.
    generation: Option<u64>,
}

impl core::fmt::Debug for StopToken {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("StopToken")
            .field("stopped", &self.is_stopped())
            .finish()
    }
}

impl StopToken {
    /// Whether the work should stop.
    pub fn is_stopped(&self) -> bool {
        self.stopped_in(&self.registry.lock())
    }

    /// Sleeps for up to `timeout`, waking early if the token is stopped.
    ///
    /// Returns `true` if the token is stopped. Use it in place of `thread::sleep` so the
    /// worker reacts to an unload immediately:
    ///
    /// ```no_run
    /// # use std::time::Duration;
    /// mirust::spawn(|stop| {
    ///     while !stop.wait_timeout(Duration::from_secs(30)) {
    ///         // periodic work
    ///     }
    /// })
    /// .expect("failed to start worker");
    /// ```
    pub fn wait_timeout(&self, timeout: Duration) -> bool {
        let Some(deadline) = Instant::now().checked_add(timeout) else {
            self.wait();
            return true;
        };
        let mut state = self.registry.lock();
        loop {
            if self.stopped_in(&state) {
                return true;
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return false;
            }
            state = self.registry.wait_timeout(state, remaining);
        }
    }

    /// Blocks until the token is stopped.
    pub fn wait(&self) {
        let mut state = self.registry.lock();
        while !self.stopped_in(&state) {
            state = self.registry.wait(state);
        }
    }

    fn stopped_in(&self, state: &State) -> bool {
        self.generation != Some(state.generation)
    }
}

/// Starts a background thread that is safe to leave running when mIRC unloads the DLL.
///
/// The closure receives a [`StopToken`]. Check it regularly (or sleep with
/// [`StopToken::wait_timeout`]) and return once it is stopped. See the
/// [module docs](self) for what happens on each kind of unload and at exit.
///
/// ```no_run
/// use std::time::Duration;
/// use mirust::{Config, Host};
///
/// fn loaded(_: &Host) {
///     mirust::spawn(|stop| {
///         while !stop.wait_timeout(Duration::from_secs(1)) {
///             // poll something
///         }
///         // clean up, flush files, ...
///     })
///     .expect("failed to start worker");
/// }
///
/// mirust::config!(Config::new().on_load(loaded));
/// # fn main() {}
/// ```
///
/// A panic in the closure ends the worker normally; it does not reach mIRC.
///
/// The thread is not joinable. Use a channel or similar if you need a result back.
///
/// # Errors
///
/// Fails if Windows can't take a reference to the DLL or create the thread.
pub fn spawn<F>(f: F) -> io::Result<()>
where
    F: FnOnce(StopToken) + Send + 'static,
{
    let module = ModuleRef::acquire()?;
    let token = REGISTRY.register_worker();
    let job = Box::new(Job {
        module,
        run: Box::new(move || f(token)),
    });
    let param = Box::into_raw(job).cast::<c_void>();

    // SAFETY: `worker_main` takes ownership of `param`, a leaked `Box<Job>`.
    let handle =
        unsafe { sys::CreateThread(ptr::null(), 0, worker_main, param, 0, ptr::null_mut()) };
    if handle.is_null() {
        let error = io::Error::last_os_error();
        // SAFETY: the thread wasn't created, so `param` is still ours. Dropping the job
        // releases the module reference.
        drop(unsafe { Box::from_raw(param.cast::<Job>()) });
        REGISTRY.lock().running -= 1;
        REGISTRY.changed.notify_all();
        return Err(error);
    }
    // SAFETY: a valid handle we own; the thread keeps running without it.
    unsafe { sys::CloseHandle(handle) };
    Ok(())
}

struct Job {
    module: ModuleRef,
    run: Box<dyn FnOnce() + Send>,
}

unsafe extern "system" fn worker_main(param: *mut c_void) -> u32 {
    // SAFETY: `spawn` passes a leaked `Box<Job>` and gives up ownership.
    let job = unsafe { Box::from_raw(param.cast::<Job>()) };
    let Job { module, run } = *job;
    // Everything the worker owns is dropped inside this call, even on panic.
    drop(panic::catch_unwind(AssertUnwindSafe(run)));

    let module = module.into_raw();
    let release = match REGISTRY.finish_worker() {
        Exit::Release => true,
        Exit::Keep => false,
        Exit::UnhookFirst => request_unhook(),
    };
    // Nothing with a destructor is left on this stack. Exit without returning, as
    // returning would run code that `FreeLibraryAndExitThread` may unmap.
    // SAFETY: `module` is a reference we own, released at most once here.
    unsafe {
        if release {
            sys::FreeLibraryAndExitThread(module, 0)
        } else {
            // Keep the reference: the DLL stays mapped until mIRC exits.
            sys::ExitThread(0)
        }
    }
}

/// What a finishing worker must do with its reference to the DLL.
#[derive(Debug, PartialEq, Eq)]
enum Exit {
    /// Release it; someone else keeps the DLL mapped, or nothing still needs it.
    Release,
    /// Keep it, so the DLL stays mapped for the rest of the process.
    Keep,
    /// This is the last reference and the hook is still installed. The hook must be
    /// removed (on mIRC's UI thread) before the DLL can be unmapped.
    UnhookFirst,
}

/// Asks mIRC's UI thread to remove the hook, waiting until it has. Returns whether the
/// hook is gone, so the DLL may be released.
fn request_unhook() -> bool {
    let main = HOOK_WINDOW.load(Ordering::SeqCst);
    let message = DETACH_MESSAGE.load(Ordering::SeqCst);
    let mut result = 0;
    // SAFETY: plain message send. The hook recognises `lparam` as this DLL's registry.
    // `SendMessageTimeoutW` returns only after the UI thread has handled the message and
    // left the hook, so releasing the DLL afterwards can't pull code from under it.
    unsafe {
        sys::SendMessageTimeoutW(
            main,
            message,
            0,
            registry_id(),
            sys::SMTO_ABORTIFHUNG,
            5_000,
            &mut result,
        );
    }
    // If the UI thread didn't get to it (window gone, UI hung), the hook is still
    // installed and the DLL must stay mapped.
    REGISTRY.lock().hook.is_none()
}

/// A counted reference to this DLL, keeping its code mapped until released.
struct ModuleRef(*mut c_void);

// SAFETY: a module handle is an opaque value usable from any thread.
unsafe impl Send for ModuleRef {}

impl ModuleRef {
    fn acquire() -> io::Result<Self> {
        let mut module = ptr::null_mut();
        // SAFETY: any address inside this DLL identifies it; this increments its reference
        // count.
        let ok = unsafe {
            sys::GetModuleHandleExW(
                sys::GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS,
                worker_main as *const u16,
                &mut module,
            )
        };
        if ok == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(Self(module))
        }
    }

    fn into_raw(self) -> *mut c_void {
        ManuallyDrop::new(self).0
    }
}

impl Drop for ModuleRef {
    fn drop(&mut self) {
        // SAFETY: we hold one reference, released exactly once. This only runs when
        // `spawn` fails, on the calling thread, which is itself running inside a mIRC
        // call or a worker, either of which keeps the DLL loaded.
        unsafe { sys::FreeLibrary(self.0) };
    }
}

/// Keeps this DLL mapped until mIRC exits. See [`Config::pin_module`](crate::Config::pin_module).
pub(crate) fn pin_module() -> bool {
    let mut module = ptr::null_mut();
    // SAFETY: as in `ModuleRef::acquire`; pinning can't be undone, which is the point.
    unsafe {
        sys::GetModuleHandleExW(
            sys::GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | sys::GET_MODULE_HANDLE_EX_FLAG_PIN,
            worker_main as *const u16,
            &mut module,
        ) != 0
    }
}

/// Called from `LoadDll`, on mIRC's UI thread. A null `main_window` means none is known.
pub(crate) fn loaded(main_window: WindowHandle, exit_grace: Duration) {
    let main_window = Some(main_window).filter(|w| !w.is_null());
    if let Some(main) = main_window {
        HOOK_WINDOW.store(main.as_raw(), Ordering::SeqCst);
        DETACH_MESSAGE.store(
            register_message("mirust.v1.detach-hook\0"),
            Ordering::SeqCst,
        );
        EXITING_MESSAGE.store(register_message("mirust.v1.exiting\0"), Ordering::SeqCst);
    }
    let mut state = REGISTRY.lock();
    state.unloaded = false;
    state.exit_grace = exit_grace;
    if main_window.is_some() && !exit_grace.is_zero() {
        install_hook(&mut state);
    }
}

/// Called from `UnloadDll`, on mIRC's UI thread, once the unload is going ahead.
pub(crate) fn unloading(exiting: bool) {
    if exiting {
        process_ending();
    } else {
        REGISTRY.stop_all();
    }
    let mut state = REGISTRY.lock();
    // mIRC is about to release its reference. If no worker holds one, the DLL will be
    // unmapped, so the hook must go now; we're on the thread that owns it. Otherwise the
    // last worker to finish removes it (see `request_unhook`).
    if state.running == 0 {
        remove_hook(&mut state);
    }
}

/// Registers an exported function call. The returned guard keeps a `$dllcall()` counted
/// for the exit grace period until it is dropped.
pub(crate) fn begin_call(is_dllcall: bool) -> CallGuard {
    let mut state = REGISTRY.lock();
    if is_dllcall {
        state.calls += 1;
    }
    CallGuard {
        token: REGISTRY.token(&state),
        counted: is_dllcall,
    }
}

pub(crate) struct CallGuard {
    pub(crate) token: StopToken,
    counted: bool,
}

impl Drop for CallGuard {
    fn drop(&mut self) {
        if self.counted {
            REGISTRY.lock().calls -= 1;
            REGISTRY.changed.notify_all();
        }
    }
}

// The hook: watches mIRC's main window for the end of the process.

/// mIRC's main window, as seen by the hook. Read without locking on every message.
static HOOK_WINDOW: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());
/// The registered message a finishing worker sends to have the hook removed.
static DETACH_MESSAGE: AtomicU32 = AtomicU32::new(0);
/// The registered message one mirust DLL sends, when the process is ending, to tell every
/// mirust DLL in mIRC to stop its work before any of them starts waiting.
static EXITING_MESSAGE: AtomicU32 = AtomicU32::new(0);

/// Registers a window message by NUL-terminated name. Returns 0 on failure, which never
/// matches a real registered message.
fn register_message(name: &str) -> u32 {
    let name: Vec<u16> = name.encode_utf16().collect();
    // SAFETY: a NUL-terminated name.
    unsafe { sys::RegisterWindowMessageW(name.as_ptr()) }
}

/// Identifies this DLL's hook in a detach request, as several mirust DLLs may be loaded.
fn registry_id() -> isize {
    ptr::from_ref(&REGISTRY) as isize
}

/// Installs the hook on the calling thread (mIRC's UI thread), unless already installed.
fn install_hook(state: &mut State) {
    if state.hook.is_some() || DETACH_MESSAGE.load(Ordering::SeqCst) == 0 {
        return;
    }
    // SAFETY: a thread hook for the current thread, whose procedure is in this DLL (so the
    // module argument is null). It is removed before the DLL can be unmapped.
    let hook = unsafe {
        sys::SetWindowsHookExW(
            sys::WH_CALLWNDPROC,
            hook_proc,
            ptr::null_mut(),
            sys::GetCurrentThreadId(),
        )
    };
    if !hook.is_null() {
        state.hook = Some(hook as usize);
    }
}

/// Removes the hook. Must run on mIRC's UI thread, which installed it.
fn remove_hook(state: &mut State) {
    if let Some(hook) = state.hook.take() {
        // SAFETY: a hook we installed on this thread and haven't removed yet.
        unsafe { sys::UnhookWindowsHookEx(hook as *mut c_void) };
    }
}

unsafe extern "system" fn hook_proc(code: i32, wparam: usize, lparam: isize) -> isize {
    if code >= 0 {
        // SAFETY: for `WH_CALLWNDPROC`, `lparam` points to a `CWPSTRUCT`.
        let message = unsafe { &*(lparam as *const sys::CwpStruct) };
        if message.hwnd == HOOK_WINDOW.load(Ordering::Relaxed) {
            // Never unwind into mIRC.
            drop(panic::catch_unwind(|| on_main_window_message(message)));
        }
    }
    // SAFETY: passing the message on, as every hook must.
    unsafe { sys::CallNextHookEx(ptr::null_mut(), code, wparam, lparam) }
}

fn on_main_window_message(message: &sys::CwpStruct) {
    match message.message {
        // The main window is going away, or Windows is ending the session (a non-zero
        // `wparam` means it really is). Either way the process is about to end.
        sys::WM_DESTROY => process_ending(),
        sys::WM_ENDSESSION if message.wparam != 0 => process_ending(),
        // Another mirust DLL (or this one) saw the process ending: stop now, wait later.
        m if m != 0 && m == EXITING_MESSAGE.load(Ordering::Relaxed) => REGISTRY.begin_exit(),
        m if m != 0
            && m == DETACH_MESSAGE.load(Ordering::Relaxed)
            && message.lparam == registry_id() =>
        {
            let mut state = REGISTRY.lock();
            // Re-check: mIRC may have loaded the DLL again since the request was sent.
            if state.unloaded && state.running == 0 {
                remove_hook(&mut state);
            }
        }
        _ => {}
    }
}

/// The process is ending. Runs on mIRC's UI thread.
///
/// Every mirust DLL in mIRC is told to stop its work first, and only then does this one
/// wait. Each DLL's grace period runs from the moment it was told to stop, so the waits
/// overlap: several DLLs delay mIRC's exit by the longest grace period, not the sum.
fn process_ending() {
    REGISTRY.begin_exit();
    broadcast_exiting();
    REGISTRY.wait_exit_grace();
}

/// Tells every mirust DLL's hook, including our own, that the process is ending.
fn broadcast_exiting() {
    let main = HOOK_WINDOW.load(Ordering::SeqCst);
    let message = EXITING_MESSAGE.load(Ordering::SeqCst);
    if main.is_null() || message == 0 {
        return;
    }
    // SAFETY: plain message send. We're on the window's own thread, so this runs the hook
    // chain (and mIRC's window procedure, which ignores the message) synchronously.
    unsafe { sys::SendMessageW(main, message, 0, 0) };
}

// Bookkeeping shared by workers, calls, the hook and the entry points.

static REGISTRY: Registry = Registry::new();

struct State {
    /// Bumped whenever work is told to stop. Tokens from an older generation are stopped.
    generation: u64,
    /// True between an unload and the next `LoadDll`. Work started then is born stopped.
    unloaded: bool,
    /// Workers whose closure hasn't returned yet, across all generations.
    running: usize,
    /// `$dllcall()` functions still running.
    calls: usize,
    /// When this DLL was told the process is ending. Its grace period runs from here.
    exit_started: Option<Instant>,
    exit_grace: Duration,
    /// The `WH_CALLWNDPROC` hook on mIRC's UI thread, if installed.
    hook: Option<usize>,
}

struct Registry {
    state: Mutex<State>,
    changed: Condvar,
}

impl Registry {
    const fn new() -> Self {
        Self {
            state: Mutex::new(State {
                generation: 0,
                unloaded: false,
                running: 0,
                calls: 0,
                exit_started: None,
                exit_grace: Duration::ZERO,
                hook: None,
            }),
            changed: Condvar::new(),
        }
    }

    // Nothing panics while holding this lock, but never let poisoning wedge an unload.
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn wait<'a>(&self, guard: MutexGuard<'a, State>) -> MutexGuard<'a, State> {
        self.changed
            .wait(guard)
            .unwrap_or_else(PoisonError::into_inner)
    }

    fn wait_timeout<'a>(
        &self,
        guard: MutexGuard<'a, State>,
        timeout: Duration,
    ) -> MutexGuard<'a, State> {
        self.changed
            .wait_timeout(guard, timeout)
            .unwrap_or_else(PoisonError::into_inner)
            .0
    }

    fn token(&'static self, state: &State) -> StopToken {
        StopToken {
            registry: self,
            generation: (!state.unloaded).then_some(state.generation),
        }
    }

    fn register_worker(&'static self) -> StopToken {
        let mut state = self.lock();
        state.running += 1;
        self.token(&state)
    }

    /// Records that a worker's closure has returned and decides what it does with its
    /// reference to the DLL.
    fn finish_worker(&self) -> Exit {
        let mut state = self.lock();
        state.running -= 1;
        let exit = if state.exit_started.is_some() {
            // The UI thread may be inside the hook, waiting for us. Keep the DLL mapped
            // under it; the process is ending anyway.
            Exit::Keep
        } else if state.unloaded && state.running == 0 && state.hook.is_some() {
            Exit::UnhookFirst
        } else {
            // mIRC still holds the DLL, another worker does, or no hook needs removing.
            Exit::Release
        };
        drop(state);
        self.changed.notify_all();
        exit
    }

    fn stop_all(&self) {
        let mut state = self.lock();
        state.generation += 1;
        state.unloaded = true;
        drop(state);
        self.changed.notify_all();
    }

    /// Records that the process is ending and stops all work. Idempotent; only the first
    /// call starts the grace period.
    fn begin_exit(&self) {
        let mut state = self.lock();
        if state.exit_started.is_some() {
            return;
        }
        state.exit_started = Some(Instant::now());
        state.generation += 1;
        state.unloaded = true;
        drop(state);
        self.changed.notify_all();
    }

    /// Waits for workers and `$dllcall()`s until the grace period that started with
    /// [`begin_exit`](Self::begin_exit) runs out. Returns whether they all finished.
    fn wait_exit_grace(&self) -> bool {
        let mut state = self.lock();
        let Some(started) = state.exit_started else {
            return state.running + state.calls == 0;
        };
        let deadline = started.checked_add(state.exit_grace);
        while state.running + state.calls > 0 {
            let remaining = deadline.map_or(Duration::MAX, |d| {
                d.saturating_duration_since(Instant::now())
            });
            if remaining.is_zero() {
                return false;
            }
            state = self.wait_timeout(state, remaining);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::thread;

    fn registry(grace: Duration) -> &'static Registry {
        let registry = Box::leak(Box::new(Registry::new()));
        registry.lock().exit_grace = grace;
        registry
    }

    #[test]
    fn tokens_stop_on_unload_and_stay_stopped_after_reload() {
        let registry = registry(Duration::ZERO);
        let old = registry.register_worker();
        assert!(!old.is_stopped());

        registry.stop_all();
        assert!(old.is_stopped());

        registry.lock().unloaded = false; // what `loaded` does
        let new = registry.register_worker();
        assert!(old.is_stopped(), "reload must not revive old workers");
        assert!(!new.is_stopped());
    }

    #[test]
    fn work_started_while_unloaded_is_born_stopped() {
        let registry = registry(Duration::ZERO);
        registry.stop_all();
        assert!(registry.register_worker().is_stopped());
    }

    #[test]
    fn wait_timeout_wakes_on_stop() {
        let registry = registry(Duration::ZERO);
        let token = registry.register_worker();
        assert!(!token.wait_timeout(Duration::from_millis(10)));

        let waiter = thread::spawn(move || token.wait_timeout(Duration::from_secs(30)));
        thread::sleep(Duration::from_millis(50));
        let started = Instant::now();
        registry.stop_all();
        assert!(waiter.join().unwrap());
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn exit_grace_is_bounded_and_granted_once() {
        let registry = registry(Duration::from_millis(100));
        let stubborn = registry.register_worker();

        let started = Instant::now();
        registry.begin_exit();
        assert!(stubborn.is_stopped());
        assert!(!registry.wait_exit_grace());
        let first = started.elapsed();
        assert!(first >= Duration::from_millis(90) && first < Duration::from_secs(5));

        // A second exit signal (UnloadDll, then WM_DESTROY) doesn't restart the clock.
        let started = Instant::now();
        registry.begin_exit();
        assert!(!registry.wait_exit_grace());
        assert!(started.elapsed() < Duration::from_millis(50));
    }

    #[test]
    fn grace_runs_from_when_the_dll_was_told_to_stop() {
        // Another DLL's wait kept this one waiting for its turn: the grace is mostly spent.
        let registry = registry(Duration::from_millis(300));
        let _stubborn = registry.register_worker();
        registry.begin_exit();
        thread::sleep(Duration::from_millis(250));

        let started = Instant::now();
        assert!(!registry.wait_exit_grace());
        assert!(
            started.elapsed() < Duration::from_millis(200),
            "waited {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn exit_grace_ends_when_work_finishes() {
        let registry = registry(Duration::from_secs(30));
        registry.register_worker();
        registry.lock().calls += 1;

        let finisher = thread::spawn(move || {
            thread::sleep(Duration::from_millis(50));
            registry.finish_worker();
            registry.lock().calls -= 1;
            registry.changed.notify_all();
        });
        let started = Instant::now();
        registry.begin_exit();
        assert!(registry.wait_exit_grace());
        assert!(started.elapsed() < Duration::from_secs(5));
        finisher.join().unwrap();
    }

    #[test]
    fn finishing_worker_keeps_the_dll_while_mirc_holds_it_or_others_run() {
        let registry = registry(Duration::ZERO);
        registry.lock().hook = Some(1);
        registry.register_worker();
        registry.register_worker();
        // mIRC still holds the DLL.
        assert_eq!(registry.finish_worker(), Exit::Release);

        registry.register_worker();
        registry.stop_all();
        // Unloaded, but another worker still holds the DLL.
        assert_eq!(registry.finish_worker(), Exit::Release);
        // Last one out, hook still installed.
        assert_eq!(registry.finish_worker(), Exit::UnhookFirst);
    }

    #[test]
    fn last_worker_releases_when_no_hook_is_installed() {
        let registry = registry(Duration::ZERO);
        registry.register_worker();
        registry.stop_all();
        assert_eq!(registry.finish_worker(), Exit::Release);
    }

    #[test]
    fn workers_finishing_during_exit_keep_the_dll() {
        let registry = registry(Duration::ZERO);
        registry.register_worker();
        registry.begin_exit();
        assert_eq!(registry.finish_worker(), Exit::Keep);
    }

    // The only test touching the global registry.
    #[test]
    fn spawned_workers_run_stop_and_contain_panics() {
        let (started_tx, started_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        spawn(move |stop| {
            started_tx.send(()).unwrap();
            stop.wait();
            done_tx.send(()).unwrap();
        })
        .unwrap();
        spawn(|_| panic!("worker panic (expected in this test)")).unwrap();

        started_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        REGISTRY.stop_all();
        done_rx.recv_timeout(Duration::from_secs(5)).unwrap();

        let deadline = Instant::now() + Duration::from_secs(5);
        while REGISTRY.lock().running > 0 {
            assert!(Instant::now() < deadline, "workers didn't finish");
            thread::sleep(Duration::from_millis(10));
        }
        REGISTRY.lock().unloaded = false;
    }
}
