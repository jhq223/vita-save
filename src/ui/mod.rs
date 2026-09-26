mod pages;
use crate::{app::Model, text_edit::WebDavField};
use anyhow::Result;
use nivora_platform::{Frame, InputEvent, Key, Size, TextMeasurer};
use nivora_ui::{
    Dialog, Dropdown, Localizer, NavigationEvent, Navigator, UiError, VirtualList, WidgetId,
};
use std::time::Duration;
pub const VIEWPORT: Size = Size {
    width: 960.0,
    height: 544.0,
};
pub const APP_ICON: &str = "app-icon";
include!(concat!(env!("OUT_DIR"), "/locales.rs"));

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Refresh,
    OpenGame(usize),
    Backups(usize),
    Snapshot(usize, String),
    AskBackup(usize),
    Backup(usize),
    AskDelete(usize, String),
    DeleteSelected,
    Delete(usize, String),
    AskRestore(usize, String),
    Restore(usize, String),
    Recover(usize),
    Cloud(usize),
    Upload(usize, String),
    Download(usize, String),
    TestConnection,
    Cancel,
    Settings,
    SettingsTab(Tab),
    Language,
    Theme,
    SetLanguage(&'static str),
    SetTheme(bool),
    Animations,
    BackupBeforeRestore,
    EditWebDav(WebDavField),
    Back,
    PageUp,
    PageDown,
    AskExit,
    Exit,
    Close,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Interface,
    Backup,
    WebDav,
    About,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Page {
    Library,
    Game(usize),
    Backups(usize),
    Snapshot(usize, String),
    Cloud(usize),
    Settings(Tab),
    Edit(WebDavField),
}
struct PageState {
    page: Page,
    list: Option<VirtualList>,
    selection: usize,
    focused_action: Option<Action>,
    restore: bool,
    editor: Option<WidgetId>,
    controls: Vec<(Action, WidgetId)>,
}
enum PendingDialog {
    Progress,
    Result(String),
}
pub struct View {
    navigator: Navigator<Action>,
    pages: Vec<PageState>,
    progress: Option<(pages::ProgressContent, WidgetId)>,
    pending_dialog: Option<PendingDialog>,
}
impl View {
    pub fn new(model: &Model) -> Result<Self> {
        let (ui, state) = pages::build(model, Page::Library)?;
        let mut navigator = Navigator::new(ui);
        navigator.set_animations_enabled(model.config.animations);
        Ok(Self {
            navigator,
            pages: vec![state],
            progress: None,
            pending_dialog: None,
        })
    }
    pub fn text(&self, model: &Model, key: &str) -> String {
        localizer(model).text(key)
    }
    pub fn page(&self) -> &Page {
        &self.pages.last().unwrap().page
    }
    pub fn selected_backup(&self, model: &Model) -> Option<(usize, String)> {
        let state = self.pages.last()?;
        let Page::Backups(index) = state.page else {
            return None;
        };
        let backup = model.detail(index)?.backups.get(state.selection)?;
        Some((index, backup.id.clone()))
    }
    pub fn home(&mut self, model: &Model) -> Result<()> {
        *self = Self::new(model)?;
        Ok(())
    }
    pub fn open(&mut self, model: &Model, page: Page) -> Result<()> {
        // Dismiss confirmation/dropdown layers before changing the screen stack.
        while self.navigator.depth() > self.pages.len() {
            self.navigator.pop();
        }
        if let Some(index) = self.pages.iter().position(|p| p.page == page) {
            while self.pages.len() > index + 1 {
                self.pages.pop();
                self.navigator.pop();
            }
            return self.rebuild(model);
        }
        let (ui, state) = pages::build(model, page)?;
        self.navigator.push_screen_animated(ui);
        self.pages.push(state);
        Ok(())
    }
    pub fn replace(&mut self, model: &Model, page: Page) -> Result<()> {
        self.pages.last_mut().unwrap().page = page;
        self.rebuild(model)
    }
    pub fn back(&mut self, model: &Model) -> Result<()> {
        if self.pages.len() > 1 {
            self.pages.pop();
            self.navigator.pop();
            self.rebuild(model)?;
        } else {
            self.confirm(model, "exit", Action::Exit)?;
        }
        Ok(())
    }
    pub fn rebuild(&mut self, model: &Model) -> Result<()> {
        self.progress = None;
        self.pending_dialog = None;
        while self.navigator.depth() > self.pages.len() {
            self.navigator.pop();
        }
        let previous = self.pages.last().unwrap();
        let selection = previous.selection;
        let focused = previous
            .focused_action
            .clone()
            .or_else(|| self.navigator.active().focused_action());
        let visible = self.navigator.active().focus_visible();
        let (mut ui, mut state) = pages::build(model, previous.page.clone())?;
        let mut control_focused = false;
        if let Some((_, id)) = state
            .controls
            .iter()
            .find(|(a, _)| Some(a) == focused.as_ref())
        {
            ui.focus(*id)?;
            control_focused = true;
        }
        ui.set_focus_visible(visible);
        state.selection = selection;
        state.focused_action = focused;
        state.restore = !control_focused;
        *self.pages.last_mut().unwrap() = state;
        self.navigator.replace(ui);
        self.navigator
            .set_animations_enabled(model.config.animations);
        Ok(())
    }
    pub fn layout(&mut self, model: &Model, measurer: &impl TextMeasurer) -> Result<(), UiError> {
        self.navigator.layout(VIEWPORT, measurer)?;
        if self.navigator.is_modal_active() {
            return Ok(());
        }
        let state = self.pages.last_mut().unwrap();
        if let Some(list) = &mut state.list {
            let count = pages::row_count(model, &state.page);
            list.update(self.navigator.active_mut(), VIEWPORT, measurer, |i| {
                pages::row(model, &state.page, i)
            })?;
            if state.restore && count > 0 {
                state.selection = state.selection.min(count - 1);
                let ui = self.navigator.active_mut();
                let visible = ui.focus_visible();
                list.scroll_to_index(ui, state.selection, VIEWPORT, measurer, |i| {
                    pages::row(model, &state.page, i)
                })?;
                if let Some(id) = list.widget_for(state.selection) {
                    ui.focus(id)?;
                    ui.set_focus_visible(visible);
                }
                state.restore = false;
            }
        }
        // Mount the underlying virtual rows before covering the page with a dialog.
        self.navigator.layout(VIEWPORT, measurer)?;
        if let Some(dialog) = self.pending_dialog.take() {
            match dialog {
                PendingDialog::Progress => self.show_progress(model)?,
                PendingDialog::Result(message) => self.navigator.push_dialog(
                    pages::theme(model),
                    Dialog::new(message).button(self.text(model, "ok"), Action::Close),
                )?,
            }
            self.navigator.layout(VIEWPORT, measurer)?;
        }
        Ok(())
    }
    pub fn handle(
        &mut self,
        model: &Model,
        event: InputEvent,
        measurer: &impl TextMeasurer,
    ) -> Result<Option<Action>, UiError> {
        if self.pending_dialog.is_some() || !self.navigator.accepts_input() {
            return Ok(None);
        }
        if self.progress.is_some() {
            // The task owns this dialog's lifetime. Its standard footer requests
            // cancellation, but must stay visible until the worker has cleaned up.
            if model.cancelling {
                return Ok(None);
            }
            if event == InputEvent::KeyDown(Key::Back) {
                return Ok(Some(Action::Cancel));
            }
            if let InputEvent::PointerDown(point) | InputEvent::PointerUp(point) = event
                && self.navigator.active().pointer_target_at(point).is_none()
            {
                self.navigator.active_mut().cancel_pointer();
                return Ok(None);
            }
            return Ok(self.navigator.active_mut().handle(event));
        }
        if !self.navigator.is_modal_active() {
            if matches!(event, InputEvent::KeyDown(Key::Back)) {
                return Ok(Some(if self.pages.len() == 1 {
                    Action::AskExit
                } else {
                    Action::Back
                }));
            }
            if let InputEvent::KeyDown(key @ (Key::Up | Key::Down)) = event {
                let state = self.pages.last_mut().unwrap();
                if key == Key::Up
                    && state.selection == 0
                    && matches!(state.page, Page::Backups(_))
                    && let Some(list) = &state.list
                    && self.navigator.active().focused() == list.widget_for(0)
                    && let Some((_, id)) = state.controls.first()
                {
                    self.navigator.active_mut().focus(*id)?;
                    self.navigator.active_mut().set_focus_visible(true);
                    self.track_selection();
                    return Ok(None);
                }
                if let Some(list) = &mut state.list
                    && list.move_focus(
                        self.navigator.active_mut(),
                        key,
                        VIEWPORT,
                        measurer,
                        |i| pages::row(model, &state.page, i),
                    )?
                {
                    self.navigator.active_mut().set_focus_visible(true);
                    self.track_selection();
                    return Ok(None);
                }
            }
        }
        let event = self.navigator.handle(event);
        self.track_selection();
        Ok(match event {
            Some(NavigationEvent::Action(a)) => Some(a),
            Some(NavigationEvent::BackAtRoot) => Some(Action::AskExit),
            _ => {
                if !self.navigator.is_modal_active()
                    && let Page::Settings(current) = self.page()
                    && let Some(Action::SettingsTab(tab)) = self.navigator.active().focused_action()
                    && tab != *current
                {
                    Some(Action::SettingsTab(tab))
                } else {
                    None
                }
            }
        })
    }
    fn track_selection(&mut self) {
        if self.navigator.is_modal_active() {
            return;
        }
        let state = self.pages.last_mut().unwrap();
        state.focused_action = self.navigator.active().focused_action();
        if let Some(list) = &state.list
            && let Some(focused) = self.navigator.active().focused()
            && let Some(index) = list
                .mounted_range()
                .find(|i| list.widget_for(*i) == Some(focused))
        {
            state.selection = index;
        }
    }
    pub fn shortcut_allowed(&self) -> bool {
        self.pending_dialog.is_none()
            && !self.navigator.is_modal_active()
            && self.navigator.accepts_input()
    }
    pub fn page_step(&mut self, amount: isize) {
        let state = self.pages.last_mut().unwrap();
        if state.list.is_none() {
            return;
        }
        self.navigator.active_mut().set_focus_visible(true);
        state.selection = state.selection.saturating_add_signed(amount);
        state.restore = true;
    }
    pub fn frame(&self) -> Frame {
        self.navigator.frame()
    }
    pub fn advance(&mut self, d: Duration) -> bool {
        self.navigator.advance(d)
    }
    pub fn cancel_pointer(&mut self) {
        self.navigator.active_mut().cancel_pointer();
    }
    pub fn confirm(&mut self, model: &Model, key: &str, yes: Action) -> Result<(), UiError> {
        self.confirm_message(model, self.text(model, key), yes)
    }
    pub fn confirm_message(
        &mut self,
        model: &Model,
        message: String,
        yes: Action,
    ) -> Result<(), UiError> {
        let l = localizer(model);
        self.navigator.push_dialog(
            pages::theme(model),
            Dialog::new(message)
                .button(l.text("no"), Action::Close)
                .button(l.text("yes"), yes)
                .default_button(0),
        )
    }
    pub fn dismiss_dialog(&mut self) {
        if self.navigator.depth() > self.pages.len() {
            self.navigator.pop();
        }
        self.progress = None;
        self.pending_dialog = None;
    }
    pub fn result(&mut self, model: &Model) -> Result<(), UiError> {
        self.pending_dialog = Some(PendingDialog::Result(model.message.clone()));
        Ok(())
    }
    pub fn start_progress(&mut self, model: &Model) -> Result<(), UiError> {
        let _ = model;
        self.pending_dialog = Some(PendingDialog::Progress);
        Ok(())
    }
    fn show_progress(&mut self, model: &Model) -> Result<(), UiError> {
        use std::{cell::Cell, rc::Rc};
        let widgets = Rc::new(Cell::new(None));
        let capture = widgets.clone();
        let t = pages::theme(model);
        self.navigator.push_dialog(
            t,
            Dialog::new("")
                .content(move |ui, parent| {
                    capture.set(Some(pages::progress(ui, parent, t)?));
                    Ok(())
                })
                .button(self.text(model, "cancel"), Action::Cancel)
                .cancelable(false),
        )?;
        self.progress = widgets.get().zip(self.navigator.active().focused());
        self.update_progress(model)
    }
    pub fn error(&mut self, model: &Model, error: &str) -> Result<(), UiError> {
        self.navigator.push_dialog(
            pages::theme(model),
            Dialog::new(format!("{}\n{error}", self.text(model, "error")))
                .button(self.text(model, "ok"), Action::Close),
        )
    }
    pub fn dropdown(&mut self, model: &Model, action: Action) -> Result<(), UiError> {
        let l = localizer(model);
        let menu = if action == Action::Language {
            Dropdown::new()
                .option("简体中文", Action::SetLanguage("zh"))
                .option("English", Action::SetLanguage("en"))
                .selected(usize::from(model.config.language == "en"))
        } else {
            Dropdown::new()
                .option(l.text("dark"), Action::SetTheme(false))
                .option(l.text("light"), Action::SetTheme(true))
                .selected(usize::from(model.config.light_theme))
        };
        if let Some(id) = self.navigator.active().focused() {
            self.navigator
                .push_dropdown_at(pages::theme(model), id, menu)
        } else {
            self.navigator.push_dropdown(pages::theme(model), menu)
        }
    }
    pub fn update_editor(&mut self, model: &Model) -> Result<(), UiError> {
        if let (Some(id), Some(edit)) = (self.pages.last().unwrap().editor, &model.edit) {
            self.navigator
                .active_mut()
                .replace_leaf(id, pages::editor(pages::theme(model), edit))?;
        }
        Ok(())
    }
    pub fn ime_request(&self) -> Option<nivora_platform::ImeRequest> {
        self.navigator.active().ime_request()
    }
    pub fn update_progress(&mut self, model: &Model) -> Result<(), UiError> {
        if let Some((content, cancel)) = self.progress {
            let p = &model.progress;
            let phase = localizer(model).text(if model.cancelling {
                "cancelled"
            } else if p.phase.is_empty() {
                "loading"
            } else {
                p.phase
            });
            let ui = self.navigator.active_mut();
            ui.set_text(content.phase, phase)?;
            ui.set_text(content.file, &p.file)?;
            ui.set_visible(content.file, !p.file.is_empty())?;
            ui.set_visible(content.spinner, p.total == 0)?;
            ui.set_visible(content.bar, p.total > 0)?;
            ui.set_visible(content.amount, p.total > 0)?;
            if p.total > 0 {
                let fraction = (p.done as f64 / p.total as f64).clamp(0.0, 1.0);
                ui.set_progress_value(content.bar, fraction as f32)?;
                ui.set_text(
                    content.amount,
                    format!(
                        "{:.0}% · {} / {}",
                        fraction * 100.0,
                        bytes(p.done),
                        bytes(p.total)
                    ),
                )?;
            }
            ui.set_enabled(cancel, !model.cancelling)?;
        }
        Ok(())
    }
}
fn localizer(model: &Model) -> Localizer {
    let mut l = Localizer::with_static_messages(&model.config.language, MESSAGES);
    l.set_fallback("en");
    l
}
pub fn bytes(n: u64) -> String {
    if n >= 1024 * 1024 * 1024 {
        format!("{:.1} GiB", n as f64 / (1024. * 1024. * 1024.))
    } else if n >= 1024 * 1024 {
        format!("{:.1} MiB", n as f64 / (1024. * 1024.))
    } else if n >= 1024 {
        format!("{:.1} KiB", n as f64 / 1024.)
    } else {
        format!("{n} B")
    }
}
