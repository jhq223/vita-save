use super::{APP_ICON, Action, Page, PageState, Tab, bytes, localizer};
use crate::{
    app::Model,
    backup::Recovery,
    text_edit::{Edit, WebDavField},
};
use nivora_ui::{
    AlignItems, ControllerButton as Button, ControllerLayout, Hint, HintSide,
    LayoutDimension as Dim, LayoutLengthPercentage as Length, LayoutLengthPercentageAuto as Auto,
    LayoutRect, LayoutSize, Localizer, ScreenFrame, SpinnerSize, TextFlow, Theme, Ui, UiError,
    VirtualList, WidgetId, WidgetSpec,
};

pub(super) fn theme(model: &Model) -> Theme {
    (if model.config.light_theme {
        Theme::light()
    } else {
        Theme::dark()
    })
    .with_animated_focus(model.config.animations)
}
fn flex_column() -> WidgetSpec<Action> {
    WidgetSpec::column().update_layout(|s| {
        s.flex_grow = 1.0;
        s.flex_basis = Dim::length(0.0);
        s.min_size.width = Auto::length(0.0);
        s.min_size.height = Auto::length(0.0);
        s.gap.height = Length::length(12.0);
    })
}
fn text(
    ui: &mut Ui<Action>,
    parent: WidgetId,
    t: Theme,
    value: impl Into<String>,
    size: f32,
) -> Result<WidgetId, UiError> {
    ui.insert(
        parent,
        t.label(value, size)
            .update_appearance(|a| a.text_flow = TextFlow::Wrap)
            .update_layout(|s| {
                s.flex_shrink = 0.0;
                s.min_size.width = Auto::length(0.0);
            }),
    )
}
fn control(
    ui: &mut Ui<Action>,
    parent: WidgetId,
    controls: &mut Vec<(Action, WidgetId)>,
    action: Action,
    spec: WidgetSpec<Action>,
    enabled: bool,
) -> Result<(), UiError> {
    let id = ui.insert(
        parent,
        spec.update_appearance(|a| a.font_size = 24.0)
            .update_layout(|s| {
                s.size.height = Dim::length(56.0);
                s.flex_shrink = 0.0;
            }),
    )?;
    ui.set_enabled(id, enabled)?;
    if enabled {
        controls.push((action, id));
    }
    Ok(())
}
#[allow(clippy::too_many_arguments)]
fn button(
    ui: &mut Ui<Action>,
    parent: WidgetId,
    controls: &mut Vec<(Action, WidgetId)>,
    t: Theme,
    l: &Localizer,
    key: &str,
    action: Action,
    enabled: bool,
) -> Result<(), UiError> {
    control(
        ui,
        parent,
        controls,
        action.clone(),
        t.button(l.text(key), action),
        enabled,
    )
}
fn image(ui: &mut Ui<Action>, parent: WidgetId, asset: String, size: f32) -> Result<(), UiError> {
    ui.insert(
        parent,
        WidgetSpec::image(asset).update_layout(|s| {
            s.size = LayoutSize {
                width: Dim::length(size),
                height: Dim::length(size),
            };
            s.flex_shrink = 0.0;
        }),
    )?;
    Ok(())
}
fn hint(
    frame: &mut ScreenFrame<Action>,
    l: &Localizer,
    button: Button,
    key: &str,
    action: Option<Action>,
    left: bool,
) -> Result<(), UiError> {
    let layout = if matches!(button, Button::LeftShoulder | Button::RightShoulder) {
        ControllerLayout::Switch
    } else {
        ControllerLayout::PlayStation
    };
    let mut hint = Hint::controller(
        layout,
        button,
        if key.is_empty() {
            String::new()
        } else {
            l.text(key)
        },
    )
    .side(if left {
        HintSide::Left
    } else {
        HintSide::Right
    });
    if let Some(action) = action {
        hint = hint.on_activate(action);
    }
    let id = frame.add_hint(hint)?;
    let mut appearance = frame.ui().appearance(id).unwrap();
    appearance.font_size = 18.0;
    frame.ui_mut().set_appearance(id, appearance)
}
pub(super) fn build(model: &Model, page: Page) -> Result<(Ui<Action>, PageState), UiError> {
    let t = theme(model);
    let l = localizer(model);
    let title = match &page {
        Page::Library => crate::APP_NAME.to_string(),
        Page::Game(i) => model.games[*i].name.clone(),
        Page::Backups(_) => l.text("backups"),
        Page::Snapshot(_, _) => l.text("backups"),
        Page::Cloud(_) => l.text("cloud"),
        Page::Settings(_) => l.text("settings"),
        Page::Edit(field) => l.text(field.key()),
    };
    let mut frame = ScreenFrame::new(t, title)?;
    frame.set_separator_inset(48.0)?;
    let status = frame.header_status();
    text(
        frame.ui_mut(),
        status,
        t,
        format!("v{}", crate::VERSION),
        18.0,
    )?;
    if page == Page::Library {
        let trailing = frame.title_trailing();
        if !model.scanning {
            text(
                frame.ui_mut(),
                trailing,
                t,
                l.format("count", &[("count", &model.games.len().to_string())]),
                18.0,
            )?;
        }
        hint(
            &mut frame,
            &l,
            Button::Select,
            "settings",
            Some(Action::Settings),
            true,
        )?;
        hint(
            &mut frame,
            &l,
            Button::Start,
            "refresh",
            Some(Action::Refresh),
            true,
        )?;
    }
    if matches!(page, Page::Library | Page::Backups(_) | Page::Cloud(_)) {
        hint(
            &mut frame,
            &l,
            Button::LeftShoulder,
            "",
            Some(Action::PageUp),
            false,
        )?;
        hint(
            &mut frame,
            &l,
            Button::RightShoulder,
            "page",
            Some(Action::PageDown),
            false,
        )?;
    }
    if matches!(page, Page::Backups(_)) && row_count(model, &page) > 0 && !model.busy {
        hint(
            &mut frame,
            &l,
            Button::FaceNorth,
            "delete",
            Some(Action::DeleteSelected),
            true,
        )?;
    }
    hint(
        &mut frame,
        &l,
        Button::FaceSouth,
        "back",
        Some(if page == Page::Library {
            Action::AskExit
        } else {
            Action::Back
        }),
        false,
    )?;
    hint(&mut frame, &l, Button::FaceEast, "select", None, false)?;
    let content = frame.content();
    let ui = frame.ui_mut();
    let mut style = nivora_ui::LayoutStyle {
        flex_grow: 1.0,
        flex_direction: nivora_ui::FlexDirection::Column,
        ..Default::default()
    };
    style.min_size = LayoutSize {
        width: Auto::length(0.0),
        height: Auto::length(0.0),
    };
    style.padding = LayoutRect {
        left: Length::length(64.0),
        right: Length::length(64.0),
        top: Length::length(16.0),
        bottom: Length::length(16.0),
    };
    style.gap = LayoutSize {
        width: Length::length(28.0),
        height: Length::length(12.0),
    };
    if matches!(page, Page::Game(_) | Page::Settings(_)) {
        style.flex_direction = nivora_ui::FlexDirection::Row;
    }
    ui.set_style(content, style)?;
    let mut state = PageState {
        page: page.clone(),
        list: None,
        selection: 0,
        focused_action: None,
        restore: true,
        editor: None,
        controls: Vec::new(),
    };
    match &page {
        Page::Library | Page::Backups(_) | Page::Cloud(_) => {
            if let Page::Backups(i) = page {
                let recovery = model.detail(i).and_then(|d| d.recovery);
                if recovery.is_some() {
                    text(ui, content, t, l.text("recovery_needed"), 22.0)?;
                }
                button(
                    ui,
                    content,
                    &mut state.controls,
                    t,
                    &l,
                    match recovery {
                        Some(Recovery::Rollback) => "recovery",
                        Some(Recovery::Retry) => "retry_restore",
                        None => "backup",
                    },
                    if recovery.is_some() {
                        Action::Recover(i)
                    } else {
                        Action::AskBackup(i)
                    },
                    !model.busy,
                )?;
            }
            let loading = page == Page::Library && model.scanning
                || matches!(page, Page::Cloud(_)) && model.busy;
            if loading {
                let center = ui.insert(
                    content,
                    flex_column().update_layout(|s| {
                        s.justify_content = Some(nivora_ui::JustifyContent::CENTER);
                        s.align_items = Some(AlignItems::CENTER);
                        s.gap.height = Length::length(24.0);
                    }),
                )?;
                ui.insert(center, t.spinner(SpinnerSize::Large))?;
                text(ui, center, t, l.text("loading"), 25.0)?;
            } else {
                let count = row_count(model, &page);
                if count > 0 {
                    state.list = Some(VirtualList::new(ui, content, count, 90.0)?.with_overscan(1));
                } else if !model.busy {
                    let center = ui.insert(
                        content,
                        flex_column().update_layout(|s| {
                            s.justify_content = Some(nivora_ui::JustifyContent::CENTER);
                            s.align_items = Some(AlignItems::CENTER);
                        }),
                    )?;
                    text(
                        ui,
                        center,
                        t,
                        l.text(if page == Page::Library {
                            "empty"
                        } else {
                            "empty_backups"
                        }),
                        28.0,
                    )?;
                }
            }
        }
        Page::Game(i) => {
            let game = &model.games[*i];
            let side = ui.insert(
                content,
                WidgetSpec::column().update_layout(|s| {
                    s.size.width = Dim::length(240.0);
                    s.flex_shrink = 0.0;
                    s.gap.height = Length::length(10.0);
                }),
            )?;
            image(
                ui,
                side,
                format!("game:{}:{i}", model.catalog_revision),
                220.0,
            )?;
            text(ui, side, t, &game.title_id, 20.0)?;
            let body = ui.insert(content, flex_column())?;
            text(
                ui,
                body,
                t,
                format!("{}\n{}", l.text("directory"), game.path.display()),
                18.0,
            )?;
            if let Some(detail) = model.detail(*i) {
                text(
                    ui,
                    body,
                    t,
                    format!("{}  {}", l.text("size"), bytes(detail.size)),
                    22.0,
                )?;
                if detail.recovery.is_some() {
                    text(ui, body, t, l.text("recovery_needed"), 22.0)?;
                }
            } else {
                ui.insert(body, t.spinner(SpinnerSize::Normal))?;
            }
            button(
                ui,
                body,
                &mut state.controls,
                t,
                &l,
                "backups",
                Action::Backups(*i),
                model.detail(*i).is_some(),
            )?;
            button(
                ui,
                body,
                &mut state.controls,
                t,
                &l,
                "cloud",
                Action::Cloud(*i),
                !model.busy,
            )?;
        }
        Page::Snapshot(i, id) => {
            let scroll = ui.insert(
                content,
                WidgetSpec::scroll().update_layout(|s| {
                    s.flex_grow = 1.0;
                    s.min_size.height = Auto::length(0.0);
                }),
            )?;
            let content = ui.insert(
                scroll,
                flex_column().update_layout(|s| {
                    s.flex_basis = Dim::auto();
                    s.flex_shrink = 0.0;
                }),
            )?;
            if let Some(m) = model
                .detail(*i)
                .and_then(|d| d.backups.iter().find(|m| &m.id == id))
            {
                text(ui, content, t, &m.title, 28.0)?;
                text(ui, content, t, stamp(m.created), 24.0)?;
                text(
                    ui,
                    content,
                    t,
                    format!("{} · {}", m.save_id, bytes(m.bytes())),
                    22.0,
                )?;
                if m.automatic {
                    text(ui, content, t, l.text("automatic"), 18.0)?;
                }
                button(
                    ui,
                    content,
                    &mut state.controls,
                    t,
                    &l,
                    "restore",
                    Action::AskRestore(*i, id.clone()),
                    !model.busy && !model.detail(*i).is_some_and(|d| d.recovery.is_some()),
                )?;
                button(
                    ui,
                    content,
                    &mut state.controls,
                    t,
                    &l,
                    "upload",
                    Action::Upload(*i, id.clone()),
                    !model.busy,
                )?;
                button(
                    ui,
                    content,
                    &mut state.controls,
                    t,
                    &l,
                    "delete",
                    Action::AskDelete(*i, id.clone()),
                    !model.busy,
                )?;
            }
        }
        Page::Settings(tab) => {
            let sidebar = ui.insert(
                content,
                WidgetSpec::column().update_layout(|s| {
                    s.size.width = Dim::length(190.0);
                    s.flex_shrink = 0.0;
                    s.gap.height = Length::length(8.0);
                }),
            )?;
            for (value, key) in [
                (Tab::Interface, "interface"),
                (Tab::Backup, "backup_settings"),
                (Tab::WebDav, "cloud_settings"),
                (Tab::About, "about"),
            ] {
                let action = Action::SettingsTab(value);
                let id = ui.insert(
                    sidebar,
                    t.sidebar_item(l.text(key), *tab == value, action.clone()),
                )?;
                state.controls.push((action, id));
            }
            let scroll = ui.insert(
                content,
                WidgetSpec::scroll().update_layout(|s| {
                    s.flex_grow = 1.0;
                    s.flex_basis = Dim::length(0.0);
                    s.min_size.width = Auto::length(0.0);
                }),
            )?;
            let body = ui.insert(
                scroll,
                flex_column().update_layout(|s| {
                    if *tab == Tab::About {
                        s.min_size.height = Auto::length(340.0);
                        s.justify_content = Some(nivora_ui::JustifyContent::CENTER);
                        s.gap.height = Length::length(28.0);
                        s.padding.left = Length::length(24.0);
                        s.padding.right = Length::length(24.0);
                    }
                }),
            )?;
            match tab {
                Tab::Interface => {
                    control(
                        ui,
                        body,
                        &mut state.controls,
                        Action::Language,
                        t.selector_cell(
                            l.text("language"),
                            if model.config.language == "zh" {
                                "简体中文"
                            } else {
                                "English"
                            },
                            Action::Language,
                        ),
                        true,
                    )?;
                    control(
                        ui,
                        body,
                        &mut state.controls,
                        Action::Theme,
                        t.selector_cell(
                            l.text("theme"),
                            l.text(if model.config.light_theme {
                                "light"
                            } else {
                                "dark"
                            }),
                            Action::Theme,
                        ),
                        true,
                    )?;
                    control(
                        ui,
                        body,
                        &mut state.controls,
                        Action::Animations,
                        t.toggle(
                            l.text("animations"),
                            model.config.animations,
                            Action::Animations,
                        ),
                        true,
                    )?;
                }
                Tab::Backup => {
                    control(
                        ui,
                        body,
                        &mut state.controls,
                        Action::BackupBeforeRestore,
                        t.toggle(
                            l.text("auto_backup_setting"),
                            model.config.backup_before_restore,
                            Action::BackupBeforeRestore,
                        ),
                        !model.busy,
                    )?;
                    text(ui, body, t, l.text("auto_backup_help"), 20.0)?;
                }
                Tab::WebDav => {
                    for field in [WebDavField::Url, WebDavField::User, WebDavField::Password] {
                        let value = field.value(&model.config);
                        let display = if value.is_empty() {
                            l.text("not_set")
                        } else if field == WebDavField::Password {
                            "••••••••".into()
                        } else {
                            value.to_owned()
                        };
                        let action = Action::EditWebDav(field);
                        control(
                            ui,
                            body,
                            &mut state.controls,
                            action.clone(),
                            t.selector_cell(l.text(field.key()), display, action),
                            !model.busy,
                        )?;
                    }
                    button(
                        ui,
                        body,
                        &mut state.controls,
                        t,
                        &l,
                        "connect",
                        Action::TestConnection,
                        !model.busy && !model.config.webdav_url.is_empty(),
                    )?;
                }
                Tab::About => {
                    let identity = ui.insert(
                        body,
                        WidgetSpec::row().update_layout(|s| {
                            s.align_items = Some(AlignItems::CENTER);
                            s.gap.width = Length::length(24.0);
                            s.flex_shrink = 0.0;
                        }),
                    )?;
                    image(ui, identity, APP_ICON.to_string(), 96.0)?;
                    let name = ui.insert(
                        identity,
                        flex_column().update_layout(|s| s.gap.height = Length::length(6.0)),
                    )?;
                    text(ui, name, t, crate::APP_NAME, 32.0)?;
                    let version = text(
                        ui,
                        name,
                        t,
                        format!("{}  {}", l.text("version"), crate::VERSION),
                        18.0,
                    )?;
                    let mut appearance = ui.appearance(version).unwrap();
                    appearance.foreground = t.colors.muted_text;
                    ui.set_appearance(version, appearance)?;
                    text(ui, body, t, l.text("description"), 20.0)?;
                    ui.insert(
                        body,
                        WidgetSpec::row()
                            .update_layout(|s| {
                                s.size.height = Dim::length(1.0);
                                s.flex_shrink = 0.0;
                            })
                            .update_appearance(|a| a.background = Some(t.colors.separator)),
                    )?;
                    text(ui, body, t, l.text("author"), 20.0)?;
                }
            }
        }
        Page::Edit(_) => {
            if let Some(edit) = &model.edit {
                let id = ui.insert(content, editor(t, edit))?;
                ui.focus(id)?;
                state.editor = Some(id);
            }
        }
    }
    // Prefer the page content, while keeping the active navigation tab focusable.
    let preferred = match page {
        Page::Settings(Tab::Interface) => Some(Action::Language),
        Page::Settings(Tab::Backup) => Some(Action::BackupBeforeRestore),
        Page::Settings(Tab::WebDav) => Some(Action::EditWebDav(WebDavField::Url)),
        Page::Settings(Tab::About) => Some(Action::SettingsTab(Tab::About)),
        _ => None,
    };
    let initial = preferred
        .and_then(|wanted| state.controls.iter().find(|(a, _)| *a == wanted))
        .or_else(|| state.controls.first());
    if let Some((_, id)) = initial {
        ui.focus(*id)?;
    }
    ui.set_animations_enabled(model.config.animations);
    Ok((frame.into_ui(), state))
}

pub(super) fn row_count(model: &Model, page: &Page) -> usize {
    match page {
        Page::Library => model.games.len(),
        Page::Backups(i) => model.detail(*i).map_or(0, |d| d.backups.len()),
        Page::Cloud(i) => model.remote.get(&model.games[*i].path).map_or(0, Vec::len),
        _ => 0,
    }
}
pub(super) fn row(model: &Model, page: &Page, index: usize) -> WidgetSpec<Action> {
    let t = theme(model);
    let widget = match page {
        Page::Library => t.list_item(
            &model.games[index].name,
            Some(format!("game:{}:{index}", model.catalog_revision)),
            Action::OpenGame(index),
        ),
        Page::Backups(i) => {
            let m = &model.detail(*i).unwrap().backups[index];
            t.selector_cell(
                stamp(m.created),
                if m.automatic {
                    format!(
                        "{} · {}",
                        bytes(m.bytes()),
                        localizer(model).text("automatic")
                    )
                } else {
                    bytes(m.bytes())
                },
                Action::Snapshot(*i, m.id.clone()),
            )
        }
        Page::Cloud(i) => {
            let m = &model.remote[&model.games[*i].path][index];
            let seconds =
                m.id.split('-')
                    .next()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
            let downloaded = model
                .detail(*i)
                .is_some_and(|detail| detail.backups.iter().any(|local| local.id == m.id));
            t.selector_cell(
                stamp(seconds),
                localizer(model).text(if downloaded { "in_local" } else { "download" }),
                if downloaded {
                    Action::Snapshot(*i, m.id.clone())
                } else {
                    Action::Download(*i, m.id.clone())
                },
            )
        }
        _ => unreachable!(),
    };
    widget
        .update_layout(|s| s.size.height = Dim::length(90.0))
        .update_appearance(|a| a.font_size = 26.0)
}
use crate::platform::time::stamp;

#[derive(Clone, Copy)]
pub(super) struct ProgressContent {
    pub phase: WidgetId,
    pub file: WidgetId,
    pub amount: WidgetId,
    pub bar: WidgetId,
    pub spinner: WidgetId,
}

pub(super) fn progress(
    ui: &mut Ui<Action>,
    parent: WidgetId,
    t: Theme,
) -> Result<ProgressContent, UiError> {
    let body = ui.insert(
        parent,
        WidgetSpec::column().update_layout(|s| {
            s.size.width = Dim::percent(1.0);
            s.flex_shrink = 0.0;
            s.gap.height = Length::length(16.0);
        }),
    )?;
    let spinner = ui.insert(
        body,
        t.spinner(SpinnerSize::Normal)
            .update_layout(|s| s.align_self = Some(AlignItems::CENTER)),
    )?;
    let label = |size, flow| {
        t.label("", size)
            .update_layout(|s| {
                s.size.width = Dim::percent(1.0);
                s.min_size.width = Auto::length(0.0);
                s.flex_shrink = 0.0;
            })
            .update_appearance(|a| {
                a.text_align = nivora_platform::TextAlign::Center;
                a.text_flow = flow;
            })
    };
    let phase = ui.insert(body, label(24.0, TextFlow::Wrap))?;
    let file = ui.insert(body, label(20.0, TextFlow::SingleLine))?;
    let bar = ui.insert(body, t.progress(0.0))?;
    let amount = ui.insert(body, label(20.0, TextFlow::Wrap))?;
    Ok(ProgressContent {
        phase,
        file,
        amount,
        bar,
        spinner,
    })
}

pub(super) fn editor(t: Theme, edit: &Edit) -> WidgetSpec<Action> {
    let mut spec = t
        .text_input("", Action::Close)
        .update_appearance(|a| a.font_size = 26.0)
        .update_layout(|s| {
            s.size.height = Dim::length(64.0);
            s.flex_shrink = 0.0;
        });
    if let nivora_ui::WidgetKind::TextInput { state, .. } = &mut spec.kind {
        *state = edit.text.ui_state(edit.field == WebDavField::Password);
    }
    spec
}
