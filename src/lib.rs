mod ffi;
mod helpers;
mod loadinfo;
mod threads;
mod version;
mod win_utils;

pub use helpers::*;
pub use loadinfo::get_loadinfo;
pub use mirust_macros::mirust_fn;
pub use threads::is_main_thread;

pub struct MircResult {
    pub code: i32,
    pub data: Option<String>,
    pub parms: Option<String>,
}