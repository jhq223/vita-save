use super::*;
use core::{
    arch::asm,
    mem::{transmute, zeroed},
    ptr,
};
const KERNEL_PID: i32 = 0x10005;
#[repr(C)]
struct TaiModuleInfo {
    size: usize,
    modid: i32,
    nid: u32,
    name: [c_char; 27],
    exports_start: usize,
    exports_end: usize,
    imports_start: usize,
    imports_end: usize,
}
#[repr(C)]
struct Segment {
    size: usize,
    perms: u32,
    address: *mut c_void,
    memory_size: usize,
    file_size: usize,
    reserved: u32,
}
#[repr(C)]
struct ModuleInfo {
    size: usize,
    modid: i32,
    attributes: u16,
    version: [u8; 2],
    name: [c_char; 28],
    unknown: u32,
    start: *mut c_void,
    stop: *mut c_void,
    exit: *mut c_void,
    exidx_top: *mut c_void,
    exidx_bottom: *mut c_void,
    extab_top: *mut c_void,
    extab_bottom: *mut c_void,
    tls: *mut c_void,
    tls_init_size: usize,
    tls_area_size: usize,
    path: [c_char; 256],
    segments: [Segment; 4],
    state: u32,
}
const _: () = assert!(size_of::<TaiModuleInfo>() == 56);
const _: () = assert!(size_of::<ModuleInfo>() == 0x1b8);
const _: () = assert!(core::mem::offset_of!(ModuleInfo, segments) == 0x154);
type FindProcess = unsafe extern "C" fn(*mut c_void, i32) -> *mut c_void;
type Mount = unsafe extern "C" fn(
    i32,
    *mut c_void,
    i32,
    *const c_char,
    *const c_char,
    *const c_char,
    *const c_void,
    *mut c_char,
) -> i32;
type GetModule = unsafe extern "C" fn(i32, i32, *mut ModuleInfo) -> i32;
unsafe extern "C" {
    fn taiGetModuleInfoForKernel(pid: i32, name: *const c_char, info: *mut TaiModuleInfo) -> i32;
    fn module_get_offset(
        pid: i32,
        module: i32,
        segment: i32,
        offset: usize,
        address: *mut usize,
    ) -> i32;
    fn module_get_export_func(
        pid: i32,
        name: *const c_char,
        library: u32,
        function: u32,
        address: *mut usize,
    ) -> i32;
    fn ksceKernelGetProcessId() -> i32;
    fn ksceKernelCopyFromUser(dst: *mut c_void, src: *const c_void, len: usize) -> i32;
    fn ksceKernelStrncpyFromUser(dst: *mut c_char, src: *const c_char, len: usize) -> i32;
    fn ksceKernelStrncpyToUser(dst: *mut c_char, src: *const c_char, len: usize) -> i32;
    fn ksceKernelRunWithStack(
        size: usize,
        work: unsafe extern "C" fn(*mut c_void) -> i32,
        args: *mut c_void,
    ) -> i32;
}

/// Firmware table shared with the original VitaShell mount procedure.
fn offsets(nid: u32) -> Option<(usize, usize)> {
    match nid {
        0x94cefe4b | 0xdfbc288c => Some((0x2de1, 0x19e15)),
        0xdbb29db7 => Some((0x2de1, 0x19b51)),
        0x1c9879d6 => Some((0x2de1, 0x19e61)),
        0x54e2e984 | 0xc3c538de => Some((0x2de1, 0x19e6d)),
        0x321e4852 | 0x700da0cd | 0xf7846b4e | 0xa8e80ba8 | 0xb299d195 | 0x30007bd3 => {
            Some((0x2de9, 0x19e95))
        }
        _ => None,
    }
}

unsafe extern "C" fn mount(raw: *mut c_void) -> i32 {
    // raw points to a kernel-local copy. User pointers are only read through checked kernel copy APIs.
    unsafe {
        let args = &*(raw as *const MountArgs);
        if ![0x6e, 0x12e, 0x12f, 0x3ed].contains(&args.id)
            || args.path.is_null()
            || args.mount_point.is_null()
        {
            return -1;
        }
        let mut app: TaiModuleInfo = zeroed();
        app.size = size_of::<TaiModuleInfo>();
        let result = taiGetModuleInfoForKernel(KERNEL_PID, c"SceAppMgr".as_ptr(), &mut app);
        if result < 0 {
            return result;
        }
        let Some((find_offset, mount_offset)) = offsets(app.nid) else {
            return -1;
        };
        let (mut find_address, mut mount_address) = (0, 0);
        if module_get_offset(KERNEL_PID, app.modid, 0, find_offset, &mut find_address) < 0
            || module_get_offset(KERNEL_PID, app.modid, 0, mount_offset, &mut mount_address) < 0
            || find_address == 0
            || mount_address == 0
        {
            return -1;
        }
        let mut info_address = 0;
        if module_get_export_func(
            KERNEL_PID,
            c"SceKernelModulemgr".as_ptr(),
            0xc445fa63,
            0xd269f915,
            &mut info_address,
        ) < 0
            && module_get_export_func(
                KERNEL_PID,
                c"SceKernelModulemgr".as_ptr(),
                0x92c9ffc2,
                0xdaa90093,
                &mut info_address,
            ) < 0
        {
            return -1;
        }
        if info_address == 0 {
            return -1;
        }
        let get_info: GetModule = transmute(info_address);
        let find: FindProcess = transmute(find_address);
        let mount: Mount = transmute(mount_address);
        let mut info: ModuleInfo = zeroed();
        info.size = size_of::<ModuleInfo>();
        let result = get_info(KERNEL_PID, app.modid, &mut info);
        if result < 0 {
            return result;
        }
        if info.segments[1].address.is_null() || info.segments[1].memory_size <= 0x500 {
            return -1;
        }
        let pid = ksceKernelGetProcessId();
        let process = find(info.segments[1].address.byte_add(0x500), pid);
        if process.is_null() {
            return -1;
        }
        let mut title = [0 as c_char; 12];
        let mut path = [0 as c_char; 256];
        let mut desired = [0 as c_char; 16];
        let mut point = [0 as c_char; 16];
        let mut key = [0u8; 16];
        if (!args.process_title_id.is_null()
            && ksceKernelStrncpyFromUser(title.as_mut_ptr(), args.process_title_id, 11) < 0)
            || ksceKernelStrncpyFromUser(path.as_mut_ptr(), args.path, 255) < 0
            || (!args.desired.is_null()
                && ksceKernelStrncpyFromUser(desired.as_mut_ptr(), args.desired, 15) < 0)
            || (!args.key.is_null()
                && ksceKernelCopyFromUser(key.as_mut_ptr().cast(), args.key, 16) < 0)
        {
            return -1;
        }
        let result = mount(
            pid,
            process.byte_add(0x580),
            args.id,
            if args.process_title_id.is_null() {
                ptr::null()
            } else {
                title.as_ptr()
            },
            path.as_ptr(),
            if args.desired.is_null() {
                ptr::null()
            } else {
                desired.as_ptr()
            },
            if args.key.is_null() {
                ptr::null()
            } else {
                key.as_ptr().cast()
            },
            point.as_mut_ptr(),
        );
        if result >= 0 {
            let copied = ksceKernelStrncpyToUser(args.mount_point, point.as_ptr(), 16);
            if copied < 0 {
                return copied;
            }
        }
        result
    }
}

/// # Safety
/// Called through the exported syscall gate; args is an untrusted user address.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vitaSaveKernelMountById(args: *mut MountArgs) -> i32 {
    unsafe {
        let state: u32;
        // Match VitaSDK ENTER_SYSCALL / EXIT_SYSCALL. Do not mark these asm blocks nomem.
        asm!("mrc p15, 0, {state}, c13, c0, 3",state=out(reg) state,options(nostack,preserves_flags));
        asm!("mcr p15, 0, {shifted}, c13, c0, 3",shifted=in(reg) state.wrapping_shl(16),options(nostack,preserves_flags));
        let mut local: MountArgs = zeroed();
        let mut result = ksceKernelCopyFromUser(
            (&mut local as *mut MountArgs).cast(),
            args.cast(),
            size_of::<MountArgs>(),
        );
        if result >= 0 {
            result = ksceKernelRunWithStack(0x2000, mount, (&mut local as *mut MountArgs).cast());
        }
        asm!("mcr p15, 0, {state}, c13, c0, 3",state=in(reg) state,options(nostack,preserves_flags));
        result
    }
}
