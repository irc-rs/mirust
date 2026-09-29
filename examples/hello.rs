//! A small mIRC DLL showing each kind of response.
//!
//! Build:  cargo build --example hello --release --target i686-pc-windows-msvc
//! Output: target/i686-pc-windows-msvc/release/examples/hello.dll
//!
//! That's the target for mIRC (32-bit only). For 64-bit or ARM64 AdiIRC, use
//! x86_64-pc-windows-msvc or aarch64-pc-windows-msvc instead.
//!
//! Then in mIRC:
//!
//!   //echo -a $dll(hello.dll, greet, World)
//!   /dll hello.dll shout hello there
//!   //echo -a $dll(hello.dll, info, $null)
//!   /noop $dllcall(hello.dll, on_slow_done, slow, 3)   (mIRC 6.17+, which has /noop)

use std::thread;
use std::time::Duration;

use mirust::{Call, Response};

/// `$dll(hello.dll, greet, World)` returns `Hello, World!`.
fn greet(call: Call) -> String {
    format!("Hello, {}!", call.data())
}

/// `/dll hello.dll shout <text>` echoes the text in capitals.
fn shout(call: Call) -> Response {
    Response::command_with("echo -a $1-", call.data().to_uppercase())
}

/// `$dll(hello.dll, info, $null)` describes the host as mirust sees it.
fn info(call: Call) -> String {
    let host = call.host();
    format!(
        "client={:?} version={} encoding={:?} capacity={} dllcall={}",
        host.client(),
        host.version(),
        host.encoding(),
        host.capacity(),
        call.is_dllcall(),
    )
}

/// `$dllcall(hello.dll, <alias>, slow, <seconds>)` sleeps on a worker thread, then mIRC
/// calls `<alias>`. Refuses to block when called with `$dll()` instead.
fn slow(call: Call) -> Response {
    if !call.is_dllcall() {
        return Response::command_with("echo -a $1-", "slow: use $dllcall(), not $dll()");
    }
    let seconds = call.data().trim().parse().unwrap_or(1);
    thread::sleep(Duration::from_secs(seconds));
    Response::Continue
}

mirust::export!(greet, shout, info, slow);
