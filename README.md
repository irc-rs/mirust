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
    mirust = { version = "0.1" }
    windows = { version = "0.62" }
    ```

## Example Usage

Here are two examples demonstrating how to export mIRC-compatible functions using Mirust:

```rs
use mirust::mirust_fn;
use windows::{Win32::Foundation::HWND, core::BOOL};

#[mirust_fn]
fn my_command(
    m_wnd: HWND,
    a_wnd: HWND,
    data: String,
    parms: String,
    show: BOOL,
    nopause: BOOL,
) -> mirust::MircResult {

    let command = "/echo -st * Message from 🦀 Rust: $1-".to_string();
    let my_string = format!("You sent me: {}", data);

    mirust::MircResult {
        code: 2, // Run command
        data: Some(command),
        parms: Some(my_string),
    }
}

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
```

## License

Licensed under the [MIT License](https://opensource.org/licenses/MIT).  
© 2025 Joshua Byrnes