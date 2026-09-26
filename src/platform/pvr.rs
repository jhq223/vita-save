use std::ffi::{CStr, CString};
use std::ptr;

use pvr_psp2_sys::gles::*;
use vitasdk_sys::*;

pub struct PvrContext {
    display: EGLDisplay,
    surface: EGLSurface,
    context: EGLContext,
}

impl PvrContext {
    pub fn new() -> Result<Self, String> {
        let _ = load_module(c"vs0:sys/external/libfios2.suprx", false);
        let _ = load_module(c"vs0:sys/external/libc.suprx", false);
        load_module(c"app0:module/libgpu_es4_ext.suprx", true)?;
        load_module(c"app0:module/libIMGEGL.suprx", true)?;
        let mut hint = PvrAppHint::default();
        if unsafe { PVRSRVInitializeAppHint(&mut hint) } == 0 {
            return Err("PVR app-hint initialization failed".to_owned());
        }
        hint.sw_tex_op_cleanup_delay = 16_000;
        hint.enable_memory_speed_test = 0;
        if unsafe { PVRSRVCreateVirtualAppHint(&mut hint) } == 0 {
            return Err("PVR virtual app-hint creation failed".to_owned());
        }
        let mut pvr = Self {
            display: ptr::null_mut(),
            surface: ptr::null_mut(),
            context: ptr::null_mut(),
        };
        pvr.initialize()?;
        Ok(pvr)
    }

    fn initialize(&mut self) -> Result<(), String> {
        self.display = unsafe { eglGetDisplay(0) };
        if self.display.is_null() {
            return Err(egl_error("eglGetDisplay"));
        }
        let mut major = 0;
        let mut minor = 0;
        egl_result("eglInitialize", unsafe {
            eglInitialize(self.display, &mut major, &mut minor)
        })?;
        egl_result("eglBindAPI", unsafe { eglBindAPI(EGL_OPENGL_ES_API) })?;

        let attributes = [
            EGL_BUFFER_SIZE,
            EGL_DONT_CARE,
            EGL_DEPTH_SIZE,
            0,
            EGL_RED_SIZE,
            8,
            EGL_GREEN_SIZE,
            8,
            EGL_BLUE_SIZE,
            8,
            EGL_ALPHA_SIZE,
            8,
            EGL_STENCIL_SIZE,
            0,
            EGL_SURFACE_TYPE,
            EGL_WINDOW_BIT | EGL_PBUFFER_BIT,
            EGL_RENDERABLE_TYPE,
            EGL_OPENGL_ES2_BIT,
            EGL_NONE,
        ];
        let mut config = ptr::null_mut();
        let mut count = 0;
        egl_result("eglChooseConfig", unsafe {
            eglChooseConfig(
                self.display,
                attributes.as_ptr(),
                &mut config,
                1,
                &mut count,
            )
        })?;
        if count == 0 || config.is_null() {
            return Err("eglChooseConfig returned no configs".to_owned());
        }

        self.surface =
            unsafe { eglCreateWindowSurface(self.display, config, ptr::null_mut(), ptr::null()) };
        if self.surface.is_null() {
            return Err(egl_error("eglCreateWindowSurface"));
        }
        let context_attributes = [EGL_CONTEXT_CLIENT_VERSION, 2, EGL_NONE];
        self.context = unsafe {
            eglCreateContext(
                self.display,
                config,
                ptr::null_mut(),
                context_attributes.as_ptr(),
            )
        };
        if self.context.is_null() {
            return Err(egl_error("eglCreateContext"));
        }
        egl_result("eglMakeCurrent", unsafe {
            eglMakeCurrent(self.display, self.surface, self.surface, self.context)
        })?;
        egl_result("eglSwapInterval", unsafe {
            eglSwapInterval(self.display, 1)
        })?;
        Ok(())
    }

    pub fn glow(&self) -> glow::Context {
        unsafe {
            glow::Context::from_loader_function(|name| {
                let name = CString::new(name).expect("OpenGL symbol contains NUL");
                eglGetProcAddress(name.as_ptr())
            })
        }
    }

    pub fn swap(&self) -> Result<(), String> {
        egl_result("eglSwapBuffers", unsafe {
            eglSwapBuffers(self.display, self.surface)
        })
    }
}

impl Drop for PvrContext {
    fn drop(&mut self) {
        unsafe {
            if !self.context.is_null() {
                let _ = eglMakeCurrent(
                    self.display,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    ptr::null_mut(),
                );
                let _ = eglDestroyContext(self.display, self.context);
            }
            if !self.surface.is_null() {
                let _ = eglDestroySurface(self.display, self.surface);
            }
            if !self.display.is_null() {
                let _ = eglTerminate(self.display);
            }
        }
    }
}

fn load_module(path: &CStr, required: bool) -> Result<i32, String> {
    let mut status = 0;
    let module = unsafe {
        sceKernelLoadStartModule(
            path.as_ptr(),
            0,
            ptr::null_mut(),
            0,
            ptr::null_mut(),
            &mut status,
        )
    };
    if module < 0 || status < 0 {
        let error = format!(
            "failed to load {}: uid={module:#010x} status={status:#010x}",
            path.to_string_lossy()
        );
        if required { Err(error) } else { Ok(module) }
    } else {
        Ok(module)
    }
}

fn egl_result(operation: &str, result: EGLBoolean) -> Result<(), String> {
    if result == EGL_TRUE {
        Ok(())
    } else {
        Err(egl_error(operation))
    }
}

fn egl_error(operation: &str) -> String {
    let error = unsafe { eglGetError() };
    format!("{operation} failed: EGL error {error:#06x}")
}
