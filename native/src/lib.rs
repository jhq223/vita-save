#![no_std]
#![deny(unsafe_op_in_unsafe_fn)]
// GPL-3.0-or-later. Firmware mount procedure derived from VitaShell / vita-save-keeper.
// See ../UPSTREAM.md. This module deliberately uses neither allocation nor unwinding.
use core::ffi::{c_char, c_void};

#[repr(C)]
pub struct MountArgs {
    id: i32,
    process_title_id: *const c_char,
    path: *const c_char,
    desired: *const c_char,
    key: *const c_void,
    mount_point: *mut c_char,
}
const _: () = assert!(size_of::<MountArgs>() == 24);

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    loop {
        core::hint::spin_loop();
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn module_start(_: usize, _: *const c_void) -> i32 {
    0
}
#[unsafe(no_mangle)]
pub extern "C" fn module_stop(_: usize, _: *const c_void) -> i32 {
    0
}

#[cfg(feature = "user")]
unsafe extern "C" {
    fn vitaSaveKernelMountById(args: *mut MountArgs) -> i32;
}
/// # Safety
/// Arguments and all pointed-to buffers must satisfy the mount syscall contract.
#[cfg(feature = "user")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vitaSaveUserMountById(args: *mut MountArgs) -> i32 {
    unsafe { vitaSaveKernelMountById(args) }
}

#[cfg(feature = "kernel")]
mod kernel;
