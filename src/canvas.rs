//! Draw into mIRC picture windows from Rust, with `/drawdll` (mIRC 7.83 and later).
//!
//! mIRC's own drawing commands (`/drawrect`, `/drawtext`, ...) cost a script command each,
//! and `/drawpic` needs a file on disk. `/drawdll` skips both: it calls a function in your
//! DLL and hands it the picture window's bitmap, so Rust can render a whole frame and copy
//! it straight in. In mIRC 7.85, a 1920×1080 frame takes about 2 ms to hand over, and 60
//! frames per second, what a 60 Hz display shows, is comfortable even at that size. (60 is
//! where our test monitor stops, not mIRC.)
//!
//! A function that draws takes a [`Draw`] and returns an [`Update`] (or anything that
//! implements [`IntoUpdate`], such as `()`). List it in [`export_draw!`](crate::export_draw),
//! then have a script, or your own worker thread, call it:
//!
//! ```
//! use mirust::canvas::{Draw, Image, Update, rgb};
//!
//! /// /drawdll -n @scene shade.dll gradient
//! fn gradient(draw: Draw) -> Update {
//!     let canvas = draw.canvas();
//!     let mut image = Image::new(canvas.width(), canvas.height());
//!     for (i, pixel) in image.pixels_mut().iter_mut().enumerate() {
//!         *pixel = rgb((i % 256) as u8, 64, 160);
//!     }
//!     canvas.draw(&image);
//!     Update::Redraw
//! }
//!
//! mirust::export_draw!(gradient);
//! # fn main() {}
//! ```
//!
//! # The picture window
//!
//! `/drawdll` works on a picture window: create one with `/window -p @scene x y w h` (add
//! `-h` to keep it hidden). Its bitmap is the window's *client area*, so it is the window's
//! width minus 22 and height minus 56 pixels, and mIRC won't make a window narrower than
//! about 198 pixels, so the smallest bitmap is about 176 pixels wide. [`Canvas::width`] and
//! [`Canvas::height`] tell you the real size.
//!
//! # Refresh
//!
//! [`draw_command`] builds the `/drawdll` command for your own export, quoting the DLL's
//! path for you. Send it with [`mirc::command`](crate::mirc::command) from a worker thread
//! for an animation, or run it from a script's timer:
//!
//! ```text
//! /drawdll -n @scene "C:\path\shade.dll" gradient any text for $1-
//! ```
//!
//! The `-n` stops mIRC redrawing the window itself; your function's [`Update`] does that.
//!
//! # Toolbar buttons
//!
//! A picture window can be a button's picture (`/toolbar -a name tip @scene`, 16×16 to
//! 256×256, and `x y w h` picks a part of the bitmap; anything bigger is refused, so a wide
//! toolbar needs several buttons side by side, each showing a slice). The button doesn't
//! follow the window, so refresh it after each frame with `/toolbar -p name @scene`. Both
//! commands can go in one string: `draw_command(..) + " | toolbar -p name @scene"`. The
//! `m_nyancat` project does this, and fills the whole toolbar.
//!
//! # Threads
//!
//! mIRC calls your function on its UI thread, and the [`Canvas`] is only valid until the
//! function returns, so [`Draw`] and [`Canvas`] can't be sent to another thread. Do heavy
//! rendering on a worker thread (see [`spawn`](crate::spawn)) into an [`Image`], and copy it
//! in from your function.

use core::ffi::c_void;
use core::mem::size_of;

use crate::{Host, WindowHandle, sys};

/// Packs red, green and blue into the `0x00RRGGBB` pixel format of [`Image`].
#[must_use]
pub const fn rgb(red: u8, green: u8, blue: u8) -> u32 {
    (red as u32) << 16 | (green as u32) << 8 | blue as u32
}

/// A block of pixels to copy into a [`Canvas`].
///
/// Pixels are `0x00RRGGBB` (see [`rgb`]), stored row by row from the top left corner.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image {
    width: u32,
    height: u32,
    pixels: Vec<u32>,
}

impl Image {
    /// A black image.
    ///
    /// # Panics
    ///
    /// If `width * height` doesn't fit in memory addressing.
    #[must_use]
    pub fn new(width: u32, height: u32) -> Self {
        let len = (width as usize)
            .checked_mul(height as usize)
            .expect("image too large");
        Self {
            width,
            height,
            pixels: vec![0; len],
        }
    }

    /// Width in pixels.
    #[must_use]
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Height in pixels.
    #[must_use]
    pub fn height(&self) -> u32 {
        self.height
    }

    /// The pixels, row by row.
    #[must_use]
    pub fn pixels(&self) -> &[u32] {
        &self.pixels
    }

    /// The pixels, row by row, to draw into. The length never changes.
    pub fn pixels_mut(&mut self) -> &mut [u32] {
        &mut self.pixels
    }

    /// Sets every pixel to `color`.
    pub fn fill(&mut self, color: u32) {
        self.pixels.fill(color);
    }

    /// A copy scaled up by a whole number of pixels, without smoothing: each pixel becomes a
    /// `factor` × `factor` block. For pixel art.
    ///
    /// # Panics
    ///
    /// If `factor` is 0 or the result is too large.
    #[must_use]
    pub fn scaled(&self, factor: u32) -> Self {
        assert!(factor > 0, "scale factor must be at least 1");
        let mut out = Self::new(self.width * factor, self.height * factor);
        let (w, f) = (self.width as usize, factor as usize);
        let out_w = out.width as usize;
        for (y, row) in self.pixels.chunks_exact(w.max(1)).enumerate() {
            let start = y * f * out_w;
            for (x, &pixel) in row.iter().enumerate() {
                out.pixels[start + x * f..start + (x + 1) * f].fill(pixel);
            }
            for copy in 1..f {
                out.pixels
                    .copy_within(start..start + out_w, start + copy * out_w);
            }
        }
        out
    }
}

/// The picture window's bitmap, for the duration of one call.
///
/// Not `Send` or `Sync`: mIRC owns the bitmap and the handles are only valid until your
/// function returns.
#[derive(Debug)]
pub struct Canvas {
    hdc: *mut c_void,
    bitmap: *mut c_void,
    width: u32,
    height: u32,
}

impl Canvas {
    /// Width of the bitmap in pixels.
    #[must_use]
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Height of the bitmap in pixels.
    #[must_use]
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Copies `image` into the top left corner of the bitmap. Whatever doesn't fit is
    /// clipped.
    pub fn draw(&self, image: &Image) {
        self.draw_at(image, 0, 0);
    }

    /// Copies `image` into the bitmap with its top left corner at (`x`, `y`). Whatever falls
    /// outside the bitmap is clipped.
    pub fn draw_at(&self, image: &Image, x: i32, y: i32) {
        if image.width == 0 || image.height == 0 {
            return;
        }
        let info = sys::BitmapInfoHeader {
            size: size_of::<sys::BitmapInfoHeader>() as u32,
            width: image.width as i32,
            // Negative: the rows run from the top down.
            height: -(image.height as i32),
            planes: 1,
            bit_count: 32,
            compression: 0,
            size_image: 0,
            x_pels_per_meter: 0,
            y_pels_per_meter: 0,
            colors_used: 0,
            colors_important: 0,
        };
        // SAFETY: `hdc` is the live memory DC mIRC passed for this call, `image.pixels` holds
        // `width * height` 32-bit pixels, and `info` describes exactly that layout.
        unsafe {
            sys::SetDIBitsToDevice(
                self.hdc,
                x,
                y,
                image.width,
                image.height,
                0,
                0,
                0,
                image.height,
                image.pixels.as_ptr().cast(),
                &info,
                sys::DIB_RGB_COLORS,
            );
        }
    }

    /// mIRC's memory device context for the window, to draw on with Win32 GDI directly.
    ///
    /// Valid only until your function returns. Don't delete it or select another bitmap
    /// into it.
    #[must_use]
    pub fn hdc(&self) -> *mut c_void {
        self.hdc
    }

    /// The window's `HBITMAP`, selected into [`hdc`](Self::hdc). Valid only until your
    /// function returns.
    #[must_use]
    pub fn bitmap(&self) -> *mut c_void {
        self.bitmap
    }
}

/// What mIRC should redraw after your function returns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Update {
    /// Let mIRC decide: it redraws unless the command used `-n`. With mirust's
    /// [`draw_command`], which uses `-n`, that means no redraw.
    Default,
    /// Redraw the whole window.
    Redraw,
    /// Redraw only this rectangle, in the bitmap's coordinates.
    Area {
        /// Left edge.
        left: i32,
        /// Top edge.
        top: i32,
        /// Right edge.
        right: i32,
        /// Bottom edge.
        bottom: i32,
    },
    /// Nothing changed; don't redraw.
    Skip,
}

impl Update {
    /// The `<update> <left> <top> <right> <bottom>` string mIRC expects back.
    pub(crate) fn reply(self) -> String {
        match self {
            Self::Default => "-1".to_owned(),
            Self::Redraw => "1".to_owned(),
            Self::Area {
                left,
                top,
                right,
                bottom,
            } => format!("1 {left} {top} {right} {bottom}"),
            Self::Skip => "0".to_owned(),
        }
    }
}

/// Converts what a drawing function returns into an [`Update`].
///
/// Implemented for [`Update`] and for `()`, which redraws the whole window.
pub trait IntoUpdate {
    /// Performs the conversion.
    fn into_update(self) -> Update;
}

impl IntoUpdate for Update {
    fn into_update(self) -> Update {
        self
    }
}

impl IntoUpdate for () {
    fn into_update(self) -> Update {
        Update::Redraw
    }
}

/// Everything mIRC passes to a drawing function: the [`Canvas`] and your text.
///
/// Not `Send`, because the [`Canvas`] must stay on mIRC's UI thread.
#[derive(Debug)]
pub struct Draw {
    data: String,
    canvas: Canvas,
    main_window: WindowHandle,
    active_window: WindowHandle,
    host: &'static Host,
}

impl Draw {
    /// The text after the export's name in the `/drawdll` command, as `$1-` would be.
    #[must_use]
    pub fn data(&self) -> &str {
        &self.data
    }

    /// The bitmap to draw into.
    #[must_use]
    pub fn canvas(&self) -> &Canvas {
        &self.canvas
    }

    /// mIRC's main window.
    #[must_use]
    pub fn main_window(&self) -> WindowHandle {
        self.main_window
    }

    /// The window that is active in mIRC.
    #[must_use]
    pub fn active_window(&self) -> WindowHandle {
        self.active_window
    }

    /// The running client; see [`Host`].
    #[must_use]
    pub fn host(&self) -> &'static Host {
        self.host
    }
}

/// The `/drawdll` command that calls your DLL's export `export` on picture window `window`
/// (such as `"@scene"`), with `data` available to it as [`Draw::data`].
///
/// The DLL's path is quoted, so it may contain spaces. The command uses `-n`: the export's
/// [`Update`] decides what is redrawn. Send it with [`mirc::command`](crate::mirc::command).
///
/// ```no_run
/// use mirust::{canvas, mirc};
///
/// // From a worker thread, once per frame:
/// let frame = 42;
/// let _ = mirc::command(&canvas::draw_command("@scene", "render", &frame.to_string()));
/// ```
#[must_use]
pub fn draw_command(window: &str, export: &str, data: &str) -> String {
    let path = crate::dll_path();
    let command = format!("drawdll -n {window} \"{path}\" {export}");
    if data.is_empty() {
        command
    } else {
        format!("{command} {data}")
    }
}

/// The three parts mIRC packs into `/drawdll`'s `data`: `hdc:0x… hbm:0x… data:<yours>`.
#[derive(Debug, PartialEq, Eq)]
struct Request<'a> {
    hdc: usize,
    bitmap: usize,
    data: &'a str,
}

fn parse_request(text: &str) -> Option<Request<'_>> {
    let rest = text.strip_prefix("hdc:")?;
    let (hdc, rest) = rest.split_once(' ')?;
    let rest = rest.strip_prefix("hbm:")?;
    let (bitmap, data) = match rest.split_once(' ') {
        Some((bitmap, rest)) => (bitmap, rest.strip_prefix("data:")?),
        None => (rest, ""),
    };
    Some(Request {
        hdc: parse_handle(hdc)?,
        bitmap: parse_handle(bitmap)?,
        data,
    })
}

fn parse_handle(text: &str) -> Option<usize> {
    let digits = text
        .strip_prefix("0x")
        .or_else(|| text.strip_prefix("0X"))
        .unwrap_or(text);
    usize::from_str_radix(digits, 16).ok().filter(|&h| h != 0)
}

/// Reads mIRC's `/drawdll` arguments. `None` if the export wasn't called by `/drawdll`
/// (for example by `/dll`), or the bitmap can't be inspected.
pub(crate) fn prepare(
    text: &str,
    main_window: WindowHandle,
    active_window: WindowHandle,
    host: &'static Host,
) -> Option<Draw> {
    let request = parse_request(text)?;
    let mut bitmap = sys::Bitmap {
        bm_type: 0,
        bm_width: 0,
        bm_height: 0,
        bm_width_bytes: 0,
        bm_planes: 0,
        bm_bits_pixel: 0,
        bm_bits: core::ptr::null_mut(),
    };
    // SAFETY: `bitmap` is a writable BITMAP, and its size is passed in.
    let written = unsafe {
        sys::GetObjectW(
            request.bitmap as *mut c_void,
            size_of::<sys::Bitmap>() as i32,
            (&raw mut bitmap).cast(),
        )
    };
    if written == 0 || bitmap.bm_width <= 0 || bitmap.bm_height <= 0 {
        return None;
    }
    Some(Draw {
        data: request.data.to_owned(),
        canvas: Canvas {
            hdc: request.hdc as *mut c_void,
            bitmap: request.bitmap as *mut c_void,
            width: bitmap.bm_width as u32,
            height: bitmap.bm_height as u32,
        },
        main_window,
        active_window,
        host,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgb_packs_channels() {
        assert_eq!(rgb(0x12, 0x34, 0x56), 0x0012_3456);
        assert_eq!(rgb(0, 0, 0), 0);
        assert_eq!(rgb(255, 255, 255), 0x00FF_FFFF);
    }

    #[test]
    fn parses_what_mirc_sends() {
        assert_eq!(
            parse_request("hdc:0xC4011991 hbm:0xF005160B data:hello world"),
            Some(Request {
                hdc: 0xC401_1991,
                bitmap: 0xF005_160B,
                data: "hello world"
            })
        );
        // No text after the export name.
        assert_eq!(
            parse_request("hdc:0x1A hbm:0x2B data:"),
            Some(Request {
                hdc: 0x1A,
                bitmap: 0x2B,
                data: ""
            })
        );
        assert_eq!(
            parse_request("hdc:0x1A hbm:0x2B"),
            Some(Request {
                hdc: 0x1A,
                bitmap: 0x2B,
                data: ""
            })
        );
        // The user's text may itself look like the header.
        assert_eq!(
            parse_request("hdc:0x1 hbm:0x2 data:hdc:0x3 hbm:0x4 data:x")
                .map(|r| (r.hdc, r.bitmap, r.data)),
            Some((1, 2, "hdc:0x3 hbm:0x4 data:x"))
        );
    }

    #[test]
    fn rejects_calls_that_did_not_come_from_drawdll() {
        assert_eq!(parse_request(""), None);
        assert_eq!(parse_request("just some text"), None);
        assert_eq!(parse_request("hdc:0x1 data:oops"), None);
        assert_eq!(parse_request("hdc:zz hbm:0x2 data:"), None);
        assert_eq!(parse_request("hdc:0x0 hbm:0x2 data:"), None);
    }

    #[test]
    fn replies_in_the_format_mirc_documents() {
        assert_eq!(Update::Default.reply(), "-1");
        assert_eq!(Update::Redraw.reply(), "1");
        assert_eq!(Update::Skip.reply(), "0");
        assert_eq!(
            Update::Area {
                left: 1,
                top: 2,
                right: 30,
                bottom: 40
            }
            .reply(),
            "1 1 2 30 40"
        );
        assert_eq!(().into_update(), Update::Redraw);
    }

    #[test]
    fn images_scale_without_smoothing() {
        let mut image = Image::new(2, 2);
        image.pixels_mut().copy_from_slice(&[1, 2, 3, 4]);
        let big = image.scaled(3);
        assert_eq!((big.width(), big.height()), (6, 6));
        let rows: Vec<&[u32]> = big.pixels().chunks(6).collect();
        assert_eq!(rows[0], [1, 1, 1, 2, 2, 2]);
        assert_eq!(rows[2], [1, 1, 1, 2, 2, 2]);
        assert_eq!(rows[3], [3, 3, 3, 4, 4, 4]);
        assert_eq!(rows[5], [3, 3, 3, 4, 4, 4]);
        assert_eq!(image.scaled(1), image);
    }

    #[test]
    fn empty_images_are_fine() {
        let image = Image::new(0, 5);
        assert!(image.pixels().is_empty());
        assert_eq!(image.scaled(2).width(), 0);
    }

    mod gdi {
        use super::super::*;

        #[link(name = "gdi32")]
        unsafe extern "system" {
            fn CreateCompatibleDC(hdc: *mut c_void) -> *mut c_void;
            fn CreateCompatibleBitmap(hdc: *mut c_void, w: i32, h: i32) -> *mut c_void;
            fn SelectObject(hdc: *mut c_void, object: *mut c_void) -> *mut c_void;
            fn GetPixel(hdc: *mut c_void, x: i32, y: i32) -> u32;
            fn DeleteObject(object: *mut c_void) -> i32;
            fn DeleteDC(hdc: *mut c_void) -> i32;
        }
        #[link(name = "user32")]
        unsafe extern "system" {
            fn GetDC(hwnd: *mut c_void) -> *mut c_void;
            fn ReleaseDC(hwnd: *mut c_void, hdc: *mut c_void) -> i32;
        }

        /// COLORREF is 0x00BBGGRR.
        fn colorref(rgb: u32) -> u32 {
            (rgb & 0xFF) << 16 | (rgb & 0xFF00) | (rgb >> 16) & 0xFF
        }

        #[test]
        fn draws_into_a_memory_dc_the_way_mirc_provides_one() {
            unsafe {
                let screen = GetDC(core::ptr::null_mut());
                let hdc = CreateCompatibleDC(screen);
                let bitmap = CreateCompatibleBitmap(screen, 8, 6);
                let old = SelectObject(hdc, bitmap);

                let handles = format!("hdc:{:#X} hbm:{:#X} data:x", hdc as usize, bitmap as usize);
                let draw = prepare(
                    &handles,
                    WindowHandle::from_raw(core::ptr::null_mut()),
                    WindowHandle::from_raw(core::ptr::null_mut()),
                    crate::host(),
                )
                .expect("prepare");
                assert_eq!(draw.data(), "x");
                let canvas = draw.canvas();
                assert_eq!((canvas.width(), canvas.height()), (8, 6));

                let mut image = Image::new(4, 3);
                image.fill(rgb(1, 2, 3));
                image.pixels_mut()[0] = rgb(200, 100, 50); // top left
                image.pixels_mut()[11] = rgb(9, 8, 7); // bottom right
                canvas.draw_at(&image, 2, 1);

                // The top row is at the top: no vertical flip.
                assert_eq!(GetPixel(hdc, 2, 1), colorref(rgb(200, 100, 50)));
                assert_eq!(GetPixel(hdc, 5, 3), colorref(rgb(9, 8, 7)));
                assert_eq!(GetPixel(hdc, 3, 2), colorref(rgb(1, 2, 3)));

                SelectObject(hdc, old);
                DeleteObject(bitmap);
                DeleteDC(hdc);
                ReleaseDC(core::ptr::null_mut(), screen);
            }
        }
    }
}
