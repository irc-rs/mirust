//! Reliable delivery of `$dllcall()` commands.
//!
//! # The problem
//!
//! mIRC keeps each DLL's pending `$dllcall()` result (return code, command and callback
//! alias) in a single slot, read only when its UI thread gets round to processing the
//! completion. Two things overwrite that slot first: another `$dllcall()` into the same DLL
//! finishing, and the script starting another one. Either way the earlier call's command
//! is lost. (Verified in mIRC 7.83; see the README.)
//!
//! # The fix
//!
//! When a `$dllcall()` returns a [`Response::Command`], mirust keeps the command in its
//! own queue, which mIRC can't overwrite, and hands mIRC a *delivery command* instead:
//!
//! ```text
//! dll " $+ $1- $+ " __mirust_deliver        with $1- = this DLL's path
//! ```
//!
//! mIRC runs it on its UI thread when it processes the completion. `__mirust_deliver`
//! answers with one `dll … __mirust_run` per queued command, and each `__mirust_run` pops
//! the oldest command and returns it for mIRC to run. The path travels as a parameter, so
//! mIRC never evaluates a `$` or `%` in it.
//!
//! Every queued call's delivery command is identical, so a clobbered slot still holds a
//! valid one. A delivery dropped because the script started a new call is made up by that
//! new call's completion: a `$dllcall()` also returns the delivery command whenever the
//! queue isn't empty, and a dropped call always queued before the new call started. Each
//! delivery drains the whole queue, so every command runs exactly once, in the order the
//! calls finished.
//!
//! The script's callback alias still runs after each completion, as before, but under
//! clobbering mIRC may run it for the wrong call. mirust can't fix that part.

use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock, PoisonError};

use crate::{Response, sys};

/// Export name of the delivery entry point (defined in `entry`).
pub(crate) const DELIVER_EXPORT: &str = "__mirust_deliver";
/// Export name of the entry point that runs one queued command.
pub(crate) const RUN_EXPORT: &str = "__mirust_run";

static QUEUE: Mutex<VecDeque<(String, String)>> = Mutex::new(VecDeque::new());

fn queue() -> std::sync::MutexGuard<'static, VecDeque<(String, String)>> {
    QUEUE.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A command line calling `export` in this DLL, with the DLL's path passed as `$1-`.
///
/// `$+` joins the quotes to the path, so paths containing spaces work. `$qt()` would be
/// tidier but only exists from mIRC 6.17, and `$dllcall()` from 6.1.
fn call_self(export: &str) -> String {
    format!("dll \" $+ $1- $+ \" {export}")
}

/// Replaces a `$dllcall()`'s response with the delivery command where needed.
pub(crate) fn intercept(response: Response) -> Response {
    let mut queue = queue();
    if let Response::Command { command, parms } = response {
        if command.is_empty() {
            // Behaves like Continue; nothing to deliver.
        } else {
            queue.push_back((command, parms));
        }
    } else if queue.is_empty() {
        return response;
    }
    if queue.is_empty() {
        return Response::Continue;
    }
    drop(queue);
    Response::command_with(call_self(DELIVER_EXPORT), own_path())
}

/// Body of `__mirust_deliver`: one `__mirust_run` per queued command.
///
/// `max_len` is the longest command the host's buffer holds. If the queue is too long to
/// fit, the last segment calls `__mirust_deliver` again to continue.
pub(crate) fn deliver(max_len: usize) -> Response {
    let pending = queue().len();
    if pending == 0 {
        return Response::Continue;
    }
    let run = call_self(RUN_EXPORT);
    let again = call_self(DELIVER_EXPORT);
    const SEPARATOR: &str = " | ";

    // n runs take n * run + (n - 1) * separator.
    let all_fit = pending * (run.len() + SEPARATOR.len()) - SEPARATOR.len() <= max_len;
    let mut segments = if all_fit {
        vec![run.as_str(); pending]
    } else {
        // k runs, then the call to deliver again: k * (run + separator) + again.
        let fit = max_len.saturating_sub(again.len()) / (run.len() + SEPARATOR.len());
        vec![run.as_str(); fit.max(1)]
    };
    if !all_fit {
        segments.push(&again);
    }
    Response::command_with(segments.join(SEPARATOR), own_path())
}

/// Body of `__mirust_run`: the oldest queued command.
pub(crate) fn run_next() -> Response {
    match queue().pop_front() {
        Some((command, parms)) => Response::Command { command, parms },
        None => Response::Continue,
    }
}

/// This DLL's full path, as Windows reports it.
fn own_path() -> String {
    static PATH: OnceLock<String> = OnceLock::new();
    PATH.get_or_init(|| {
        let Some(module) = crate::entry::own_module() else {
            return String::new();
        };
        let mut buf = vec![0u16; 260];
        loop {
            // SAFETY: the buffer length is passed in.
            let len = unsafe { sys::GetModuleFileNameW(module, buf.as_mut_ptr(), buf.len() as u32) }
                as usize;
            if len < buf.len() {
                return String::from_utf16_lossy(&buf[..len]);
            }
            // Truncated: long paths can exceed MAX_PATH.
            buf.resize(buf.len() * 2, 0);
        }
    })
    .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    // One test: the queue is global.
    #[test]
    fn queues_commands_and_delivers_each_once_in_order() {
        let delivery = Response::command_with(call_self(DELIVER_EXPORT), own_path());
        assert!(own_path().ends_with(".exe") || own_path().ends_with(".dll"));

        // Nothing queued: non-commands pass through.
        assert_eq!(intercept(Response::Continue), Response::Continue);
        assert_eq!(
            intercept(Response::Return("x".into())),
            Response::Return("x".into())
        );
        // An empty command behaves like Continue.
        assert_eq!(intercept(Response::command("")), Response::Continue);

        // Commands are queued; mIRC gets the delivery command instead.
        assert_eq!(
            intercept(Response::command_with("first $1-", "a")),
            delivery
        );
        assert_eq!(
            intercept(Response::command_with("second $1-", "b")),
            delivery
        );
        // A non-command while the queue isn't empty also delivers, to make up for a
        // delivery mIRC dropped.
        assert_eq!(intercept(Response::Continue), delivery);

        // One run per queued command, then the commands in order.
        let Response::Command { command, parms } = deliver(10_000) else {
            panic!("expected a command");
        };
        assert_eq!(command, format!("{0} | {0}", call_self(RUN_EXPORT)));
        assert_eq!(parms, own_path());
        assert_eq!(run_next(), Response::command_with("first $1-", "a"));
        assert_eq!(run_next(), Response::command_with("second $1-", "b"));
        assert_eq!(run_next(), Response::Continue);
        assert_eq!(deliver(10_000), Response::Continue);

        // A queue too long for the buffer ends with a call to deliver again.
        for i in 0..50 {
            intercept(Response::command_with("cmd", i.to_string()));
        }
        let Response::Command { command, .. } = deliver(200) else {
            panic!("expected a command");
        };
        assert!(command.len() <= 200, "{} > 200", command.len());
        assert!(command.ends_with(&call_self(DELIVER_EXPORT)));
        let runs = command.matches(RUN_EXPORT).count();
        assert!((1..50).contains(&runs), "{runs} runs");
        while run_next() != Response::Continue {}
    }
}
