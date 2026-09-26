//! SQLite only reads deserialized app.db snapshots; no filesystem VFS is exposed.
use rusqlite::ffi;
use std::ffi::{c_char, c_int};
static mut VFS: ffi::sqlite3_vfs = unsafe { std::mem::zeroed() };

#[unsafe(no_mangle)]
unsafe extern "C" fn sqlite3_os_init() -> c_int {
    // SQLite invokes this once during initialization, before creating connections.
    unsafe {
        let vfs = &raw mut VFS;
        (*vfs).iVersion = 1;
        (*vfs).szOsFile = size_of::<ffi::sqlite3_file>() as c_int;
        (*vfs).mxPathname = 512;
        (*vfs).zName = c"vita-memory".as_ptr();
        (*vfs).xOpen = Some(open);
        (*vfs).xDelete = Some(delete);
        (*vfs).xAccess = Some(access);
        (*vfs).xFullPathname = Some(full_path);
        (*vfs).xRandomness = Some(random);
        (*vfs).xSleep = Some(sleep);
        (*vfs).xCurrentTime = Some(time);
        ffi::sqlite3_vfs_register(vfs, 1)
    }
}
#[unsafe(no_mangle)]
extern "C" fn sqlite3_os_end() -> c_int {
    ffi::SQLITE_OK
}
unsafe extern "C" fn open(
    _: *mut ffi::sqlite3_vfs,
    _: *const c_char,
    _: *mut ffi::sqlite3_file,
    _: c_int,
    _: *mut c_int,
) -> c_int {
    ffi::SQLITE_CANTOPEN
}
unsafe extern "C" fn delete(_: *mut ffi::sqlite3_vfs, _: *const c_char, _: c_int) -> c_int {
    ffi::SQLITE_READONLY
}
unsafe extern "C" fn access(
    _: *mut ffi::sqlite3_vfs,
    _: *const c_char,
    _: c_int,
    out: *mut c_int,
) -> c_int {
    unsafe {
        *out = 0;
    }
    ffi::SQLITE_OK
}
unsafe extern "C" fn full_path(
    _: *mut ffi::sqlite3_vfs,
    _: *const c_char,
    _: c_int,
    _: *mut c_char,
) -> c_int {
    ffi::SQLITE_CANTOPEN
}
unsafe extern "C" fn random(_: *mut ffi::sqlite3_vfs, n: c_int, out: *mut c_char) -> c_int {
    if n <= 0 {
        return 0;
    }
    let bytes = unsafe { std::slice::from_raw_parts_mut(out.cast(), n as usize) };
    if getrandom::getrandom(bytes).is_ok() {
        n
    } else {
        0
    }
}
unsafe extern "C" fn sleep(_: *mut ffi::sqlite3_vfs, micros: c_int) -> c_int {
    std::thread::sleep(std::time::Duration::from_micros(micros.max(0) as u64));
    micros.max(0)
}
unsafe extern "C" fn time(_: *mut ffi::sqlite3_vfs, out: *mut f64) -> c_int {
    unsafe {
        *out = 2440587.5 + crate::backup::timestamp() as f64 / 86400.0;
    }
    ffi::SQLITE_OK
}

static DATABASE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
// Connection drops before its lock guard. Neither this wrapper nor its connection
// escapes app_database(), so the single-thread SQLite build is never used concurrently.
pub(crate) struct Database {
    connection: rusqlite::Connection,
    _guard: std::sync::MutexGuard<'static, ()>,
}
impl std::ops::Deref for Database {
    type Target = rusqlite::Connection;
    fn deref(&self) -> &Self::Target {
        &self.connection
    }
}
impl std::ops::DerefMut for Database {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.connection
    }
}
pub(crate) fn memory() -> anyhow::Result<Database> {
    let guard = DATABASE_LOCK
        .lock()
        .map_err(|_| anyhow::anyhow!("Database worker failed; restart the app"))?;
    let mut handle = std::ptr::null_mut();
    let code = unsafe {
        ffi::sqlite3_open_v2(
            c":memory:".as_ptr(),
            &mut handle,
            ffi::SQLITE_OPEN_READWRITE | ffi::SQLITE_OPEN_CREATE | ffi::SQLITE_OPEN_NOMUTEX,
            std::ptr::null(),
        )
    };
    if code != ffi::SQLITE_OK {
        if !handle.is_null() {
            unsafe {
                ffi::sqlite3_close(handle);
            }
        }
        anyhow::bail!("Open metadata snapshot: SQLite {code}");
    }
    // The handle is newly owned and protected for its entire lifetime, including destruction.
    let connection = unsafe { rusqlite::Connection::from_handle_owned(handle) }?;
    Ok(Database {
        connection,
        _guard: guard,
    })
}
