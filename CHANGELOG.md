# Changelog

## 1.2.0

### Added

- `canvas`: render frames from Rust straight into an mIRC picture window, through mIRC's
  `/drawdll` (mIRC 7.83 and later). `export_draw!` exports functions of the form
  `fn(Draw) -> impl IntoUpdate`; `Draw` and `Canvas` give access to the window's bitmap,
  `Image` holds `0x00RRGGBB` pixels to copy in (`Canvas::draw`, `draw_at`), and `Update`
  says what mIRC should redraw (`Redraw`, `Area`, `Skip`, `Default`). `draw_command` builds
  the `/drawdll` command for one of your own exports, quoting the DLL's path.
  - Fast: in mIRC 7.85 a 1920 × 1080 frame takes about 2 ms to copy in and a 398 × 264
    frame 0.25 ms, against about 6 ms and 11 ms for drawing a BMP with `/drawpic` (before
    counting writing the file).
  - A picture window can be a toolbar button's picture. The button doesn't follow the
    window, so refresh it with `/toolbar -p name @window` after each frame; both commands
    fit in one `mirc::command` string.
  - Calling a drawing export any way other than `/drawdll` does nothing; a panic halts the
    script.
  - Tested in mIRC 7.85: each `Update` variant, a panicking export, a DLL path containing a
    space, and 30, 60 and 120 frames per second into a toolbar button, with every frame
    sent reaching the export.
- `dll_path()`: the DLL's own full path, for commands that name it, such as `/dll -u` to
  unload itself from a worker (the worker keeps the DLL mapped until it returns).
- Notes on drawing into a toolbar button in the README (the 256 pixel limit, sizing to the
  toolbar with `TB_GETITEMRECT`, what it costs mIRC), from `m_nyancat`, a separate project
  that animates Nyan Cat in mIRC's toolbar with `canvas`.

## 1.1.0

### Added

- `mirc`: run commands and evaluate identifiers in mIRC from any thread, through mIRC's
  SendMessage interface. `mirc::command` and `mirc::evaluate`, plus `mirc::Command` and
  `mirc::Evaluate` for options (target window, plain text, flood protection, event
  context, timeout).
  - Calls never affect each other: from mIRC 6.2 each uses its own exclusively created,
    never-reused shared-memory block; before 6.2, calls from the DLL take turns.
  - Every failure is a `mirc::SendError`, including mIRC exiting (refused at once rather
    than waiting), timeouts, SendMessage being disabled, and mIRC's detailed error codes
    from 7.33. Text that doesn't fit is rejected, never truncated.
  - Commands run as if typed: a leading `/` is added if missing, so identifiers aren't
    evaluated unless the text starts with `//`.
  - Tested in mIRC 6.03, 6.12, 6.2, 7.14, 7.32, 7.52 and 7.85, including 200 commands in a
    row on mIRC's UI thread and four threads each making 300 requests at once, all with
    correct results. In 7.85, also event context through `/signal`, and SendMessage
    turned off (`Disabled` on every thread).

### Changed

- CI and release workflows use `actions/checkout@v7` (was `v4`).

### Fixed

- The README said mIRC 6.1 – 6.16 run only one `$dllcall()` per DLL at a time. That came
  from a test that used `/noop`, which those versions lack; they run `$dllcall()`s
  concurrently like later versions. The README also now notes that `/noop` needs 6.17.

## 1.0.0

A complete rewrite. Not source-compatible with 0.x; see "Upgrading from 0.x" in the README.

### Changed

- The `#[mirust_fn]` proc-macro and the `mirust_macros` dependency are replaced by two
  declarative macros: `export!` lists the functions mIRC can call, and the optional
  `config!` changes the defaults. The `windows` dependency is gone too; mirust has no
  dependencies.
- `LoadDll` and `UnloadDll` are built in, so a DLL needs only its functions and
  `export!`. `config!` sets their `Config`: `keep_loaded`, `unicode`, `on_load` and
  `on_unload` hooks, `pin_module` and `exit_grace`.
- Exported functions take a `Call` and return anything implementing `IntoResponse`
  (`Response`, `String`, `&str` or `()`), replacing the six raw arguments and
  `MircResult`. `Call` provides the decoded input, the windows, the quiet and no-pause
  flags, `is_dllcall()` (replacing `#[mirust_fn(dllcall = true)]`), a stop token and the
  host.
- `get_loadinfo()` and the public `LOADINFO` struct are replaced by `host()` and `Host`.
  `Host` covers every `LOADINFO` field (`version`, `main_window`, `keep_loaded`,
  `unicode`, `beta`, `capacity`) with a valid value on every mIRC version, including
  those that predate the field, so there are no `Option`s to check.
- Window handles are `WindowHandle` rather than the `windows` crate's `HWND`; convert with
  `as_raw()`.

### Added

- `Version`: mIRC versions that compare in release order and display as mIRC does, with
  the reporting bugs of mIRC 5.8 – 6.2 corrected. On mIRC 5.6 – 5.71, which never call
  `LoadDll`, the version is read from the executable's version resource.
- `Client` and `Client::of_window`: mIRC or AdiIRC, from the main window's class, so
  renamed executables are still recognised.
- `Encoding` and `buffer_capacity`, describing how strings cross the boundary and how long
  a response can be on each version.
- `spawn` and `StopToken`: background threads that keep the DLL mapped until they
  finish, so an unload can no longer crash mIRC, and that are told when mIRC unloads the
  DLL.
- `Config::exit_grace` (default 1 s): a bounded wait for workers and running
  `$dllcall()`s when mIRC exits, including exits mIRC doesn't report through `UnloadDll`
  (detected by watching mIRC's main window). Several mirust DLLs share the wait, so mIRC's
  exit is delayed by the longest grace period, not the sum.
- `Call::stop_token`, so a long-running `$dllcall()` can notice mIRC exiting.
- Reliable `$dllcall()` commands. mIRC keeps each DLL's pending `$dllcall()` result in a
  single slot, so overlapping calls can overwrite each other's command. mirust queues
  commands returned by `$dllcall()`s itself and has mIRC call back into the DLL to run
  each exactly once, in order. Scripts need no changes. Not available with
  `keep_loaded(false)`.
- `Config::pin_module`: keep the DLL in memory for threads mirust doesn't manage.
- 64-bit and ARM64 builds (`x86_64-pc-windows-msvc`, `aarch64-pc-windows-msvc`) for the
  matching AdiIRC builds. mIRC remains 32-bit x86 only.
- Reserved exports: `__mirust_config`, `__mirust_deliver` and `__mirust_run`. They use
  mIRC's calling convention, so a script calling one by name does nothing harmful.

### Fixed

- Response lengths now follow the running mIRC version, instead of a fixed 900 bytes: 900
  up to 6.31, 4151 for 6.32 – 7.52 (mIRC rejects longer strings as "line too long"), 4200
  for 7.53 – 7.63, and `mBytes` from 7.64. `mBytes` is halved on 7.64 – 7.83, which
  reported a UTF-16 byte count even to ANSI DLLs and could overflow their buffers.
- Input is read up to its terminator, so long input is no longer cut short.
- Responses are truncated at a character boundary instead of possibly splitting a UTF-16
  surrogate pair or a multi-byte character.
- ANSI-mode DLLs on mIRC 7+ are sent UTF-8, not the system code page; mirust now decodes
  them as UTF-8.
- `LOADINFO` is no longer read as a whole struct. mIRC before 7.0 passes a 12-byte struct,
  and v0 read past the end of it.
- `UnloadDll` no longer converts mIRC's integer into a Rust enum, which was undefined
  behaviour for unexpected values.
- Panics halt the calling script instead of aborting mIRC.
- On mIRC 5.6 – 5.71, which never call `LoadDll` or `UnloadDll`, mirust now does what
  they would around each call: the host's main window is known, and `on_load`,
  `on_unload` and background threads behave as with `keep_loaded(false)` on later
  versions.
- Input strings with invalid ANSI bytes are no longer discarded.

### Tested

The same test DLL was run inside mIRC 5.6, 5.7, 6.03, 6.12, 6.14, 6.15, 6.16, 6.17, 6.2,
6.21, 6.3, 6.31, 6.35, 7.14, 7.27, 7.29, 7.32, 7.42, 7.52, 7.62, 7.72, 7.82, 7.83, 7.84 and
7.85. It checks version detection, encoding (including non-ASCII text), the longest
response, long input, `$dllcall()` delivery and the exit grace period. AdiIRC hasn't been
tested yet.
