mod helpers;
mod loadinfo;
mod version;
mod win_utils;
mod ffi;

pub use helpers::*;
pub use loadinfo::get_loadinfo;
pub use mirust_macros::mirust_fn;

pub struct MircResult {
    pub code: i32,
    pub data: Option<String>,
    pub parms: Option<String>,
}