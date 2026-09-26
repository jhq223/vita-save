//! Native IME overlay used alongside PVR (no GXM common-dialog render target).
use crate::text_edit::{Edit, EditText, WebDavField};
use anyhow::{Result, ensure};
use std::{cell::UnsafeCell, ffi::c_void, marker::PhantomData, rc::Rc};
use vitasdk_sys::*;

const TEXT_CAPACITY: usize = (SCE_IME_MAX_TEXT_LENGTH + SCE_IME_MAX_PREEDIT_LENGTH + 1) as usize;
const _: () = assert!(size_of::<SceImeParam>() == 0x40);

struct Callback {
    text: [u16; TEXT_CAPACITY],
    length: usize,
    caret: usize,
    preedit_start: usize,
    preedit_length: usize,
    changed: bool,
    finished: Option<bool>,
    invalid: bool,
}
struct Buffers {
    work: UnsafeCell<[u64; SCE_IME_WORK_BUFFER_SIZE as usize / 8]>,
    initial: UnsafeCell<[u16; TEXT_CAPACITY]>,
    input: UnsafeCell<[u16; TEXT_CAPACITY]>,
    callback: UnsafeCell<Callback>,
}
pub(super) enum Event {
    Changed(EditText),
    Finished(Option<String>),
}
pub(super) struct Keyboard {
    buffers: Option<Box<Buffers>>,
    open: bool,
    loaded: bool,
    anchor: Option<(u32, u32, u32)>,
    _main_thread: PhantomData<Rc<()>>,
}
impl Keyboard {
    pub fn open(edit: &Edit) -> Result<Self> {
        let mut text = [0; TEXT_CAPACITY];
        let initial: Vec<u16> = edit.text.value.encode_utf16().collect();
        ensure!(
            initial.len() <= edit.field.limit(),
            "Input exceeds keyboard capacity"
        );
        text[..initial.len()].copy_from_slice(&initial);
        let buffers = Box::new(Buffers {
            work: UnsafeCell::new([0; SCE_IME_WORK_BUFFER_SIZE as usize / 8]),
            initial: UnsafeCell::new(text),
            input: UnsafeCell::new(text),
            callback: UnsafeCell::new(Callback {
                text,
                length: initial.len(),
                caret: initial.len(),
                preedit_start: 0,
                preedit_length: 0,
                changed: false,
                finished: None,
                invalid: false,
            }),
        });
        let mut keyboard = Self {
            buffers: Some(buffers),
            open: false,
            loaded: false,
            anchor: None,
            _main_thread: PhantomData,
        };
        if unsafe { sceSysmoduleIsLoaded(SCE_SYSMODULE_IME) } != 0 {
            check("sceSysmoduleLoadModule(IME)", unsafe {
                sceSysmoduleLoadModule(SCE_SYSMODULE_IME)
            })?;
            keyboard.loaded = true;
        }
        let buffers = keyboard.buffers.as_ref().unwrap();
        let mut param: SceImeParam = unsafe { std::mem::zeroed() };
        param.sdkVersion = PSP2_SDK_VERSION;
        param.supportedLanguages = 0; // Use the system's enabled keyboard languages.
        param.type_ = if edit.field == WebDavField::Url {
            // URL input uses Latin characters without a composition-confirm step.
            SCE_IME_TYPE_BASIC_LATIN
        } else {
            SCE_IME_TYPE_DEFAULT
        };
        param.enterLabel = SCE_IME_ENTER_LABEL_GO as u8;
        param.option = SCE_IME_OPTION_NO_ASSISTANCE | SCE_IME_OPTION_NO_AUTO_CAPITALIZATION;
        param.work = buffers.work.get().cast();
        param.arg = buffers.callback.get().cast();
        param.handler = Some(on_event);
        param.initialText = buffers.initial.get().cast();
        param.inputTextBuffer = buffers.input.get().cast();
        param.maxTextLength = edit.field.limit() as u32;
        check("sceImeOpen", unsafe { sceImeOpen(&param) })?;
        keyboard.open = true;
        Ok(keyboard)
    }
    pub fn poll(&mut self) -> Result<Option<Event>> {
        // Callbacks execute synchronously during sceImeUpdate on this UI thread.
        // Do not borrow Callback across the FFI call.
        check("sceImeUpdate", unsafe { sceImeUpdate() })?;
        let state = unsafe { &mut *self.buffers.as_ref().unwrap().callback.get() };
        ensure!(!state.invalid, "Invalid text returned by system keyboard");
        if let Some(accepted) = state.finished.take() {
            let value = if accepted {
                // PRESS_ENTER submits the OS-owned committed buffer. The last
                // UPDATE_TEXT callback can still describe the previous preedit;
                // it is for display, not the authoritative submitted value.
                let input = unsafe { &*self.buffers.as_ref().unwrap().input.get() };
                Some(crate::text_edit::committed_utf16(input)?)
            } else {
                None
            };
            self.close()?;
            return Ok(Some(Event::Finished(value)));
        }
        if std::mem::take(&mut state.changed) {
            return Ok(Some(Event::Changed(EditText::from_utf16(
                &state.text[..state.length],
                state.caret,
                state.preedit_start..state.preedit_start.saturating_add(state.preedit_length),
            )?)));
        }
        Ok(None)
    }
    pub fn anchor(&mut self, request: Option<nivora_platform::ImeRequest>) -> Result<()> {
        if let Some(request) = request {
            let rect = request.cursor;
            let next = (
                rect.x.clamp(0.0, 959.0) as u32,
                rect.y.clamp(0.0, 220.0) as u32,
                rect.height.clamp(1.0, 64.0) as u32,
            );
            if self.anchor != Some(next) {
                let geometry = SceImePreeditGeometry {
                    x: next.0,
                    y: next.1,
                    height: next.2,
                };
                check("sceImeSetPreeditGeometry", unsafe {
                    sceImeSetPreeditGeometry(&geometry)
                })?;
                self.anchor = Some(next);
            }
        }
        Ok(())
    }
    fn close(&mut self) -> Result<()> {
        if self.open {
            let code = unsafe { sceImeClose() };
            // If the system already closed the IME, its buffers are no longer in use.
            if code != SCE_IME_ERROR_NOT_OPENED as i32 {
                check("sceImeClose", code)?;
            }
            self.open = false;
        }
        Ok(())
    }
}
impl Drop for Keyboard {
    fn drop(&mut self) {
        if self.close().is_err() {
            // Keep callback/work pointers valid if the OS refuses to release them.
            if let Some(buffers) = self.buffers.take() {
                Box::leak(buffers);
            }
            return;
        }
        if self.loaded {
            unsafe {
                sceSysmoduleUnloadModule(SCE_SYSMODULE_IME);
            }
        }
    }
}
fn check(operation: &str, code: i32) -> Result<()> {
    ensure!(code >= 0, "{operation}: {code:#010x}");
    Ok(())
}
unsafe extern "C" fn on_event(arg: *mut c_void, event: *const SceImeEventData) {
    if arg.is_null() || event.is_null() {
        return;
    }
    let state = unsafe { &mut *arg.cast::<Callback>() };
    let event = unsafe { &*event };
    match event.id {
        SCE_IME_EVENT_UPDATE_TEXT => {
            let edit = unsafe { event.param.text };
            if edit.str_.is_null() {
                state.invalid = true;
                return;
            }
            let mut length = 0;
            // The SDK owns the text pointer for the duration of this callback.
            // Copy now; never retain the event pointer or allocate in the callback.
            while length < TEXT_CAPACITY {
                let unit = unsafe { *edit.str_.add(length) };
                state.text[length] = unit;
                if unit == 0 {
                    break;
                }
                length += 1;
            }
            state.invalid |= length == TEXT_CAPACITY;
            state.length = length.min(TEXT_CAPACITY - 1);
            state.caret = edit.caretIndex as usize;
            state.preedit_start = edit.preeditIndex as usize;
            state.preedit_length = edit.preeditLength as usize;
            state.changed = true;
        }
        SCE_IME_EVENT_UPDATE_CARET => {
            state.caret = unsafe { event.param.caretIndex } as usize;
            state.changed = true;
        }
        SCE_IME_EVENT_PRESS_ENTER => state.finished = Some(true),
        SCE_IME_EVENT_PRESS_CLOSE if state.finished.is_none() => {
            state.finished = Some(false);
        }
        _ => {}
    }
}
