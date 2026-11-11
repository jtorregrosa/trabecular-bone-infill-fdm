//! WebAssembly entry points of bone-infill.
//!
//! The module exposes the web app's API ([`bone_infill::app::respond`]) to a
//! Web Worker: the worker passes a method, a URL and a body, and reads back a
//! status, a content type and a body. The page is the same one the native
//! server uses; a small shim routes its `fetch("api/...")` calls here.
//!
//! The interface is a plain C ABI over the module's linear memory rather than
//! `wasm-bindgen`, so building needs nothing but the `wasm32-unknown-unknown`
//! target:
//!
//! * `alloc(len)` reserves a buffer the caller fills;
//! * `request(...)` consumes three such buffers (method, URL, body) and
//!   stores the reply;
//! * `reply_*()` describe the stored reply, valid until the next `request`.
//!
//! Everything runs on one thread: without wasm threads, Rayon executes its
//! parallel iterators sequentially.

use bone_infill::app::{Reply, Shared, respond};
use std::sync::{Mutex, OnceLock};

/// Voxel budget in the browser. About 15 bytes are held per voxel, plus the
/// meshes, and a tab cannot be assumed to have gigabytes to spare.
const MAX_VOXELS: f64 = 16.0e6;

#[link(wasm_import_module = "env")]
unsafe extern "C" {
    /// Seconds on a monotonic clock (`performance.now() / 1000`).
    fn now() -> f64;
}

fn clock() -> f64 {
    // SAFETY: `now` is provided by the worker, takes no arguments and has no
    // side effects on the module.
    unsafe { now() }
}

static STATE: OnceLock<Shared> = OnceLock::new();
static REPLY: Mutex<Option<Reply>> = Mutex::new(None);

/// Reserves `len` bytes and returns their address. The buffer must be handed
/// to [`request`], which releases it.
#[unsafe(no_mangle)]
pub extern "C" fn alloc(len: usize) -> *mut u8 {
    let mut buffer = Vec::<u8>::with_capacity(len.max(1));
    let pointer = buffer.as_mut_ptr();
    std::mem::forget(buffer);
    pointer
}

/// Takes back a buffer created by [`alloc`] and filled with `len` bytes.
///
/// # Safety
/// `pointer` must come from `alloc(len)` and must not be used afterwards.
unsafe fn take(pointer: *mut u8, len: usize) -> Vec<u8> {
    // SAFETY: guaranteed by the caller; the capacity matches `alloc`.
    unsafe { Vec::from_raw_parts(pointer, len, len.max(1)) }
}

/// Answers one request and stores the reply for the `reply_*` functions.
///
/// # Safety
/// Each pointer must come from `alloc` with the matching length, filled by
/// the caller, and is consumed by this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn request(
    method: *mut u8,
    method_len: usize,
    url: *mut u8,
    url_len: usize,
    body: *mut u8,
    body_len: usize,
) {
    bone_infill::util::set_time_source(clock);
    // SAFETY: forwarded from this function's contract.
    let (method, url, body) = unsafe { (take(method, method_len), take(url, url_len), take(body, body_len)) };
    let state = STATE.get_or_init(|| Shared::with_voxel_limit(MAX_VOXELS));
    let reply = respond(state, &String::from_utf8_lossy(&method), &String::from_utf8_lossy(&url), &body);
    *REPLY.lock().unwrap() = Some(reply);
}

fn with_reply<T: Default>(read: impl FnOnce(&Reply) -> T) -> T {
    REPLY.lock().unwrap().as_ref().map(read).unwrap_or_default()
}

#[unsafe(no_mangle)]
pub extern "C" fn reply_status() -> u32 {
    with_reply(|r| r.status as u32)
}

#[unsafe(no_mangle)]
pub extern "C" fn reply_body() -> usize {
    with_reply(|r| r.body.as_ptr() as usize)
}

#[unsafe(no_mangle)]
pub extern "C" fn reply_body_len() -> usize {
    with_reply(|r| r.body.len())
}

#[unsafe(no_mangle)]
pub extern "C" fn reply_type() -> usize {
    with_reply(|r| r.content_type.as_ptr() as usize)
}

#[unsafe(no_mangle)]
pub extern "C" fn reply_type_len() -> usize {
    with_reply(|r| r.content_type.len())
}

/// File name to save the reply under, if it is a download (else length 0).
#[unsafe(no_mangle)]
pub extern "C" fn reply_name() -> usize {
    with_reply(|r| r.download.as_ref().map_or(0, |n| n.as_ptr() as usize))
}

#[unsafe(no_mangle)]
pub extern "C" fn reply_name_len() -> usize {
    with_reply(|r| r.download.as_ref().map_or(0, String::len))
}
