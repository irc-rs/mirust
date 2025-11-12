# Mirust

**Mirust** is a Rust SDK designed to simplify the development of mIRC and AdiIRC-compatible DLLs. It abstracts away the low-level complexity of Windows DLL exports and mIRC-specific quirks, allowing developers to write clean, idiomatic Rust code that integrates seamlessly with IRC clients.

## Features

- **Effortless DLL Exporting**: Annotate your Rust functions with `#[mirust_fn]` to expose them to mIRC/AdiIRC.
- **Broad Compatibility**: Supports mIRC versions starting from 5.6—the earliest version with DLL support.
- **Modern Loadinfo Support**: Provides a unified `loadinfo` object across all mIRC versions, including:
  - `m_version` (added in 5.8)
  - `m_hwnd` (added in 5.8)
  - `m_keep` (added in 5.8)
  - `m_unicode` (added in 7.0)
  - `m_beta` (added in 7.51)
  - `m_bytes` (added in 7.64)
- **Version Correction**: Automatically fixes incorrect version reporting in mIRC versions 5.8–6.2.
- **String Encoding Made Easy**: Seamlessly handles ANSI, UTF-8, and UTF-16 conversions - just use standard Rust `String`s.
- **Safe Pointer Interactions**: Read and write to mIRC memory safely and idiomatically.

## Getting Started

To use Mirust in your project:

1. Create a new Rust library crate.
    ```sh
    cargo new --lib my_library
    ```

2. In your `Cargo.toml`, specify the crate type:

    ```toml
    [lib]
    crate-type = ["cdylib"]
    ```

3. Add dependencies:

    ```toml
    [dependencies]
    mirust = { version = "0.2" }
    windows = { version = "0.62" }
    ```

## Example Usage

Here are three examples demonstrating how to export mIRC-compatible functions using Mirust:

```rs
use mirust::mirust_fn;
use windows::{Win32::Foundation::HWND, core::BOOL};

/// Handles a custom mIRC command invoked via `/dll` or `$dll`.
/// This function constructs a command string to be executed and returns it
/// along with a formatted message based on the input `data`.
/// 
/// `code: 2` instructs mIRC to execute the returned `data` as a command.
/// The `parms` field is used to pass additional context or feedback to the script.
/// 
/// Example usage in mIRC:
///   /dllcall my_library.dll my_command Hello World
#[mirust_fn]
fn my_command(
    m_wnd: HWND,
    a_wnd: HWND,
    data: String,
    parms: String,
    show: BOOL,
    nopause: BOOL,
) -> mirust::MircResult {

    let command = "echo -st * Message from 🦀 Rust: $1-".to_string();
    let my_string = format!("You sent me: {}", data);

    mirust::MircResult {
        code: 2, // Instructs mIRC to execute `data` as a command
        data: Some(command),
        parms: Some(my_string),
    }
}

/// Handles a custom mIRC identifier invoked via `$dll()`.
/// This function returns a static string response to the calling script.
/// 
/// `code: 3` tells mIRC to treat the `data` field as the return value of the identifier.
/// This is used for synchronous calls via `$dll()`.
/// 
/// Example usage in mIRC:
///   /echo -at $dll(my_library.dll, my_identifier, Hello World!)
#[mirust_fn]
fn my_identifier(
    m_wnd: HWND,
    a_wnd: HWND,
    data: String,
    parms: String,
    show: BOOL,
    nopause: BOOL,
) -> mirust::MircResult {

    let my_string = "Hello from 🦀 Rust".to_string();

    mirust::MircResult {
        code: 3, // Return the result to a $dll call from mIRC
        data: Some(my_string),
        parms: None,
    }
}

/// Marks this function for asynchronous invocation via `$dllcall()` from mIRC scripting.
/// The `dllcall = true` attribute ensures that the function is dispatched on a worker thread,
/// allowing it to perform blocking or long-running operations (e.g., sleep, I/O) without
/// interfering with the main GUI thread. This prevents UI freezes and maintains responsiveness.
/// Attempting to invoke this function using `$dll()` will result in an immediate return,
/// as `$dll()` executes on the main thread and does not support asynchronous dispatch.
///
/// Example usage in mIRC:
///   /noop $dllcall(my_library.dll, my_return_alias, my_long_running_fn, Hello World!)
/// 
/// - Will call $my_return_alias(C:\path\to\my_library.dll) after 10 seconds
#[mirust_fn(dllcall = true)]
fn my_long_running_fn(
    m_wnd: HWND,
    a_wnd: HWND,
    data: String,
    parms: String,
    show: BOOL,
    nopause: BOOL,
) -> mirust::MircResult {

    // Sleep for 10 seconds to simulate a long-running operation.
    sleep(std::time::Duration::from_secs(10));

    // The return value is ignored by mIRC when using `$dllcall()`.
    // This function is executed asynchronously, and mIRC does not consume or display
    // the returned result. The structure is maintained for consistency and internal use.
    mirust::MircResult {
        code: 1, // Continue (ignored)
        data: None,
        parms: None,
    }
}
```

## Building

To compile your Rust project into a Windows-compatible DLL for use with mIRC or AdiIRC, use the `--target` flag with Cargo to specify the appropriate architecture.

### 🔹 Current Target (mIRC is x86-only)

mIRC currently supports only 32-bit DLLs, so you must compile using the `i686-pc-windows-msvc` target:

```sh
cargo build --release --target=i686-pc-windows-msvc
```

This will produce a DLL at:

```
target/i686-pc-windows-msvc/release/your_library.dll
```

You can then load this DLL in mIRC using `/dll` or `$dll()`.

### 🔸 Future Support for mIRC (x64 and ARM64)

mIRC is expected to support 64-bit and ARM64 DLLs in upcoming releases. You can prepare builds for those targets as follows:

#### For 64-bit Windows (x86_64):

```sh
cargo build --release --target=x86_64-pc-windows-msvc
```

#### For ARM64 Windows:

```sh
cargo build --release --target=aarch64-pc-windows-msvc
```

> ✅ Tip: You can install additional targets using:
> ```sh
> rustup target add i686-pc-windows-msvc
> rustup target add x86_64-pc-windows-msvc
> rustup target add aarch64-pc-windows-msvc
> ```

### 📦 Distribution

When distributing your DLL:
- Only include the `.dll` file from the `target/.../release/` directory.
- Ensure that your DLL matches the architecture of the mIRC client.
- You may optionally include a README or version manifest for clarity.

## License

Licensed under the [MIT License](https://opensource.org/licenses/MIT).  
© 2025 Joshua Byrnes