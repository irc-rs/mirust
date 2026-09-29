<p align="center">
  <img src="https://raw.githubusercontent.com/irc-rs/mirust/master/assets/mirust-logo.png" alt="mirust" width="300">
</p>

# mirust

Write [mIRC](https://www.mirc.com) and [AdiIRC](https://adiirc.com) DLLs in safe, idiomatic
Rust.

mIRC scripts can call functions in a DLL with `/dll`, `$dll()` and `$dllcall()`. The
interface behind them is a small, old C ABI with many version-specific quirks. mirust
implements that ABI for you, including:

- the `LoadDll`/`UnloadDll` entry points and the `LOADINFO` handshake;
- string encoding (ANSI, UTF-8 or UTF-16, depending on the mIRC version);
- buffer sizes, which differ between releases and were misreported by some;
- threads that outlive a call, which would otherwise crash mIRC when it unloads the DLL;
- rendering your own graphics into mIRC windows, even the toolbar, at 60 frames per second.

It supports every mIRC release since DLL support arrived in 5.6. You write ordinary Rust
functions that take a `Call` and return a string or a `Response`.

```rust
use mirust::{Call, Response};

/// In mIRC: //echo -a $dll(hello.dll, greet, World)   ->  Hello, World!
fn greet(call: Call) -> String {
    format!("Hello, {}!", call.data())
}

/// In mIRC: /dll hello.dll shout hi there   ->  echoes "HI THERE" in the active window
fn shout(call: Call) -> Response {
    Response::command_with("echo -a $1-", call.data().to_uppercase())
}

// Makes the functions callable from mIRC under their Rust names.
mirust::export!(greet, shout);
```

That is a complete DLL: mirust provides the `LoadDll`/`UnloadDll` entry points itself, and
`mirust::config!` changes their defaults if you need to.

mirust has no dependencies and uses no procedural macros. It requires Rust 1.85 or later
(edition 2024) and only builds for Windows targets.

## Contents

- [Quick start](#quick-start)
- [Calling your DLL from mIRC](#calling-your-dll-from-mirc)
- [Writing exported functions](#writing-exported-functions)
- [Responses: what mIRC does after your function returns](#responses-what-mirc-does-after-your-function-returns)
- [`$dllcall()`: running without freezing mIRC](#dllcall-running-without-freezing-mirc)
- [State, statics and concurrency](#state-statics-and-concurrency)
- [Loading, unloading and `Config`](#loading-unloading-and-config)
- [Background threads](#background-threads)
- [Sending commands to mIRC](#sending-commands-to-mirc)
- [Drawing into mIRC windows](#drawing-into-mirc-windows)
- [The host: version, encoding and buffer sizes](#the-host-version-encoding-and-buffer-sizes)
- [Panics and errors](#panics-and-errors)
- [API at a glance](#api-at-a-glance)
- [Compatibility](#compatibility)
- [Limitations](#limitations)
- [Troubleshooting](#troubleshooting)
- [Upgrading from 0.x](#upgrading-from-0x)
- [Projects using mirust](#projects-using-mirust)

## Quick start

1. Create a library crate, add mirust, and install the Windows targets you need (see
   step 5):

   ```sh
   cargo new --lib my_dll
   cd my_dll
   cargo add mirust
   rustup target add i686-pc-windows-msvc                           # mIRC, 32-bit AdiIRC
   rustup target add x86_64-pc-windows-msvc aarch64-pc-windows-msvc # 64-bit and ARM64 AdiIRC
   ```

2. Make it build a DLL. In `Cargo.toml`:

   ```toml
   [lib]
   crate-type = ["cdylib"]
   ```

3. Optionally, link the C runtime statically so users don't need the Visual C++
   Redistributable installed. Create `.cargo/config.toml`:

   ```toml
   [target.'cfg(all(windows, target_env = "msvc"))']
   rustflags = ["-C", "target-feature=+crt-static"]
   ```

4. Write `src/lib.rs` (for example, the code at the top of this page).

5. Build for the architecture of the client that will load the DLL. Windows can only load
   a DLL built for the same architecture as the process loading it:

   | Client | Architecture | Target |
   |--------|--------------|--------|
   | mIRC (every version) | 32-bit x86 only | `i686-pc-windows-msvc` |
   | AdiIRC, 32-bit build | x86 | `i686-pc-windows-msvc` |
   | AdiIRC, 64-bit build | x64 | `x86_64-pc-windows-msvc` |
   | AdiIRC, ARM64 build | ARM64 | `aarch64-pc-windows-msvc` |

   AdiIRC's architecture is that of the AdiIRC build installed, not of Windows: 32-bit
   AdiIRC on 64-bit Windows still needs the 32-bit DLL.

   ```sh
   cargo build --release --target i686-pc-windows-msvc     # mIRC and 32-bit AdiIRC
   cargo build --release --target x86_64-pc-windows-msvc   # 64-bit AdiIRC
   cargo build --release --target aarch64-pc-windows-msvc  # ARM64 AdiIRC
   ```

   Each DLL is `target/<target>/release/my_dll.dll`. For mIRC alone, the `i686` build is
   all you need. To support every AdiIRC build as well, ship all three, with names or
   folders that tell them apart.

6. Try it in mIRC (the `//` makes mIRC evaluate identifiers in a command typed into the
   editbox):

   ```text
   //echo -a $dll(C:\path\to\my_dll.dll, greet, World)
   ```

The repository's [`examples/hello.rs`](examples/hello.rs) is a complete DLL showing each
kind of response. Build it with
`cargo build --example hello --release --target i686-pc-windows-msvc`.

## Calling your DLL from mIRC

mIRC offers three ways to call an exported function:

- `<file>` is the DLL's path. A full path is the most reliable; if it has no extension,
  mIRC appends `.dll`.
- `<proc>` is the exported function's name. It is case-sensitive.
- `[data]` is the text your function receives as `call.data()`.

| mIRC syntax                               | Runs on          | Waits for the result? | Your function's return value              |
|-------------------------------------------|------------------|-----------------------|-------------------------------------------|
| `/dll <file> <proc> [data]`               | mIRC's UI thread | yes                   | can halt the script or run a command      |
| `$dll(<file>, <proc>, [data])`            | mIRC's UI thread | yes                   | can also become the identifier's value    |
| `$dllcall(<file>, <alias>, <proc>, [data])` | a new worker thread | no; `$dllcall()` is `$null` at once | only a command has an effect; then `<alias>` is called |

Other script commands you'll use:

| mIRC syntax     | Effect                                                                  |
|-----------------|-------------------------------------------------------------------------|
| `/dll -u <file>` | unloads the DLL (mIRC then calls `UnloadDll`)                          |
| `$dll(0)`       | number of loaded DLLs                                                   |
| `$dll(N)`       | path of the Nth loaded DLL                                              |
| `/.dll ...`     | a quiet call: `call.is_quiet()` is `true`                               |

mIRC evaluates identifiers and variables in `[data]` before calling you, so you receive
the final text. mIRC loads the DLL on the first call. With the default configuration it
stays loaded until `/dll -u`, ten minutes of disuse (which mirust refuses by default), or
mIRC exiting.

## Writing exported functions

An exported function is any function that can be called as `fn(Call) -> R`, where `R`
implements `IntoResponse`. List it in `mirust::export!` to make it callable from mIRC:

```rust
use mirust::Call;

mod utils {
    pub fn helper(_: mirust::Call) {}
}

fn version(_: Call) -> &'static str {
    env!("CARGO_PKG_VERSION")
}

fn do_thing(_: Call) {}

// Exported as "version", "DoThing" and "helper".
mirust::export!(version, do_thing as "DoThing", utils::helper);
```

- The export name is the function's name (the last segment of its path), or the string
  after `as`. mIRC matches it case-sensitively.
- `export!` can be invoked more than once, but each export name must be unique across the
  DLL.
- Don't export functions named `__mirust_config`, `__mirust_deliver` or `__mirust_run`;
  mirust reserves them.

### What a `Call` gives you

`Call` describes one invocation. It is passed by value, so you can take ownership of the
input with `into_data()`.

| Method                     | Returns                  | Meaning                                                                   |
|----------------------------|--------------------------|---------------------------------------------------------------------------|
| `data()`                   | `&str`                   | the `[data]` text from the script, already decoded                        |
| `into_data()`              | `String`                 | consumes the call, returning the text                                     |
| `main_window()`            | `WindowHandle`           | mIRC's main window                                                        |
| `active_window()`          | `WindowHandle`           | the window the command was issued from (not always the active one for remote scripts) |
| `is_quiet()`               | `bool`                   | `true` for `/.dll` (the script asked for no output)                       |
| `no_pause()`               | `bool`                   | `true` if mIRC is in a critical routine: don't do anything that pauses it, such as opening a dialog |
| `is_dllcall()`             | `bool`                   | `true` on a `$dllcall()` worker thread, `false` on mIRC's UI thread        |
| `stop_token()`             | `&StopToken`             | stopped if mIRC exits while this call runs (useful in long `$dllcall()`s)  |
| `host()`                   | `&'static Host`          | the client, version, encoding and buffer size                             |
| `max_response_len()`       | `usize`                  | the longest response, in code units, that fits without being truncated    |

## Responses: what mIRC does after your function returns

Return any type that implements `IntoResponse`:

| Return type / value                          | mIRC return code | With `/dll` and `$dll()`                             |
|----------------------------------------------|------------------|------------------------------------------------------|
| `String`, `&str`, or `Response::Return(s)`   | 3                | `$dll()` evaluates to the string                     |
| `()` or `Response::Continue`                 | 1                | the script carries on; `$dll()` evaluates to `$null` |
| `Response::Halt`                             | 0                | halts the calling script, as `/halt` does            |
| `Response::command(cmd)`                     | 2                | mIRC runs `cmd`                                      |
| `Response::command_with(cmd, parms)`         | 2                | mIRC runs `cmd` with `parms` available as `$1-`      |

```rust
use mirust::{Call, Response};

/// $dll(my.dll, double, 21) evaluates to 42; invalid input echoes an error instead.
fn double(call: Call) -> Response {
    match call.data().trim().parse::<i64>() {
        Ok(n) => Response::Return((n * 2).to_string()),
        Err(_) => Response::command_with("echo -a double: not a number: $1-", call.into_data()),
    }
}

mirust::export!(double);
```

Things to know about responses:

- `Response::Command { command, parms }` can also be built directly. An empty `command`
  behaves like `Continue`.
- Text longer than the host's buffers is truncated at a character boundary, never
  mid-character. Check `call.max_response_len()` if that matters.
- You can implement `IntoResponse` for your own types.
- A panic is turned into `Response::Halt` (see [Panics and errors](#panics-and-errors)).

## `$dllcall()`: running without freezing mIRC

`/dll` and `$dll()` run your function on mIRC's UI thread: mIRC is frozen until you return,
so keep them quick. For slow work (network requests, file I/O, sleeping), have scripts use
`$dllcall()`. mIRC then runs your function on a new worker thread, `$dllcall()` evaluates
to `$null` at once, and the script continues. When your function returns, mIRC:

1. runs your `Response::Command`, if you returned one, and then
2. runs the callback named in `$dllcall()`, with the DLL's full path as `$1-`.

`Halt`, `Continue` and `Return` have no effect with `$dllcall()`: there is no script left to
halt and no identifier to return a value to. **A returned value is lost.**

**Use the returned command as the per-call callback**, with the result as its parameters,
and pass `noop` (mIRC's do-nothing command, from 6.17) as the `$dllcall()` callback:

```rust
use mirust::{Call, Response};

fn fetch(call: Call) -> Response {
    if !call.is_dllcall() {
        return Response::command_with("echo -a $1-", "fetch: call me with $dllcall()");
    }
    let result = format!("fetched {}", call.data()); // blocking work is fine here
    // Runs `fetch_done <result>` in mIRC once this call has finished.
    Response::command_with("fetch_done $1-", result)
}

mirust::export!(fetch);
```

```text
; mIRC script
alias start_fetch { noop $dllcall(my.dll, noop, fetch, example.com) }
alias fetch_done { echo -a Result: $1- }
```

Returned commands are delivered reliably, even when calls overlap; the `$dllcall()`
callback isn't (see below). A `$dllcall()` still running when mIRC exits can notice through
`call.stop_token()`, and gets the same exit grace period as background threads (see
[Background threads](#background-threads)).

### Overlapping `$dllcall()`s into the same DLL

Several `$dllcall()`s into one DLL run in parallel, each with its own input. mIRC itself,
though, keeps each DLL's pending `$dllcall()` result (the command and the callback) in a
single slot, read only when its UI thread gets round to processing the result. Two things
overwrite that slot first:

- **another `$dllcall()` into the same DLL finishing** before mIRC has processed the
  previous result;
- **the script starting another `$dllcall()` into the same DLL** while a finished call's
  result is still waiting, for example because the UI thread was busy running a script.

In mIRC 7.83 both happened on every run: the earlier call's command and callback were
lost, and the later call's callback ran twice.

**mirust protects returned commands from this.** When a `$dllcall()` returns a
`Response::Command`, mirust keeps the command in its own queue and hands mIRC a small
delivery command instead, which calls back into the DLL (through two reserved exports,
`__mirust_deliver` and `__mirust_run`) to run every queued command. Each command runs
**exactly once, in the order the calls finished**, whatever mIRC does to its slot. Tested in
mIRC 7.83:

| Scenario | Without mirust's queue | With it |
|---|---|---|
| Three calls finishing together | one command lost, another run twice | each command runs once |
| A call finishing while the UI is busy, then another starting | the first command lost | both run once |
| 200 calls at once | — | 200 commands, each run once |
| 450 calls finishing before mIRC can process any | — | 450 commands, each run once |

**The `$dllcall()` callback itself is still mIRC's**, and can still run for the wrong call,
or twice, when calls overlap. That's why the callback should be `noop`, with the real work
in the returned command.

Details:

- The script needs nothing special: no alias to define, and no path to pass. mirust finds
  its own path, and passes it as a parameter so mIRC never evaluates a `$` or `%` in it.
- `/dll` and `$dll()` calls are never queued; they don't use the slot.
- With `Config::keep_loaded(false)` commands aren't queued, because mIRC unloads the DLL
  (and the queue with it) after every call. They are handed to mIRC directly, as mIRC
  intends, and overlapping calls can lose them.
- Don't load the DLL by its 8.3 short name (such as `MYDLL~1.DLL`). Delivery still works,
  but mIRC then registers the DLL a second time under its long name and calls `LoadDll`
  (and `on_load`) again.
- Don't export functions named `__mirust_deliver` or `__mirust_run`; the build fails with a
  duplicate-symbol error if you do.

## State, statics and concurrency

Each call gets a fresh `Call`. State that must survive between calls goes in `static`s,
which persist for as long as the DLL stays loaded (the default):

```rust
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use mirust::Call;

static COUNTER: AtomicU64 = AtomicU64::new(0);
static HISTORY: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// $dll(my.dll, remember, text) stores the text and returns how many calls so far.
fn remember(call: Call) -> String {
    HISTORY.lock().unwrap().push(call.into_data());
    (COUNTER.fetch_add(1, Ordering::SeqCst) + 1).to_string()
}

mirust::export!(remember);
```

Keep in mind:

- **Functions can run concurrently.** A `$dll()` call on the UI thread can overlap with any
  number of `$dllcall()` worker threads, and with your own background threads. Shared state
  must be thread-safe (`Mutex`, atomics, `OnceLock`, ...).
- **With `Config::keep_loaded(false)`, statics don't persist.** mIRC loads and unloads the
  DLL around every call, so each call starts from fresh statics.
- **Statics can outlive an unload.** If a background thread is still finishing, or the
  module is pinned, mIRC can load the DLL again without it ever leaving memory. `on_load`
  then runs again, with statics still holding their old values.
- `mirust::host()` returns the `Host` from anywhere, including background threads.

## Loading, unloading and `Config`

Every DLL built with mirust exports the `LoadDll` and `UnloadDll` functions mIRC looks for,
using `Config::new()`'s defaults. To change them, invoke `mirust::config!` once, anywhere in
the crate, with a `Config`. It must be a `const` expression:

```rust
use std::time::Duration;
use mirust::{Config, Host, Unload, UnloadReason};

fn loaded(host: &Host) {
    // Runs on mIRC's UI thread each time mIRC loads the DLL.
    let _ = host.version();
}

fn unloading(reason: UnloadReason) -> Unload {
    match reason {
        // Ten minutes unused: stay loaded.
        UnloadReason::Idle => Unload::Keep,
        // `/dll -u`, mIRC exiting, ...: the answer is ignored; mIRC unloads anyway.
        _ => Unload::Allow,
    }
}

mirust::config!(
    Config::new()
        .keep_loaded(true)
        .unicode(true)
        .on_load(loaded)
        .on_unload(unloading)
        .pin_module(false)
        .exit_grace(Duration::from_secs(1))
);
```

| `Config` method        | Default | Meaning                                                                          |
|------------------------|---------|----------------------------------------------------------------------------------|
| `keep_loaded(bool)`    | `true`  | stay loaded between calls (`mKeep`). With `false`, mIRC unloads after every call |
| `unicode(bool)`        | `true`  | ask mIRC 7+ for UTF-16 strings (`mUnicode`). With `false`, mIRC 7+ sends UTF-8. Your code sees `&str` either way |
| `on_load(fn(&Host))`   | none    | runs after mirust has read `LOADINFO`, on mIRC's UI thread; can run more than once (see above) |
| `on_unload(fn(UnloadReason) -> Unload)` | none | runs when mIRC unloads the DLL; its answer only matters for `Idle` |
| `pin_module(bool)`     | `false` | keep the DLL in memory until mIRC exits (see [Background threads](#background-threads)) |
| `exit_grace(Duration)` | 1 s     | how long mIRC's exit waits for background work to finish. `Duration::ZERO` disables it |

Why mIRC unloads a DLL (`UnloadReason`):

| Reason      | When                                                                                     |
|-------------|------------------------------------------------------------------------------------------|
| `Manual`    | `/dll -u`, or the end of a call with `keep_loaded(false)`. mIRC before 6.3 also reports exit this way |
| `Idle`      | unused for ten minutes. Return `Unload::Keep` to stay loaded. Without an `on_unload` hook, mirust refuses idle unloads while `keep_loaded` is set |
| `Exit`      | mIRC is exiting                                                                          |
| `Other(n)`  | a value this version of mirust doesn't know                                              |

**mIRC never unloads a DLL while one of its functions is running**, including a
`$dllcall()`. In that case it skips the unload entirely and doesn't call `on_unload`:
`/dll -u` is silently dropped, idle unloads are retried ten minutes later, and exit goes
ahead without `UnloadDll`. So don't rely on `on_unload` alone for cleanup that must happen.
mirust still stops background work and gives it the exit grace period in that case.

**A DLL can unload itself.** Send `/dll -u` for its own path, `mirust::dll_path()`, from a
worker thread. mIRC calls `UnloadDll` and frees the DLL, and because the worker holds it
(see [Background threads](#background-threads)), it stays mapped until the worker returns:

```rust,no_run
use mirust::{Call, mirc};

fn goodbye(_: Call) {
    mirust::spawn(|_stop| {
        // ... clean up, then:
        let _ = mirc::command(&format!("dll -u \"{}\"", mirust::dll_path()));
    })
    .expect("failed to start worker");
}

mirust::export!(goodbye);
# fn main() {}
```

Send it from the worker, not from the exported function: mIRC never unloads a DLL while one
of its functions is running. The `m_nyancat` project unloads itself this way when its `stop`
function is called (checked in mIRC 7.85: gone from the process within 0.2 s).

`config!` works by exporting a hidden function, `__mirust_config`, which mirust's
`LoadDll` looks up. It has the signature mIRC expects of every exported function, so a
script that calls it by name (`/dll my.dll __mirust_config`) does nothing harmful. Invoking
`config!` twice fails to link with a duplicate-symbol error.

## Background threads

**Don't start long-lived threads with `std::thread::spawn`.** When mIRC unloads a DLL, it
unmaps the DLL's code from memory. Any thread still running that code then crashes mIRC.
Unloads happen on `/dll -u`, after every call with `keep_loaded(false)`, on an idle unload
you allow, and at exit.

Use `mirust::spawn` instead. Each worker keeps the DLL in memory until it returns, and
receives a `StopToken` that is stopped when mIRC unloads the DLL or exits:

```rust
use std::time::Duration;
use mirust::{Config, Host};

fn loaded(_: &Host) {
    mirust::spawn(|stop| {
        // Wakes every 30 s, or at once when mIRC unloads the DLL or exits.
        while !stop.wait_timeout(Duration::from_secs(30)) {
            // periodic work
        }
        // clean up: flush files, close connections, ...
    })
    .expect("failed to start worker");
}

mirust::config!(Config::new().on_load(loaded));
```

`StopToken` has three methods: `is_stopped()` checks it, `wait()` blocks until it is
stopped, and `wait_timeout(duration)` sleeps until it is stopped or the time runs out,
returning `true` if it is stopped. Once stopped, a token stays stopped, even if mIRC loads
the DLL again.

How unloads and exits treat workers:

| Event                                   | Workers told to stop? | mirust waits for them?                       |
|-----------------------------------------|-----------------------|----------------------------------------------|
| `/dll -u`, or end of call with `keep_loaded(false)` | yes       | no; mIRC carries on at once                  |
| Idle for ten minutes                    | only if you allow the unload | no                                    |
| mIRC exits, or Windows ends the session | yes                   | up to `exit_grace` (1 s by default), ending as soon as all have returned |

- **mirust never waits while mIRC keeps running.** Unloading runs on mIRC's UI thread, so
  waiting there would freeze mIRC. A worker that ignores its token doesn't freeze or crash
  mIRC; it keeps the DLL in memory, and its file locked, until it returns. When the last
  worker returns, the DLL fully unloads.
- **mirust catches every exit.** mIRC doesn't always call `UnloadDll` when it exits (not
  after `/dll -u`, not while a `$dllcall()` runs, and old versions report exit as a normal
  unload). mirust therefore also watches mIRC's main window, and grants the grace period
  once per exit.
- **Several mirust DLLs share the wait.** The first to notice the exit tells the others to
  stop too, so mIRC's exit is delayed by the longest `exit_grace`, not the sum.
- **Threads are never killed.** Killing a thread can leave locks held, including the heap
  lock mIRC shares, and freeze mIRC. Workers must return on their own.

Your responsibilities as a worker author:

- Check the token regularly, and sleep with `wait_timeout` rather than `thread::sleep` so
  the worker reacts at once.
- Don't block indefinitely, for example on network calls without a timeout.
- **Once the token is stopped, don't make blocking calls into mIRC** yourself, such as
  `SendMessage` to its window. During the exit grace period mIRC's UI thread is waiting for
  your workers and can't answer, so the call hangs until the grace period runs out.
  [`mirust::mirc`](#sending-commands-to-mirc) handles this for you: it refuses requests
  during exit with `SendError::Exiting` instead of waiting.
- Make `on_load` safe to run more than once (for example, guard one-time setup with a
  `static OnceLock`).
- `spawn` returns `std::io::Result<()>` and the thread isn't joinable. Use a channel or a
  `static` to get results out of it.

For threads you don't create yourself, such as an async runtime's or a connection pool's,
use `Config::new().pin_module(true)`. The DLL then stays in memory until mIRC exits, so
nothing can be unmapped under those threads. The cost is that its file stays locked until
mIRC restarts, and `on_load` runs again whenever mIRC reloads it.

Background threads can hand results to mIRC directly with
[`mirust::mirc`](#sending-commands-to-mirc), for example
`mirc::command("set %result 42")`.

The [`worker` module docs](https://docs.rs/mirust/latest/mirust/worker/index.html) have the
full details and the behaviour measured in real mIRC.

## Sending commands to mIRC

`mirust::mirc` runs commands and evaluates identifiers in mIRC from anywhere in your DLL:
exported functions, `$dllcall()`s and background threads. It wraps mIRC's documented
SendMessage interface (`WM_MCOMMAND` and `WM_MEVALUATE`).

```rust
use std::time::Duration;
use mirust::{Call, Config, Host, mirc};

fn loaded(_: &Host) {
    mirust::spawn(|stop| {
        while !stop.wait_timeout(Duration::from_secs(60)) {
            // Background threads can talk to mIRC directly.
            if let Err(e) = mirc::command("echo -s another minute passed") {
                // Every failure is a value: mIRC busy, exiting, SendMessage disabled, ...
                let _ = e;
            }
        }
    })
    .expect("failed to start worker");
}

mirust::config!(Config::new().on_load(loaded));

/// $dll(my.dll, whoami, $null) returns your nickname, asked of mIRC.
fn whoami(_: Call) -> String {
    mirc::evaluate("$me").unwrap_or_default()
}

mirust::export!(whoami);
```

| Function / builder | Does |
|--------------------|------|
| `mirc::command(text)` | runs `text` as if typed in the main window's editbox |
| `mirc::evaluate(text)` | evaluates `text` (such as `"$version"`) and returns the result |
| `mirc::Command::new(text)` | a command with options: `.window(w)`, `.plain_text()`, `.flood_protection()`, `.event_id(id)`, `.timeout(d)`, then `.send()` |
| `mirc::Evaluate::new(text)` | an evaluation with options: `.window(w)`, `.event_id(id)`, `.timeout(d)`, then `.send()` |

**Commands run as if typed.** mirust adds a leading `/` if it is missing, so `"echo -a hi"`
runs `/echo -a hi`. As when typing, a single `/` leaves identifiers and variables
unevaluated: `/echo $me` prints `$me`. That keeps text built from user input from being
evaluated by accident. Start the text with `//` to have mIRC evaluate it first (`//echo $me`
prints your nickname). With `.plain_text()`, the text is sent as a message to the window's
channel or query instead, and no `/` is added.

**Calls never affect each other.** From mIRC 6.2, each call uses its own uniquely named
shared-memory block, created exclusively and never reused, so calls in a row, or at the
same time from different threads, can't see each other's data. mIRC 5.9 – 6.17 only read a
single block named `mIRC`; there, calls from your DLL take turns. Text too long for the block
is rejected rather than truncated.

**Threads.** On mIRC's UI thread (in `/dll`, `$dll()`, `on_load`), mIRC runs the request
before the call returns. From other threads, the request waits for mIRC's UI thread, for up
to the timeout (5 seconds by default). While mIRC is exiting, requests from other threads
fail at once with `SendError::Exiting`, because mIRC's UI thread is then waiting for your
workers and can't answer.

**Errors** are `mirc::SendError` values; nothing panics:

| Error | Meaning |
|-------|---------|
| `Unsupported` | mIRC before 5.9, or `.event_id` before 7.33 |
| `NoWindow` | no mIRC window, or it was destroyed while the request waited (for example as mIRC closed) |
| `InvalidText`, `TooLong` | the text contains NUL, or doesn't fit (32,767 UTF-16 units; 65,535 bytes before 7.0) |
| `Busy` | before 6.2 only: another program was using the `mIRC` block |
| `Exiting` | mIRC is exiting; the request wasn't sent |
| `Timeout` | mIRC didn't answer in time, or isn't responding |
| `Disabled` | SendMessage is turned off in mIRC: Options → Other, or the Lock dialog (reported from 7.34) |
| `Failed { code }` | mIRC reported a failure, such as an error in the command. From 7.33, `code` says why |
| `System` | a Windows call failed |

**Event context.** Inside a remote event, pass its `$eventid` to your DLL and give it to
`.event_id(id)`: identifiers describing the event (`$nick`, `$chan`, `$signal`, ...) then
refer to that event, as if the request ran inside it. The handler's own parameters (`$1-`)
aren't included, so pass those in the text, and the id stops working once the event
finishes (`Failed { code: Some(9) }`). Needs mIRC 7.33.

Before mIRC 7.33 every failure looks the same (`Failed { code: None }`), and some failed
commands are reported as successful. mIRC passes unknown commands to the IRC server, so an
unknown command fails with a server error (for example "not connected to server") rather
than "unknown command". Text is sent as UTF-16 from mIRC 7.0, and in the ANSI code page
before.

## Drawing into mIRC windows

`mirust::canvas` lets Rust render frames straight into an mIRC **picture window**, using
mIRC's `/drawdll` command (mIRC 7.83 and later). `/drawdll` calls a function in your DLL and
hands it the window's bitmap, so there are no per-shape script commands and no image files:
Rust draws a whole frame and copies it in.

```rust
use mirust::canvas::{Draw, Image, Update, rgb};

/// /drawdll -n @scene "C:\path\shade.dll" gradient
fn gradient(draw: Draw) -> Update {
    let canvas = draw.canvas();
    let mut image = Image::new(canvas.width(), canvas.height());
    for (i, pixel) in image.pixels_mut().iter_mut().enumerate() {
        *pixel = rgb((i % 256) as u8, 64, 160);
    }
    canvas.draw(&image);
    Update::Redraw
}

// Like `export!`, but for functions that draw. It takes several functions, `as "Name"`
// and paths, just as `export!` does.
mirust::export_draw!(gradient);
```

A script creates the window and calls the function:

```text
/window -pk0 @scene 20 20 420 320
/drawdll -n @scene "C:\path\shade.dll" gradient any text, available as draw.data()
```

**Animation.** Have a worker thread (see [Background threads](#background-threads)) send
the command once per frame. `canvas::draw_command` builds it, quoting the DLL's own path for
you. Render on the worker into an `Image` and copy it in from the export, because the export
runs on mIRC's UI thread and must be quick:

```rust,no_run
use std::time::{Duration, Instant};
use mirust::{Call, canvas, mirc};

fn animate(_: Call) {
    mirust::spawn(|stop| {
        let interval = Duration::from_micros(1_000_000 / 60);
        let begin = Instant::now();
        for frame in 0u32.. {
            let command = canvas::draw_command("@scene", "render", &frame.to_string());
            if mirc::command(&command).is_err() {
                break; // mIRC is exiting, or the window is gone
            }
            let due = begin + interval * (frame + 1);
            if stop.wait_timeout(due.saturating_duration_since(Instant::now())) {
                break;
            }
        }
    })
    .expect("failed to start worker");
}

mirust::export!(animate);
# fn main() {}
```

| Item | Purpose |
|------|---------|
| `export_draw!(f, g as "Name")` | exports functions callable as `fn(Draw) -> impl IntoUpdate` |
| `Draw` | one call: `canvas()`, `data()` (the text after the export's name), `main_window()`, `active_window()`, `host()` |
| `Canvas` | the window's bitmap: `width()`, `height()`, `draw(&image)`, `draw_at(&image, x, y)`, and raw `hdc()` / `bitmap()` for GDI |
| `Image` | pixels to copy in: `new`, `width`, `height`, `pixels`, `pixels_mut`, `fill`, `scaled(factor)` |
| `rgb(r, g, b)` | packs a pixel; pixels are `0x00RRGGBB` |
| `Update` | what mIRC redraws afterwards: `Redraw`, `Area { left, top, right, bottom }`, `Skip`, `Default` |
| `draw_command(window, export, data)` | the `/drawdll` command for one of your exports |

`Update` values, checked in mIRC 7.85. The command must use `-n` (as `draw_command` does):

| Value | Effect |
|-------|--------|
| `Redraw` (also what `()` means) | the whole window is redrawn |
| `Area { .. }` | only that rectangle is redrawn |
| `Skip` | nothing is redrawn |
| `Default` | mIRC decides: with `-n`, nothing is redrawn |

`Skip` and `Area` change the bitmap all the same: the part that wasn't redrawn shows the new
picture as soon as something redraws it.

**The picture window.**

- Its bitmap is the window's *client area*: the window's width minus 22 and height minus 56
  pixels. mIRC won't make a window narrower than about 198 pixels, so the smallest bitmap is
  about 176 pixels wide. `Canvas::width` and `height` give the real size.
- Add `-h` to `/window` to keep the window hidden. Drawing works the same.
- `Draw` and `Canvas` are not `Send`: the handles are only valid until the function returns.
- If a script calls an exported drawing function any other way, such as `/dll`, there is no
  bitmap and nothing happens. A panic halts the script, as with `export!`.

**Speed** (mIRC 7.85, 32-bit). These were measured on a 60 Hz monitor, so at most 60 frames
per second could be *seen*; that is a limit of the test display, not of mIRC or `/drawdll`:

| Frame | Copying it into the window | Result |
|-------|----------------------------|--------|
| 398 × 264 | about 0.25 ms | 60 frames per second, none dropped |
| 1920 × 1080 | about 2 ms on mIRC's UI thread | 58 of 60 frames per second shown, with about 6 ms of Rust rendering on 28 cores |

For comparison, saving each frame as a BMP and drawing it with `/drawpic` costs about 6 ms at
398 × 264 and 11 ms at 878 × 644. Rendering is on you: a full-window Mandelbrot zoom took
about 55 ms a frame at 1280 × 720, so heavy scenes belong on worker threads, at a lower
resolution, or both.

### In the toolbar

A picture window can be the picture of a toolbar button: `/toolbar -a name tip @scene`
(16 × 16 to 256 × 256 pixels; add `x y w h` to use part of the bitmap). The button doesn't
follow the window, so refresh it after each frame with `/toolbar -p name @scene`. (`-pu`
also forces an immediate update, which mIRC 7.85 doesn't need and which costs about 0.2 ms
a frame more on its UI thread.) Both commands fit in one string, which is one round trip
to mIRC:

```rust,no_run
use mirust::{canvas, mirc};

let frame = canvas::draw_command("@scene", "render", "42");
let _ = mirc::command(&format!("{frame} | toolbar -p scene @scene 0 0 120 32"));
```

`m_nyancat`, a separate project built on this, does exactly this: a pixel-art cat and rainbow
trail animate at 10 frames per second, like the original, in the toolbar, filling all the
room the toolbar has left. In mIRC 7.85 the pipeline, toolbar refresh included, kept up with
a target of 120 frames per second, though the 60 Hz test monitor could only show 60.

**Fitting the toolbar.** A button can size itself to the toolbar: fill all the room left
of the other buttons, at the height of their row, and follow the window as it is resized.
mIRC's toolbar is a standard Windows toolbar (class `mIRC_ToolBar`), so `TB_GETITEMRECT`
gives every visible button's rectangle, in real pixels at whatever display scaling is in
use. What it showed in mIRC 7.85:

- A picture button is its picture plus a padding on each side (21 pixels at 150% scaling),
  and the toolbar is as tall as its tallest button. The padding can't be asked for, but it
  is your own button's rectangle minus the picture size you gave mIRC.
- So the picture that fits the row is *the other buttons' height minus the padding*: 16
  pixels with mIRC's default icons, more with larger icons. A taller picture makes the
  whole toolbar taller, by the difference: a 32 pixel picture made this toolbar 55 pixels
  tall instead of 39, and 48 pixels made it 71.
- A button that is too wide wraps onto a second row and makes the toolbar twice as tall.
  Once it has wrapped, its own position is no use for measuring; the room is what's left
  after the previous button.
- The smallest picture mIRC takes is 16 pixels wide, so a button needs room for that plus
  its padding. With less it wraps onto a row of its own, which makes the toolbar taller.
- While mIRC is minimized nobody can see the toolbar, but mIRC still redraws it in full
  every frame (about 2.5 ms of its UI thread with 8 buttons), so skip frames then
  (`IsIconic` on the main window works from any thread).
- Buttons added to the toolbar later go after yours. Count the toolbar's buttons
  (`TB_BUTTONCOUNT`) to notice, and delete and add yours again to move them to the end.
- Change the size with `/toolbar -p name @scene 0 0 w h`, the same command that refreshes
  the picture, so nothing is removed or re-added.
- Measure on mIRC's UI thread, such as inside a drawing export, where sending messages to
  the toolbar is safe. Measuring makes the toolbar lay itself out again, which costs too
  much to do every frame with several buttons (about 0.8 ms each), so do it a couple of
  times a second.

**A picture is at most 256 × 256 pixels.** `/toolbar` rejects anything bigger with
"invalid image size", so one button can't fill a wide toolbar. Use as few
buttons as will fill it, side by side, each showing a slice of the picture window's bitmap
(`x y w h`). The slices are stacked one above the other in the bitmap, which stays 256
pixels wide, and size the window to exactly the height they need: a bigger
bitmap makes every frame cost mIRC more. The padding around each picture leaves a gap
between slices (20 pixels at 150% scaling), so the scene is drawn as if the gaps were there,
and its stars and rainbow carry on behind them. Send `/toolbar` commands that add or delete
buttons one at a time: mIRC does nothing with a `|` chain that contains `/toolbar -a` or
`-d`, and still reports success. Chains that start with `/drawdll` or `/toolbar -p` work.

**What it costs mIRC.** Only the last step has to run on mIRC's UI thread, so render each
frame on a worker thread and let the export just copy it in (about 0.02 ms).
Per frame, in mIRC 7.85, the UI thread is busy for about 0.65 ms with one 16 pixel button,
and about 2.2 ms with the 8 buttons of 32 pixels that fill this 4K display's
toolbar (1.8 ms at 16 pixels). At 10 frames per second that is about 2% of the thread's
time, and it would be three times that at 30. Nearly all of it is mIRC's own work: about
0.14 ms to receive a command, 0.15 ms to run `/drawdll`, and 0.15 to 0.35 ms to refresh each
button. mIRC has to do that on its UI thread, since it owns the window and the toolbar. If mIRC is too busy to
keep up (a long script, say), a worker can wait, and then skip the frames it missed instead
of sending them all at once: `m_nyancat` did this, and with the UI thread blocked for 1.5 s
its animation jumped forward 15 frames and carried on at 10 per second.

Not tested: AdiIRC (whether it supports `/drawdll` is unknown), and mIRC before 7.83, which
doesn't have `/drawdll`.

## The host: version, encoding and buffer sizes

`call.host()` or `mirust::host()` returns a `Host` describing the client that loaded the
DLL. It is mirust's equivalent of mIRC's `LOADINFO` struct, but **every value is valid on
every mIRC version**. mIRC added `LOADINFO`'s fields over time (and 5.6 – 5.71 have no
`LOADINFO` at all), so mirust fills in what older versions don't report. You never need
to check the version before reading one:

| `Host` method    | `LOADINFO` field (added in) | Returns        | Meaning, and value before mIRC had the field                               |
|------------------|-----------------------------|----------------|------------------------------------------------------------------------------|
| `version()`      | `mVersion` (5.8)            | `Version`      | the mIRC version, corrected for old reporting bugs. On 5.6 – 5.71, read from the executable's version resource |
| `main_window()`  | `mHwnd` (5.8)               | `WindowHandle` | mIRC's main window. On 5.6 – 5.71, taken from the call's `mWnd`, which is the same window |
| `keep_loaded()`  | `mKeep` (5.8)               | `bool`         | whether mIRC keeps the DLL loaded between calls; `false` before 5.8          |
| `unicode()`      | `mUnicode` (7.0)            | `bool`         | whether strings are UTF-16; `false` before 7.0                               |
| `beta()`         | `mBeta` (7.51)              | `u32`          | the public beta number, or 0 for a release; 0 before 7.51                    |
| `capacity()`     | `mBytes` (7.64)             | `usize`        | the longest response mIRC accepts, in code units (bytes, or UTF-16 units), including the NUL terminator; measured for older versions |
| `encoding()`     | —                           | `Encoding`     | `Utf16`, `Utf8` or `Ansi`: exactly how strings cross the boundary           |
| `client()`       | —                           | `Client`       | `Mirc` if the main window's class is mIRC's (`mIRC`, or `mIRC32` before 6.0), otherwise `AdiIrc`; `Client::of_window(window)` checks any window |

`Version` compares in release order and displays as mIRC does (`7.85`, `7.01`, `6.2`). The
minor part is two digits: `Version::new(6, 20)` is 6.2. Constants such as
`Version::V7_0` and `Version::V7_64` mark the releases that changed the DLL interface:

```rust
use mirust::{Call, Version};

fn features(call: Call) -> String {
    let version = call.host().version();
    if version >= Version::V7_0 {
        format!("mIRC {version}: Unicode")
    } else {
        format!("mIRC {version}: ANSI only")
    }
}

mirust::export!(features);
```

`WindowHandle` wraps an `HWND`. Convert it for other Win32 bindings with `as_raw()`. For
example, `windows::Win32::Foundation::HWND(handle.as_raw())` with the `windows` crate; with
`windows-sys` the raw pointer already is an `HWND`. `is_current_thread()` tells you whether
the calling thread owns the window.

**Encoding.** Your code always works with `&str` and `String`; mirust converts at the
boundary. On mIRC 7+ it asks for UTF-16 (unless `unicode(false)`, which gets UTF-8). On
mIRC 6 and older, strings use the system's ANSI code page, so characters it can't represent
are replaced by mIRC and Windows.

## Panics and errors

- A panic in an exported function halts the calling script (`Response::Halt`) instead of
  unwinding into mIRC, which would be undefined behaviour. mIRC keeps running.
- A panic in `on_load` is ignored. A panic in `on_unload` counts as `Unload::Allow`. A panic
  in a worker ends that worker.
- This relies on Rust's default `panic = "unwind"`. With `panic = "abort"` in your
  `Cargo.toml` profile, a panic takes mIRC down with it.
- mirust has no error type of its own. Report errors to the user by returning a command,
  for example `Response::command_with("echo -a $1-", format!("error: {e}"))`, or by
  returning an error string for `$dll()` callers to check.

## API at a glance

Everything public, from the crate root:

| Item | Kind | Purpose |
|------|------|---------|
| `config!(config)` | macro | optional: changes the `Config` of the built-in `LoadDll`/`UnloadDll`; at most once |
| `export!(f, g as "Name", path::h)` | macro | exports functions callable as `fn(Call) -> impl IntoResponse` |
| `Call` | struct | one invocation: input text, windows, flags, stop token, host |
| `Response` | enum | `Halt`, `Continue`, `Command { command, parms }`, `Return(String)`; constructors `command(cmd)` and `command_with(cmd, parms)` |
| `IntoResponse` | trait | implemented for `Response`, `String`, `&str` and `()` |
| `Config` | struct | `const` builder: `new`, `keep_loaded`, `unicode`, `on_load`, `on_unload`, `pin_module`, `exit_grace` |
| `UnloadReason` | enum | `Manual`, `Idle`, `Exit`, `Other(i32)` |
| `Unload` | enum | `Allow`, `Keep` (answer to an idle unload) |
| `spawn(f)` | fn | starts a background thread that is safe across unloads; `f: FnOnce(StopToken)` |
| `StopToken` | struct | `is_stopped()`, `wait()`, `wait_timeout(Duration) -> bool` |
| `host()` | fn | the `Host`, from anywhere |
| `dll_path()` | fn | this DLL's full path, for commands such as `/dll -u` |
| `Host` | struct | `client`, `version`, `beta`, `main_window`, `encoding`, `capacity`, `keep_loaded` |
| `Client` | enum | `Mirc`, `AdiIrc`; `Client::of_window(window)` |
| `Version` | struct | `new`, `from_raw`, `to_raw`, `major`, `minor`, `V5_6` … `V7_84` constants, `Display`, `Ord` |
| `Encoding` | enum | `Utf16`, `Utf8`, `Ansi`; `unit_size()` |
| `WindowHandle` | struct | `from_raw`, `as_raw`, `is_null`, `is_current_thread` |
| `buffer_capacity(version, reported_bytes)` | fn | the buffer-size rule mirust uses, for reference |
| `mirc::command(text)`, `mirc::evaluate(text)` | fn | run a command or evaluate an identifier in mIRC, from any thread |
| `mirc::Command`, `mirc::Evaluate`, `mirc::SendError` | structs, enum | the same with options (window, plain text, flood protection, event context, timeout), and why a request failed |
| `canvas` | module | render frames into mIRC picture windows with `/drawdll` (7.83+): `Draw`, `Canvas`, `Image`, `Update`, `IntoUpdate`, `rgb`, `draw_command` |
| `export_draw!(f, g as "Name")` | macro | exports functions callable as `fn(Draw) -> impl IntoUpdate`, for `/drawdll` |
| `worker` | module | documentation of background threads and exit handling |

Full API documentation: <https://docs.rs/mirust>.

## Compatibility

mirust reads `LOADINFO` field by field, and only the fields the reported version has.
Older versions pass a smaller struct, so it is never read or written past its end. mirust
corrects the known version-reporting bugs and limits responses to what each release
accepts:

| mIRC        | `LoadDll` version report | Strings¹       | Longest response (units incl. NUL) |
|-------------|--------------------------|----------------|------------------------------------|
| 5.6 – 5.71  | no `LoadDll`; read from the executable⁴ | ANSI code page | 900                 |
| 5.8 – 6.2   | often wrong; corrected   | ANSI code page | 900                                |
| 6.21 – 6.31 | correct                  | ANSI code page | 900                                |
| 6.32 – 6.35 | correct                  | ANSI code page | 4151²                              |
| 7.0 – 7.52  | correct                  | UTF-16         | 4151²                              |
| 7.53 – 7.63 | correct                  | UTF-16         | 4200                               |
| 7.64 – 7.83 | correct                  | UTF-16         | `mBytes / 2`³ (10240)              |
| 7.84+       | correct                  | UTF-16         | `mBytes` (10240)                   |

¹ With `Config::unicode(false)`, mIRC 7+ sends UTF-8 instead of UTF-16.

² The buffers hold 4200 units, but these versions reject results longer than 4150
characters as "line too long".

³ mIRC 7.64 – 7.83 reported `mBytes` as a UTF-16 byte count even to ANSI DLLs, twice the
real ANSI buffer. mirust halves it.

⁴ These versions never call `LoadDll` or `UnloadDll`, and unload the DLL after every
call. mirust reads their exact version from the executable's version resource (falling
back to 5.6, the lowest version with DLL support), takes mIRC's main window from the
call, and does what `LoadDll` and `UnloadDll` would around each call, so `on_load`,
`on_unload` and background threads behave as on later versions with
`Config::keep_loaded(false)`. Tested in 5.6 and 5.7.

Input from mIRC isn't limited by these values: mirust reads it up to its terminator, so
you always receive all of it.

Where these values come from:

- the mIRC changelog;
- disassembly of mIRC 6.03, 6.3, 6.31, 6.35, 7.14 – 7.42 and 7.85;
- the same test DLL run in mIRC 5.6, 5.7, 6.03, 6.12, 6.14, 6.15, 6.16, 6.17, 6.2, 6.21, 6.3, 6.31, 6.35, 7.14, 7.27, 7.29, 7.32, 7.42,
  7.52, 7.62, 7.72, 7.82, 7.83, 7.84 and 7.85. It checks the detected version, encoding
  (including non-ASCII text), the longest response, long input, `$dllcall()` delivery,
  and the exit grace period.

The docs for `Version` and `buffer_capacity` give the details.

Version differences to be aware of:

- `$dllcall()` exists from mIRC 6.1. Overlapping `$dllcall()`s into one DLL run
  concurrently in every version tested (6.12, 6.14 – 6.17 and later).
- `/noop`, used in this README to discard `$dllcall()`'s result and as a do-nothing
  callback, exists from mIRC 6.17. On 6.1 – 6.16, discard the result with `/set` (for
  example `set %ignored $dllcall(...)`) and name an alias of your own as the callback.
- mirust recognises mIRC by its main window's class (`mIRC`, or `mIRC32` before 6.0), so
  renamed copies (such as `mircx.exe`) are still reported correctly.

**AdiIRC** 4.4+ presents itself as mIRC 7.64 and is handled as such. mirust has not been
tested in AdiIRC.

## Limitations

- **mIRC's user can turn off SendMessage** (Options → Other, or the Lock dialog), in which
  case `mirust::mirc` requests fail with `Disabled` or `Failed`.
- **Drawing needs mIRC 7.83 or later** (`/drawdll`) and a picture window. A toolbar button
  showing one doesn't follow the window by itself; see
  [Drawing into mIRC windows](#drawing-into-mirc-windows).
- **Windows only.** mIRC loads only 32-bit x86 DLLs. AdiIRC loads x86, x64 or ARM64 DLLs,
  matching the AdiIRC build installed. The x64 and ARM64 builds compile, but haven't been
  run inside AdiIRC yet.
- **Not tested:** AdiIRC, and the exit grace period when Windows logs off or shuts down.
- **Overlapping `$dllcall()`s into the same DLL** can run each other's `$dllcall()`
  callback, because of how mIRC hands results back. Returned commands are protected (see
  [Overlapping `$dllcall()`s](#overlapping-dllcalls-into-the-same-dll)).
- **Several DLLs built with mirust** coordinate their exit grace periods; DLLs built
  without it don't take part.

## Troubleshooting

| Symptom | Likely cause |
|---------|--------------|
| mIRC or AdiIRC can't open the DLL | built for the wrong architecture (mIRC needs `i686-pc-windows-msvc`; AdiIRC needs its own build's architecture, see [Quick start](#quick-start)), missing `crate-type = ["cdylib"]`, wrong path, or the Visual C++ runtime isn't installed (link it statically, see [Quick start](#quick-start)) |
| mIRC reports an error about the routine | the name isn't listed in `export!`, or its case differs |
| Rebuilding fails because the DLL is in use | mIRC still has it loaded: `/dll -u <file>` first. If a worker is still running, or the module is pinned, the file stays locked until the worker returns or mIRC restarts |
| mIRC freezes during a call | slow work in `/dll` or `$dll()`: move it to `$dllcall()` or a background thread |
| mIRC crashes a while after `/dll -u` | a thread started with `std::thread::spawn` outlived the DLL: use `mirust::spawn` or `pin_module(true)` |
| A `$dllcall()` result never arrives | `Return` values are lost with `$dllcall()`: return a command that runs your callback with the result instead |
| The wrong `$dllcall()` callback runs, or one runs twice | overlapping `$dllcall()`s into the same DLL: pass `noop` as the callback and put the callback in the returned command; see [Overlapping `$dllcall()`s](#overlapping-dllcalls-into-the-same-dll) |
| Statics reset between calls | `keep_loaded(false)` |
| `/dll -u` does nothing | one of the DLL's functions (often a `$dllcall()`) was still running, so mIRC skipped the unload |
| Non-ASCII text turns into `?` | the host is mIRC 6 or older, which only supports the system code page |

## Upgrading from 0.x

1.0 is a rewrite and is not source-compatible with 0.x:

- `#[mirust_fn]` is gone. Write `fn name(call: Call) -> …`, list the functions in
  `mirust::export!`. The entry points are provided automatically; use `mirust::config!`
  to change their defaults.
- `MircResult { code, data, parms }` is replaced by `Response` and `IntoResponse`.
- `get_loadinfo()` is replaced by `mirust::host()`.
- The `windows` crate is no longer required; use `WindowHandle::as_raw()` for interop.
- `#[mirust_fn(dllcall = true)]` is replaced by checking `call.is_dllcall()`.

See [CHANGELOG.md](CHANGELOG.md) for everything that changed.

## Projects using mirust

- [m_nowplaying](https://github.com/irc-rs/m_nowplaying), lets mIRC and AdiIRC scripts ask
  Windows what media is playing, so you can show the current track in a channel
- [m_nyancat](https://github.com/irc-rs/m_nyancat), a pixel-art Nyan Cat animated in mIRC's
  toolbar

Built something with mirust? Add a link to your README with this button, and open an issue
or pull request to get your project listed above.

<a href="https://github.com/irc-rs/mirust"><img src="https://raw.githubusercontent.com/irc-rs/mirust/master/assets/powered-by-mirust.png" alt="Powered by mirust" width="220"></a>

```markdown
<a href="https://github.com/irc-rs/mirust"><img src="https://raw.githubusercontent.com/irc-rs/mirust/master/assets/powered-by-mirust.png" alt="Powered by mirust" width="220"></a>
```

## License

[MIT](LICENSE.md) © 2025-2026 Joshua Byrnes
