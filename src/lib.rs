//! Write mIRC and AdiIRC DLLs in safe, idiomatic Rust.
//!
//! mirust handles the C side of mIRC's DLL interface: the `LoadDll`/`UnloadDll` entry
//! points, the `LOADINFO` handshake, string encoding and buffer sizes across every mIRC
//! release since 5.6. You write ordinary Rust functions that take a [`Call`] and return a
//! [`Response`] (or anything that implements [`IntoResponse`]), and list them in
//! [`export!`]. The entry points are provided for you; use [`config!`] only to change the
//! defaults.
//!
//! ```
//! use mirust::{Call, Response};
//!
//! /// $dll(hello.dll, greet, World) returns "Hello, World!"
//! fn greet(call: Call) -> String {
//!     format!("Hello, {}!", call.data())
//! }
//!
//! /// /dll hello.dll shout hi  ->  echoes "HI" in the active window
//! fn shout(call: Call) -> Response {
//!     Response::command_with("echo -a $1-", call.data().to_uppercase())
//! }
//!
//! mirust::export!(greet, shout);
//! # fn main() {}
//! ```
//!
//! Build with `crate-type = ["cdylib"]`, for the architecture of the client that loads the
//! DLL: mIRC is 32-bit x86 only (`i686-pc-windows-msvc`); AdiIRC is x86, x64 or ARM64
//! (`x86_64-pc-windows-msvc`, `aarch64-pc-windows-msvc`), matching the AdiIRC build
//! installed.
//!
//! # What mirust takes care of
//!
//! - **Buffer sizes.** mIRC's `data`/`parms` buffers have grown from 900 bytes to over
//!   10 KB, and `mBytes` was only added in 7.64 (and misreported until 7.84). mirust picks
//!   the right size for the running version ([`Host::capacity`]) and truncates responses at
//!   a character boundary instead of overflowing.
//! - **Encoding.** Your code always sees `&str`/`String`. mirust requests UTF-16 on mIRC 7+
//!   and converts from UTF-8 or the ANSI code page where it has to ([`Encoding`]).
//! - **Version quirks.** mIRC 5.8 – 6.2 misreported their version; [`Version`] corrects
//!   it. Fields missing from older `LOADINFO` structs are never touched.
//! - **Panics.** A panic in your function halts the calling script instead of unwinding
//!   into mIRC.
//! - **`$dllcall()`.** [`Call::is_dllcall`] tells you whether you are on a worker thread
//!   and free to block.
//! - **Background threads.** mIRC unmaps the DLL when it unloads it, which crashes any
//!   thread still running inside it. Threads started with [`spawn`] keep the DLL mapped
//!   until they finish and are told when to stop; see [`worker`].

#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(missing_docs, unreachable_pub)]
#![deny(unsafe_op_in_unsafe_fn)]

#[cfg(not(windows))]
compile_error!("mirust only supports Windows targets; mIRC and AdiIRC are Windows programs.");

mod call;
mod config;
mod delivery;
mod encoding;
mod entry;
mod host;
mod response;
mod sys;
mod version;
pub mod worker;

#[doc(hidden)]
pub mod runtime;

pub use call::Call;
pub use config::{Config, Unload, UnloadReason};
pub use encoding::Encoding;
pub use host::{Client, Host, WindowHandle, buffer_capacity, host};
pub use response::{IntoResponse, Response};
pub use version::Version;
pub use worker::{StopToken, spawn};

// Compiles the README's Rust examples as doctests, so they can't drift from the API.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct ReadmeDoctests;

/// Configures the DLL: how mIRC loads it, and hooks for loading and unloading.
///
/// Optional: every DLL built with mirust gets the `LoadDll` and `UnloadDll` entry points
/// automatically, using [`Config::new`]'s defaults. Use this macro, at most once per DLL,
/// to change them. The argument must be a `const` expression:
///
/// ```
/// use mirust::{Config, Host};
///
/// fn loaded(_: &Host) {}
///
/// mirust::config!(Config::new().on_load(loaded));
/// # fn main() {}
/// ```
///
/// It generates a hidden export, `__mirust_config`, that mirust's `LoadDll` looks up to
/// find the configuration. The export has the signature mIRC expects of every exported
/// function, and does nothing if a script calls it.
///
/// Every DLL also exports two reserved functions, `__mirust_deliver` and `__mirust_run`,
/// which mIRC calls to run commands returned by `$dllcall()`s (see
/// [`Response`](crate::Response#with-dllcall)). Don't export functions with these three
/// names.
#[macro_export]
macro_rules! config {
    ($config:expr $(,)?) => {
        const _: () = {
            static CONFIG: $crate::Config = $config;

            #[unsafe(export_name = "__mirust_config")]
            unsafe extern "system" fn config(
                main_window: *mut ::core::ffi::c_void,
                _active_window: *mut ::core::ffi::c_void,
                data: *mut ::core::ffi::c_void,
                _parms: *mut ::core::ffi::c_void,
                show: i32,
                _no_pause: i32,
            ) -> i32 {
                // SAFETY: called by mIRC like any exported function, or by mirust's
                // `LoadDll` with its marker values.
                unsafe { $crate::runtime::provide_config(main_window, data, show, &CONFIG) }
            }
        };
    };
}

/// Exports functions so mIRC can call them with `/dll`, `$dll()` and `$dllcall()`.
///
/// Each function must be callable as `fn(Call) -> impl IntoResponse`. The export takes the
/// function's name unless you give another with `as`:
///
/// ```
/// use mirust::Call;
///
/// fn version(_: Call) -> &'static str { env!("CARGO_PKG_VERSION") }
/// fn do_thing(_: Call) {}
///
/// mirust::export!(version, do_thing as "DoThing");
/// # fn main() {}
/// ```
#[macro_export]
macro_rules! export {
    ($($($segment:ident)::+ $(as $name:literal)?),+ $(,)?) => {
        $( $crate::__export_one!([$($segment)::+] [$($segment)+] $($name)?); )+
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __export_one {
    // No `as "name"`: export under the path's last segment.
    ([$($path:tt)+] [$last:ident]) => {
        $crate::__export_one!([$($path)+] [] ::core::stringify!($last));
    };
    ([$($path:tt)+] [$first:ident $($rest:ident)+]) => {
        $crate::__export_one!([$($path)+] [$($rest)+]);
    };
    ([$($path:tt)+] [$($segment:ident)*] $name:expr) => {
        const _: () = {
            #[unsafe(export_name = $name)]
            unsafe extern "system" fn export(
                main_window: *mut ::core::ffi::c_void,
                active_window: *mut ::core::ffi::c_void,
                data: *mut ::core::ffi::c_void,
                parms: *mut ::core::ffi::c_void,
                show: i32,
                no_pause: i32,
            ) -> i32 {
                // SAFETY: mIRC calls exported functions with this signature and buffers
                // sized as described by `Host::capacity`.
                unsafe {
                    $crate::runtime::dispatch(
                        main_window,
                        active_window,
                        data,
                        parms,
                        show,
                        no_pause,
                        $($path)+,
                    )
                }
            }
        };
    };
}
