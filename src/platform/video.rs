use super::{
    Environment,
    ime::{Event as ImeEvent, Keyboard},
    input::{VitaInput, VitaInputEvent},
    pvr::PvrContext,
};
use crate::{app::App, ui::APP_ICON};
use anyhow::{Context, Result};
use glow::HasContext;
use nivora_platform::{DrawCommand, RgbaImage, TextMeasurer};
use nivora_render_gles2::{Gles2Options, Gles2Renderer, Gles2Target};
use std::{
    collections::HashSet,
    time::{Duration, Instant},
};

// These symbols must survive compilation AND cargo-vita's ELF packaging step.
// SceLibc serves the PVR modules; Rust/newlib uses a separate heap.
#[used]
#[unsafe(no_mangle)]
pub static sceUserMainThreadStackSize: u32 = 2 * 1024 * 1024;
#[used]
#[unsafe(no_mangle)]
pub static sceLibcHeapSize: u32 = 16 * 1024 * 1024;
#[used]
#[unsafe(no_mangle)]
pub static _newlib_heap_size_user: u32 = 128 * 1024 * 1024;

pub fn run() -> Result<()> {
    // Drop UI and renderer before the EGL surface/context.
    let pvr = PvrContext::new().map_err(anyhow::Error::msg)?;
    let gl = pvr.glow();
    let target = unsafe { Gles2Target::new(&gl) };
    let mut renderer = Renderer::new(&target)?;
    let mut app = App::new(Environment::vita())?;
    let mut input = VitaInput::new();
    let mut keyboard: Option<Keyboard> = None;
    let mut previous = Instant::now();
    let mut dirty = true;
    while !app.exit {
        let now = Instant::now();
        dirty |= app.view.advance(now.saturating_duration_since(previous));
        previous = now;
        match app.poll() {
            Ok(changed) => dirty |= changed,
            Err(e) => {
                app.error(e)?;
                dirty = true;
            }
        }
        app.view.layout(&app.model, renderer.measurer())?;
        let ime_owned_frame = keyboard.is_some() || app.model.edit.is_some();
        if keyboard.is_none()
            && let Some(edit) = &app.model.edit
        {
            input.reset();
            app.view.cancel_pointer();
            match Keyboard::open(edit) {
                Ok(ime) => keyboard = Some(ime),
                Err(error) => {
                    app.finish_edit(None)?;
                    app.error(error)?;
                }
            }
            dirty = true;
        }
        if let Some(ime) = &mut keyboard {
            let event = ime.poll().and_then(|event| {
                if !matches!(event, Some(ImeEvent::Finished(_))) {
                    ime.anchor(app.view.ime_request())?;
                }
                Ok(event)
            });
            match event {
                Ok(Some(ImeEvent::Changed(text))) => app.update_edit(text)?,
                Ok(Some(ImeEvent::Finished(value))) => {
                    keyboard.take();
                    if let Err(error) = app.finish_edit(value) {
                        app.error(error)?;
                    }
                    input.reset();
                }
                Err(error) => {
                    keyboard.take();
                    app.finish_edit(None)?;
                    app.error(error)?;
                    input.reset();
                }
                Ok(None) => {}
            }
            dirty = true;
        }
        if !ime_owned_frame {
            for event in input.poll(now) {
                dirty = true;
                let action = match event {
                    VitaInputEvent::Ui(event) => {
                        app.view.handle(&app.model, event, renderer.measurer())?
                    }
                    VitaInputEvent::Shortcut(action) => {
                        app.view.shortcut_allowed().then_some(action)
                    }
                    VitaInputEvent::CancelPointer => {
                        app.view.cancel_pointer();
                        None
                    }
                };
                if let Some(action) = action
                    && let Err(error) = app.dispatch(action)
                {
                    app.error(error)?;
                }
                app.view.layout(&app.model, renderer.measurer())?;
                if app.model.edit.is_some() {
                    app.view.cancel_pointer();
                    input.reset();
                    break;
                }
            }
        }
        if dirty {
            app.view.layout(&app.model, renderer.measurer())?;
            renderer.present(&app, &gl, &pvr)?;
            dirty = false;
        } else {
            std::thread::sleep(Duration::from_millis(8));
        }
        if app.model.busy || keyboard.is_some() {
            unsafe {
                vitasdk_sys::sceKernelPowerTick(0);
            }
        }
    }
    input.reset();
    Ok(())
}
struct Renderer<'gl> {
    renderer: Gles2Renderer<'gl>,
    images: HashSet<String>,
    fallback: image::RgbaImage,
}
impl<'gl> Renderer<'gl> {
    fn new(target: &'gl Gles2Target<'gl>) -> Result<Self> {
        let font = std::fs::read("app0:font/QiushuiShotai.ttf").context("Read bundled font")?;
        let mut renderer = Gles2Renderer::with_ab_glyph_font_and_options(
            target,
            font,
            Gles2Options {
                glyph_page_size: 512,
                cached_glyph_pages: 2,
            },
        )?;
        let fallback = image::load_from_memory(include_bytes!("../../runtime/sce_sys/icon0.png"))?
            .into_rgba8();
        upload(&mut renderer, APP_ICON, &fallback)?;
        Ok(Self {
            renderer,
            images: HashSet::new(),
            fallback,
        })
    }
    fn measurer(&self) -> &impl TextMeasurer {
        &self.renderer
    }
    fn present(&mut self, app: &App, gl: &glow::Context, pvr: &PvrContext) -> Result<()> {
        let frame = app.view.frame();
        let needed: HashSet<_> = frame
            .commands
            .iter()
            .filter_map(|c| match c {
                DrawCommand::Image { asset, .. } if asset != APP_ICON => Some(asset.clone()),
                _ => None,
            })
            .collect();
        self.images.retain(|asset| {
            if needed.contains(asset) {
                true
            } else {
                self.renderer.remove_image(asset);
                false
            }
        });
        for asset in needed {
            if self.images.contains(&asset) {
                continue;
            }
            let decoded = asset
                .strip_prefix("game:")
                .and_then(|s| s.rsplit_once(':'))
                .and_then(|(_, index)| index.parse::<usize>().ok())
                .and_then(|i| app.model.games.get(i))
                .and_then(|g| g.icon.as_ref())
                .and_then(|p| image::ImageReader::open(p).ok())
                .and_then(|mut r| {
                    let mut limits = image::Limits::default();
                    limits.max_alloc = Some(8 * 1024 * 1024);
                    limits.max_image_width = Some(1024);
                    limits.max_image_height = Some(1024);
                    r.limits(limits);
                    r.decode().ok()
                })
                .map(|i| i.thumbnail(220, 220).into_rgba8());
            upload(
                &mut self.renderer,
                &asset,
                decoded.as_ref().unwrap_or(&self.fallback),
            )?;
            self.images.insert(asset);
        }
        unsafe {
            gl.bind_framebuffer(glow::FRAMEBUFFER, None);
            gl.color_mask(true, true, true, true);
        }
        self.renderer.render(&frame)?;
        pvr.swap().map_err(anyhow::Error::msg)
    }
}
fn upload(renderer: &mut Gles2Renderer<'_>, asset: &str, rgba: &image::RgbaImage) -> Result<()> {
    renderer.upload_image(
        asset,
        RgbaImage::new(rgba.width(), rgba.height(), rgba.as_raw())?,
    )?;
    Ok(())
}
