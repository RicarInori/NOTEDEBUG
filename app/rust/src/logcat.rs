//! Reporting Rust failures where an Android developer can see them.
//!
//! Rust's `stderr` does not reach logcat — a panic prints to a stream nobody is reading, and the
//! process simply disappears after the `abort`. That turns every failure inside the platform view
//! into "the app closed", which is the least debuggable outcome there is. liblog is part of the NDK,
//! so writing there needs no dependency.

use std::ffi::{c_char, CString};

const ANDROID_LOG_DEBUG: i32 = 3;
const ANDROID_LOG_ERROR: i32 = 6;

#[link(name = "log")]
extern "C" {
    fn __android_log_write(prio: i32, tag: *const c_char, text: *const c_char) -> i32;
}

/// Write one debugging line to logcat under `tag`.
pub fn log_debug(tag: &str, message: &str) {
    write(ANDROID_LOG_DEBUG, tag, message);
}

/// Write one line to logcat under `tag`.
pub fn log_error(tag: &str, message: &str) {
    write(ANDROID_LOG_ERROR, tag, message);
}

fn write(priority: i32, tag: &str, message: &str) {
    // The tag has to be NUL-terminated; an interior NUL in the tag is a caller bug, not a reason to
    // lose the message, so it falls back to a fixed tag.
    let tag = CString::new(tag).unwrap_or_else(|_| CString::new("Inkweave").expect("literal"));
    let Ok(message) = CString::new(message) else {
        return;
    };
    // SAFETY: both pointers are NUL-terminated and outlive this call.
    unsafe { __android_log_write(priority, tag.as_ptr(), message.as_ptr()) };
}

/// Route panics to logcat.
///
/// Called once from the host's bootstrap. Without it a panic is invisible: the hook prints to stderr,
/// stderr goes nowhere on Android, and the abort that follows looks like a silent crash.
pub fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        log_error("InkweavePanic", &info.to_string());
        previous(info);
    }));
}
