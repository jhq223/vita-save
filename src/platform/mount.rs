use crate::saves::Game;
use anyhow::{Context, Result, ensure};
use std::{
    ffi::{CString, c_char, c_void},
    path::Path,
    ptr,
    sync::Mutex,
};
use vitasdk_sys::*;

static MOUNT_MODULES: Mutex<Option<Modules>> = Mutex::new(None);
const MODULE_ALREADY_LOADED: i32 = 0x8002_D013u32 as i32;
#[repr(C)]
struct MountArgs {
    id: i32,
    title: *const c_char,
    path: *const c_char,
    desired: *const c_char,
    key: *const c_void,
    mount: *mut c_char,
}
#[repr(C)]
struct TaiArgs {
    size: usize,
    pid: i32,
    args: usize,
    argp: *mut c_void,
    flags: i32,
}
const _: () = assert!(size_of::<MountArgs>() == 24);
const _: () = assert!(size_of::<TaiArgs>() == 20);

unsafe extern "C" {
    fn vitaSaveUserMountById(args: *mut MountArgs) -> i32;
    fn taiLoadStartKernelModuleForUser(path: *const c_char, args: *mut TaiArgs) -> i32;
    fn taiStopUnloadKernelModuleForUser(
        id: i32,
        args: *mut TaiArgs,
        options: *mut c_void,
        result: *mut i32,
    ) -> i32;
}
struct Modules {
    kernel: i32,
    user: i32,
}
impl Modules {
    fn load() -> Result<Self> {
        // Kernel modules need a global device path. app0: is process-local;
        // querying its backing path through the ForShell API requires privileges
        // unavailable to this app. Use the VPK install path, as vita-savemgr does.
        let kernel_path = c"ux0:app/VSAVE0001/module/vita-save-kernel.skprx";
        let mut args = TaiArgs {
            size: size_of::<TaiArgs>(),
            pid: 0,
            args: 0,
            argp: ptr::null_mut(),
            flags: 0,
        };
        let mut modules = Self {
            kernel: unsafe { taiLoadStartKernelModuleForUser(kernel_path.as_ptr(), &mut args) },
            user: -1,
        };
        ensure!(
            modules.kernel >= 0 || modules.kernel == MODULE_ALREADY_LOADED,
            "Mount kernel module ({}): {:#010x}",
            kernel_path.to_string_lossy(),
            modules.kernel
        );
        let mut status = 0;
        modules.user = unsafe {
            sceKernelLoadStartModule(
                c"app0:module/vita-save-user.suprx".as_ptr(),
                0,
                ptr::null_mut(),
                0,
                ptr::null_mut(),
                &mut status,
            )
        };
        ensure!(
            modules.user >= 0 && status >= 0,
            "Mount user module: {:#010x}, status {status:#010x}",
            modules.user
        );
        Ok(modules)
    }
}
impl Drop for Modules {
    fn drop(&mut self) {
        let mut status = 0;
        if self.user >= 0 {
            unsafe {
                sceKernelStopUnloadModule(
                    self.user,
                    0,
                    ptr::null_mut(),
                    0,
                    ptr::null_mut(),
                    &mut status,
                );
            }
        }
        if self.kernel >= 0 {
            let mut args = TaiArgs {
                size: size_of::<TaiArgs>(),
                pid: 0,
                args: 0,
                argp: ptr::null_mut(),
                flags: 0,
            };
            unsafe {
                taiStopUnloadKernelModuleForUser(
                    self.kernel,
                    &mut args,
                    ptr::null_mut(),
                    &mut status,
                );
            }
        }
    }
}
struct Mount {
    point: [c_char; 16],
    active: bool,
}
impl Mount {
    fn close(&mut self) -> Result<()> {
        if self.active {
            let result = unsafe { sceAppMgrUmount(self.point.as_ptr()) };
            ensure!(result >= 0, "Unmount failed: {result:#010x}");
            self.active = false;
        }
        Ok(())
    }
}
impl Drop for Mount {
    fn drop(&mut self) {
        let _ = self.close();
    }
}

pub fn with_save<T>(game: &Game, work: impl FnOnce(&Path, Option<u64>) -> Result<T>) -> Result<T> {
    let mut modules = MOUNT_MODULES
        .lock()
        .map_err(|_| anyhow::anyhow!("Previous mount operation panicked; restart the app"))?;
    if modules.is_none() {
        *modules = Some(Modules::load()?);
    }
    let path = CString::new(game.path.to_str().context("Invalid save path")?)?;
    let mut mounted = Mount {
        point: [0; 16],
        active: false,
    };
    let key = [0u8; 16];
    let mut result = -1;
    for id in [0x6e, 0x12e, 0x12f, 0x3ed] {
        let mut args = MountArgs {
            id,
            title: c"VSAVE0001".as_ptr(),
            path: path.as_ptr(),
            desired: ptr::null(),
            key: key.as_ptr().cast(),
            mount: mounted.point.as_mut_ptr(),
        };
        result = unsafe { vitaSaveUserMountById(&mut args) };
        if result >= 0 {
            break;
        }
    }
    if result < 0 {
        result = unsafe {
            sceAppMgrGameDataMount(
                path.as_ptr(),
                ptr::null(),
                ptr::null(),
                mounted.point.as_mut_ptr(),
            )
        };
    }
    ensure!(result >= 0, "Mount {} failed: {result:#010x}", game.save_id);
    mounted.active = true;
    let bytes: Vec<u8> = mounted
        .point
        .iter()
        .take_while(|b| **b != 0)
        .map(|b| *b as u8)
        .collect();
    let point = std::str::from_utf8(&bytes)?;
    ensure!(
        !point.is_empty() && point.ends_with(':'),
        "Invalid mount point"
    );
    let mut account = 0u64;
    let code = unsafe {
        sceRegMgrGetKeyBin(
            c"/CONFIG/NP".as_ptr(),
            c"account_id".as_ptr(),
            (&mut account as *mut u64).cast(),
            8,
        )
    };
    ensure!(code >= 0, "Read account ID: {code:#010x}");
    // PFS mounting decrypts access through the original save directory, as in
    // vita-savemgr's copy_savedata_to_slot/copy_slot_to_savedata. The returned
    // mount name is only an unmount handle; names such as trophy_sys0: cannot
    // be parsed by newlib's device-name parser.
    let result = work(&game.path, Some(account));
    let unmount = mounted.close();
    match (result, unmount) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(e), Ok(())) => Err(e),
        (Ok(_), Err(e)) => Err(e),
        (Err(e), Err(close)) => Err(e.context(close.to_string())),
    }
}
