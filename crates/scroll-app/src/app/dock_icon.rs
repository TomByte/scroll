//! The app's icon in the Dock. The app runs as a plain binary with no bundle to carry an icon,
//! so it hands AppKit one when it starts.

// `objc`'s macros test a `cargo-clippy` feature this crate does not declare.
#![allow(unexpected_cfgs)]

use std::ffi::c_void;

use objc::runtime::Object;
use objc::{class, msg_send, sel, sel_impl};

const APP_ICON_PNG: &[u8] = include_bytes!("../../assets/app-icon.png");

/// Must run on the main thread, once the application exists.
pub(super) fn show_app_icon() {
    // SAFETY: plain AppKit calls on the main thread; `APP_ICON_PNG` is static, and NSData copies it.
    unsafe {
        let data: *mut Object = msg_send![
            class!(NSData),
            dataWithBytes: APP_ICON_PNG.as_ptr().cast::<c_void>()
            length: APP_ICON_PNG.len()
        ];
        let image: *mut Object = msg_send![class!(NSImage), alloc];
        let image: *mut Object = msg_send![image, initWithData: data];
        let application: *mut Object = msg_send![class!(NSApplication), sharedApplication];
        let _: () = msg_send![application, setApplicationIconImage: image];
    }
}
