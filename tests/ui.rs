use nivora_platform::{InputEvent, Key, Size, TextMeasurer, TextStyle};
use std::collections::BTreeMap;
use vita_save::{
    app::{Detail, Model},
    config::Config,
    job::Progress,
    saves::Game,
    ui::{Action, Page, Tab, View},
};
struct Measure;
impl TextMeasurer for Measure {
    fn measure_text(&self, text: &str, style: TextStyle, max: Option<f32>) -> Size {
        let width = text.chars().count() as f32 * style.font_size * 0.6;
        Size {
            width: width.min(max.unwrap_or(f32::MAX)),
            height: style.font_size * 1.3,
        }
    }
}
fn model() -> Model {
    let games: Vec<_> = (0..50)
        .map(|i| Game {
            title_id: format!("PCSG{i:05}"),
            save_id: format!("SAVE{i:05}"),
            name: format!("Game {i}"),
            path: format!("savedata/SAVE{i:05}").into(),
            icon: None,
        })
        .collect();
    let details = games
        .iter()
        .map(|g| (g.path.clone(), Detail::default()))
        .collect();
    Model {
        config: Config::default(),
        games,
        catalog_revision: 0,
        details,
        remote: BTreeMap::new(),
        busy: false,
        cancelling: false,
        scanning: false,
        progress: Progress::default(),
        message: String::new(),
        edit: None,
    }
}
#[test]
fn every_screen_lays_out_at_vita_resolution_and_back_preserves_selection() {
    let m = model();
    let mut view = View::new(&m).unwrap();
    view.layout(&m, &Measure).unwrap();
    view.page_step(24);
    view.layout(&m, &Measure).unwrap();
    assert_eq!(
        view.handle(&m, InputEvent::KeyDown(Key::Accept), &Measure)
            .unwrap(),
        Some(Action::OpenGame(24))
    );
    for page in [
        Page::Game(24),
        Page::Backups(24),
        Page::Cloud(24),
        Page::Settings(Tab::Interface),
        Page::Settings(Tab::Backup),
        Page::Settings(Tab::WebDav),
        Page::Settings(Tab::About),
    ] {
        view.open(&m, page).unwrap();
        view.advance(std::time::Duration::from_secs(1));
        view.layout(&m, &Measure).unwrap();
        assert!(!view.frame().commands.is_empty());
        view.back(&m).unwrap();
        view.layout(&m, &Measure).unwrap();
    }
    assert_eq!(
        view.handle(&m, InputEvent::KeyDown(Key::Accept), &Measure)
            .unwrap(),
        Some(Action::OpenGame(24))
    );
}
#[test]
fn controller_moves_across_virtual_list_windows() {
    let m = model();
    let mut view = View::new(&m).unwrap();
    view.layout(&m, &Measure).unwrap();
    for _ in 0..30 {
        view.handle(&m, InputEvent::KeyDown(Key::Down), &Measure)
            .unwrap();
        view.layout(&m, &Measure).unwrap();
    }
    assert_eq!(
        view.handle(&m, InputEvent::KeyDown(Key::Accept), &Measure)
            .unwrap(),
        Some(Action::OpenGame(30))
    );
}
#[test]
fn busy_detail_and_progress_dialog_are_valid_in_both_languages() {
    let mut m = model();
    m.busy = true;
    for language in ["zh", "en"] {
        m.config.language = language.into();
        let mut view = View::new(&m).unwrap();
        view.open(&m, Page::Game(0)).unwrap();
        view.layout(&m, &Measure).unwrap();
        view.start_progress(&m).unwrap();
        view.layout(&m, &Measure).unwrap();
        view.update_progress(&m).unwrap();
    }
}

#[test]
fn webdav_fields_open_editor_and_password_never_enters_frame_text() {
    use vita_save::text_edit::{Edit, WebDavField};
    let mut m = model();
    m.config.webdav_password = "private-device-password".into();
    let mut view = View::new(&m).unwrap();
    view.open(&m, Page::Settings(Tab::WebDav)).unwrap();
    view.advance(std::time::Duration::from_secs(1));
    view.layout(&m, &Measure).unwrap();
    assert_eq!(
        view.handle(&m, InputEvent::KeyDown(Key::Accept), &Measure)
            .unwrap(),
        Some(Action::EditWebDav(WebDavField::Url))
    );
    for field in [WebDavField::Url, WebDavField::User, WebDavField::Password] {
        m.edit = Some(Edit::new(field, &m.config).unwrap());
        view.open(&m, Page::Edit(field)).unwrap();
        view.advance(std::time::Duration::from_secs(1));
        view.layout(&m, &Measure).unwrap();
        let request = view
            .ime_request()
            .expect("Nivora text field must have IME focus");
        assert!(request.cursor.y + request.cursor.height <= 220.0);
        let texts: Vec<_> = view
            .frame()
            .commands
            .into_iter()
            .filter_map(|c| match c {
                nivora_platform::DrawCommand::Text { text, .. } => Some(text),
                _ => None,
            })
            .collect();
        assert!(!texts.iter().any(|s| s.contains("private-device-password")));
        if field == WebDavField::Password {
            assert!(texts.iter().any(|s| s.contains('•')));
        }
        view.back(&m).unwrap();
        m.edit = None;
    }
}

#[test]
fn refresh_replaces_old_rows_with_a_centered_animated_loader() {
    use nivora_platform::DrawCommand;
    use vita_save::{app::App, platform::Environment};
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir(tmp.path().join("savedata")).unwrap();
    let mut app = App::new(Environment::host(tmp.path())).unwrap();
    let ready = |app: &mut App| {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while app.model.busy {
            assert!(std::time::Instant::now() < deadline, "scan did not finish");
            app.poll().unwrap();
            std::thread::yield_now();
        }
    };
    ready(&mut app);
    app.model = model();
    app.view.rebuild(&app.model).unwrap();
    app.view.layout(&app.model, &Measure).unwrap();
    assert!(
        app.view
            .frame()
            .commands
            .iter()
            .any(|c| { matches!(c, DrawCommand::Text { text, .. } if text.starts_with("Game ")) })
    );

    app.dispatch(Action::Refresh).unwrap();
    assert!(app.model.scanning);
    // Repeated Start presses do not stack jobs or interrupt the loading screen.
    app.dispatch(Action::Refresh).unwrap();
    app.view.layout(&app.model, &Measure).unwrap();
    let frame = app.view.frame();
    assert!(!frame.commands.iter().any(|c| {
        matches!(c, DrawCommand::Text { text, .. } if text.starts_with("Game "))
            || matches!(c, DrawCommand::Image { asset, .. } if asset.starts_with("game:"))
    }));
    let strokes: Vec<_> = frame
        .commands
        .iter()
        .filter_map(|c| match c {
            DrawCommand::StrokeLine {
                from, to, width, ..
            } if *width > 2.0 => Some((*from, *to)),
            _ => None,
        })
        .collect();
    assert!(strokes.len() >= 8, "the Nivora spinner must be visible");
    let x = strokes.iter().map(|(a, b)| a.x + b.x).sum::<f32>() / (2.0 * strokes.len() as f32);
    let y = strokes.iter().map(|(a, b)| a.y + b.y).sum::<f32>() / (2.0 * strokes.len() as f32);
    assert!((x - 480.0).abs() < 1.0, "spinner x={x}");
    assert!((200.0..290.0).contains(&y), "spinner y={y}");
    assert!(!matches!(
        app.view
            .handle(&app.model, InputEvent::KeyDown(Key::Accept), &Measure)
            .unwrap(),
        Some(Action::OpenGame(_))
    ));
    app.view.advance(std::time::Duration::from_millis(150));
    assert_ne!(app.view.frame(), frame, "spinner must animate");
    ready(&mut app);
    assert!(!app.model.scanning);
    app.view.layout(&app.model, &Measure).unwrap();
    assert!(app.view.frame().commands.iter().any(|c| {
        matches!(c, DrawCommand::Text { text, .. } if text == &app.view.text(&app.model, "empty"))
    }));
}

#[test]
fn settings_focus_switches_tabs_and_stays_in_the_sidebar() {
    let m = model();
    let mut view = View::new(&m).unwrap();
    view.open(&m, Page::Settings(Tab::Interface)).unwrap();
    view.advance(std::time::Duration::from_secs(1));
    view.layout(&m, &Measure).unwrap();
    // Enter the sidebar from the language field, then switch without Accept.
    assert_eq!(
        view.handle(&m, InputEvent::KeyDown(Key::Left), &Measure)
            .unwrap(),
        None
    );
    for (key, tab) in [
        (Key::Down, Tab::Backup),
        (Key::Down, Tab::WebDav),
        (Key::Down, Tab::About),
        (Key::Up, Tab::WebDav),
        (Key::Up, Tab::Backup),
        (Key::Up, Tab::Interface),
    ] {
        assert_eq!(
            view.handle(&m, InputEvent::KeyDown(key), &Measure).unwrap(),
            Some(Action::SettingsTab(tab))
        );
        view.replace(&m, Page::Settings(tab)).unwrap();
        view.layout(&m, &Measure).unwrap();
        assert_eq!(
            view.handle(&m, InputEvent::KeyDown(Key::Accept), &Measure)
                .unwrap(),
            Some(Action::SettingsTab(tab))
        );
    }
    view.handle(&m, InputEvent::KeyDown(Key::Right), &Measure)
        .unwrap();
    assert_eq!(
        view.handle(&m, InputEvent::KeyDown(Key::Accept), &Measure)
            .unwrap(),
        Some(Action::Language)
    );
    view.dropdown(&m, Action::Language).unwrap();
    view.layout(&m, &Measure).unwrap();
    assert_eq!(
        view.handle(&m, InputEvent::KeyDown(Key::Down), &Measure)
            .unwrap(),
        None
    );
    assert_eq!(view.page(), &Page::Settings(Tab::Interface));
    view.handle(&m, InputEvent::KeyDown(Key::Back), &Measure)
        .unwrap();
    view.back(&m).unwrap();
    assert_eq!(view.page(), &Page::Library);
}

#[test]
fn automatic_backup_setting_persists_and_controls_confirmation_and_restore() {
    use nivora_platform::DrawCommand;
    use vita_save::{app::App, backup::Store, job::Control, platform::Environment};
    let tmp = tempfile::tempdir().unwrap();
    let env = Environment::host(tmp.path());
    std::fs::create_dir(tmp.path().join("savedata")).unwrap();
    let mut app = App::new(env.clone()).unwrap();
    let ready = |app: &mut App| {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while app.model.busy {
            assert!(std::time::Instant::now() < deadline);
            app.poll().unwrap();
            std::thread::yield_now();
        }
    };
    ready(&mut app);
    let game = Game {
        path: tmp.path().join("savedata/SAVE00000"),
        ..model().games.remove(0)
    };
    std::fs::create_dir(&game.path).unwrap();
    std::fs::write(game.path.join("save.bin"), b"old").unwrap();
    let store = Store::new(&env.data);
    let snapshot = store
        .create(&game, &game.path, false, &Control::default())
        .unwrap();
    app.model.games = vec![game.clone()];
    app.model.details.insert(
        game.path.clone(),
        Detail {
            backups: vec![snapshot.clone()],
            ..Default::default()
        },
    );
    assert!(app.model.config.backup_before_restore);
    for (enabled, language, message) in [
        (false, "zh", "不会留下副本"),
        (true, "en", "backed up first"),
    ] {
        app.model.config.language = language.into();
        app.dispatch(Action::SettingsTab(Tab::Backup)).unwrap();
        app.view.layout(&app.model, &Measure).unwrap();
        let action = app
            .view
            .handle(&app.model, InputEvent::KeyDown(Key::Accept), &Measure)
            .unwrap()
            .unwrap();
        assert_eq!(action, Action::BackupBeforeRestore);
        app.dispatch(action).unwrap();
        assert_eq!(app.model.config.backup_before_restore, enabled);
        assert_eq!(
            Config::load(&env.data).unwrap().backup_before_restore,
            enabled
        );
        app.view.layout(&app.model, &Measure).unwrap();
        assert_eq!(
            app.view
                .handle(&app.model, InputEvent::KeyDown(Key::Accept), &Measure)
                .unwrap(),
            Some(Action::BackupBeforeRestore)
        );
        app.dispatch(Action::Snapshot(0, snapshot.id.clone()))
            .unwrap();
        app.dispatch(Action::AskRestore(0, snapshot.id.clone()))
            .unwrap();
        app.view.layout(&app.model, &Measure).unwrap();
        let text = app
            .view
            .frame()
            .commands
            .iter()
            .filter_map(|c| match c {
                DrawCommand::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join(" ");
        assert!(text.contains(message), "{text}");
        std::fs::write(game.path.join("save.bin"), b"current").unwrap();
        app.dispatch(Action::Close).unwrap();
        app.dispatch(Action::Restore(0, snapshot.id.clone()))
            .unwrap();
        ready(&mut app);
        assert_eq!(std::fs::read(game.path.join("save.bin")).unwrap(), b"old");
        assert_eq!(
            store.list(&game).unwrap().len(),
            if enabled { 2 } else { 1 }
        );
        app.dispatch(Action::Close).unwrap();
    }
}

#[test]
fn interrupted_restore_action_describes_the_journal_not_the_current_setting() {
    use nivora_platform::DrawCommand;
    use vita_save::backup::Recovery;
    let mut m = model();
    for (recovery, setting, expected, absent) in [
        (Recovery::Rollback, false, "recovery", "retry_restore"),
        (Recovery::Retry, true, "retry_restore", "recovery"),
    ] {
        m.config.backup_before_restore = setting;
        m.details.get_mut(&m.games[0].path).unwrap().recovery = Some(recovery);
        let mut view = View::new(&m).unwrap();
        view.open(&m, Page::Backups(0)).unwrap();
        view.layout(&m, &Measure).unwrap();
        let frame = view.frame();
        assert!(frame.commands.iter().any(
            |c| matches!(c, DrawCommand::Text { text, .. } if text == &view.text(&m, expected))
        ));
        assert!(!frame.commands.iter().any(
            |c| matches!(c, DrawCommand::Text { text, .. } if text == &view.text(&m, absent))
        ));
        assert_eq!(
            view.handle(&m, InputEvent::KeyDown(Key::Accept), &Measure)
                .unwrap(),
            Some(Action::Recover(0))
        );
    }
}

#[test]
fn empty_states_are_centered_and_game_has_only_backup_destinations() {
    use nivora_platform::DrawCommand;
    let mut m = model();
    m.config.animations = false;
    let mut view = View::new(&m).unwrap();
    view.open(&m, Page::Game(0)).unwrap();
    view.layout(&m, &Measure).unwrap();
    assert_eq!(
        view.handle(&m, InputEvent::KeyDown(Key::Accept), &Measure)
            .unwrap(),
        Some(Action::Backups(0))
    );
    view.handle(&m, InputEvent::KeyDown(Key::Down), &Measure)
        .unwrap();
    assert_eq!(
        view.handle(&m, InputEvent::KeyDown(Key::Accept), &Measure)
            .unwrap(),
        Some(Action::Cloud(0))
    );
    for page in [Page::Backups(0), Page::Cloud(0)] {
        view.open(&m, page).unwrap();
        view.layout(&m, &Measure).unwrap();
        let bounds = view
            .frame()
            .commands
            .into_iter()
            .find_map(|command| match command {
                DrawCommand::Text { text, bounds, .. }
                    if text == view.text(&m, "empty_backups") =>
                {
                    Some(bounds)
                }
                _ => None,
            })
            .unwrap();
        assert!(
            (bounds.x + bounds.width / 2.0 - 480.0).abs() < 1.0,
            "{bounds:?}"
        );
        assert!(
            (240.0..330.0).contains(&(bounds.y + bounds.height / 2.0)),
            "{bounds:?}"
        );
    }
}

#[test]
fn progress_cancel_keeps_the_dialog_open_and_blocks_page_shortcuts() {
    let mut m = model();
    m.config.animations = false;
    m.busy = true;
    let mut view = View::new(&m).unwrap();
    view.open(&m, Page::Settings(Tab::WebDav)).unwrap();
    view.start_progress(&m).unwrap();
    view.layout(&m, &Measure).unwrap();
    assert!(!view.shortcut_allowed());
    for key in [Key::Accept, Key::Back] {
        assert_eq!(
            view.handle(&m, InputEvent::KeyDown(key), &Measure).unwrap(),
            Some(Action::Cancel)
        );
        assert_eq!(view.page(), &Page::Settings(Tab::WebDav));
        assert!(!view.shortcut_allowed());
    }
    m.cancelling = true;
    view.update_progress(&m).unwrap();
    view.layout(&m, &Measure).unwrap();
    assert_eq!(
        view.handle(&m, InputEvent::KeyDown(Key::Accept), &Measure)
            .unwrap(),
        None
    );
}

#[test]
fn connection_dialog_shows_the_complete_cancel_label_without_fake_byte_progress() {
    use nivora_platform::DrawCommand;
    for language in ["zh", "en"] {
        let mut m = model();
        m.config.animations = false;
        m.config.language = language.into();
        m.busy = true;
        m.progress.phase = "connect";
        let mut view = View::new(&m).unwrap();
        view.open(&m, Page::Settings(Tab::WebDav)).unwrap();
        view.start_progress(&m).unwrap();
        view.layout(&m, &Measure).unwrap();
        let frame = view.frame();
        let label = view.text(&m, "cancel");
        let cancel = frame
            .commands
            .iter()
            .find_map(|c| match c {
                DrawCommand::Text { text, bounds, .. } if text == &label => Some(bounds),
                _ => None,
            })
            .expect("cancel label must not be ellipsized");
        assert!(
            cancel.width >= 200.0,
            "cancel footer must have a usable touch area: {cancel:?}"
        );
        let center = cancel.center();
        view.handle(&m, InputEvent::PointerDown(center), &Measure)
            .unwrap();
        assert_eq!(
            view.handle(&m, InputEvent::PointerUp(center), &Measure)
                .unwrap(),
            Some(Action::Cancel)
        );
        assert!(
            !view.shortcut_allowed(),
            "cancel must wait for worker cleanup"
        );
        let outside = nivora_platform::Point { x: 10.0, y: 10.0 };
        view.handle(&m, InputEvent::PointerDown(outside), &Measure)
            .unwrap();
        assert_eq!(
            view.handle(&m, InputEvent::PointerUp(outside), &Measure)
                .unwrap(),
            None
        );
        assert_eq!(
            view.handle(&m, InputEvent::KeyDown(Key::Accept), &Measure)
                .unwrap(),
            Some(Action::Cancel)
        );
        assert!(
            !frame
                .commands
                .iter()
                .any(|c| matches!(c, DrawCommand::Text { text, .. } if text.contains("0 B / 0 B")))
        );
    }
}

#[test]
fn create_and_delete_complete_in_dialogs_on_the_local_backup_page() {
    use vita_save::{app::App, platform::Environment};
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("savedata/SAVE00001");
    std::fs::create_dir_all(&path).unwrap();
    std::fs::write(path.join("data.bin"), b"save data").unwrap();
    let mut app = App::new(Environment::host(tmp.path())).unwrap();
    let ready = |app: &mut App| {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while app.model.busy {
            assert!(std::time::Instant::now() < deadline);
            app.poll().unwrap();
            std::thread::yield_now();
        }
    };
    ready(&mut app);
    app.model.config.animations = false;
    app.model.games = vec![Game {
        title_id: "PCSG00001".into(),
        save_id: "SAVE00001".into(),
        name: "Game".into(),
        path: path.clone(),
        icon: None,
    }];
    app.dispatch(Action::OpenGame(0)).unwrap();
    ready(&mut app);
    app.dispatch(Action::Backups(0)).unwrap();
    app.dispatch(Action::AskBackup(0)).unwrap();
    app.view.layout(&app.model, &Measure).unwrap();
    app.view
        .handle(&app.model, InputEvent::KeyDown(Key::Right), &Measure)
        .unwrap();
    let action = app
        .view
        .handle(&app.model, InputEvent::KeyDown(Key::Accept), &Measure)
        .unwrap()
        .unwrap();
    assert_eq!(action, Action::Backup(0));
    app.dispatch(action).unwrap();
    assert_eq!(app.view.page(), &Page::Backups(0));
    ready(&mut app);
    assert_eq!(app.model.detail(0).unwrap().backups.len(), 1);
    let id = app.model.detail(0).unwrap().backups[0].id.clone();
    app.dispatch(Action::Close).unwrap();
    app.view.layout(&app.model, &Measure).unwrap();
    // The creation control remains reachable from the first virtual list row.
    app.view
        .handle(&app.model, InputEvent::KeyDown(Key::Up), &Measure)
        .unwrap();
    assert_eq!(
        app.view
            .handle(&app.model, InputEvent::KeyDown(Key::Accept), &Measure)
            .unwrap(),
        Some(Action::AskBackup(0))
    );
    app.dispatch(Action::DeleteSelected).unwrap();
    app.dispatch(Action::Delete(0, id)).unwrap();
    ready(&mut app);
    assert_eq!(app.view.page(), &Page::Backups(0));
    assert!(app.model.detail(0).unwrap().backups.is_empty());
    assert_eq!(std::fs::read(path.join("data.bin")).unwrap(), b"save data");
}

#[test]
fn task_completion_restores_the_triggering_settings_control() {
    let mut m = model();
    m.config.animations = false;
    m.config.webdav_url = "https://example.com/dav/".into();
    let mut view = View::new(&m).unwrap();
    view.open(&m, Page::Settings(Tab::WebDav)).unwrap();
    view.layout(&m, &Measure).unwrap();
    for _ in 0..3 {
        view.handle(&m, InputEvent::KeyDown(Key::Down), &Measure)
            .unwrap();
    }
    assert_eq!(
        view.handle(&m, InputEvent::KeyDown(Key::Accept), &Measure)
            .unwrap(),
        Some(Action::TestConnection)
    );
    m.busy = true;
    view.rebuild(&m).unwrap();
    view.start_progress(&m).unwrap();
    view.layout(&m, &Measure).unwrap();
    m.busy = false;
    view.dismiss_dialog();
    view.rebuild(&m).unwrap();
    m.message = "Connected".into();
    view.result(&m).unwrap();
    view.layout(&m, &Measure).unwrap();
    assert_eq!(
        view.handle(&m, InputEvent::KeyDown(Key::Accept), &Measure)
            .unwrap(),
        Some(Action::Close)
    );
    assert_eq!(
        view.handle(&m, InputEvent::KeyDown(Key::Accept), &Measure)
            .unwrap(),
        Some(Action::TestConnection)
    );
    assert_eq!(view.page(), &Page::Settings(Tab::WebDav));
}

#[test]
fn controller_focus_reappears_after_touching_a_virtual_row() {
    use nivora_platform::DrawCommand;
    let mut m = model();
    m.config.animations = false;
    let mut view = View::new(&m).unwrap();
    view.layout(&m, &Measure).unwrap();
    let point = view
        .frame()
        .commands
        .iter()
        .find_map(|c| match c {
            DrawCommand::Text { text, bounds, .. } if text == "Game 0" => Some(bounds.center()),
            _ => None,
        })
        .unwrap();
    view.handle(&m, InputEvent::PointerDown(point), &Measure)
        .unwrap();
    view.handle(&m, InputEvent::PointerUp(point), &Measure)
        .unwrap();
    let is_focus = |c: &DrawCommand| {
        matches!(
            c,
            DrawCommand::FocusRing { .. } | DrawCommand::StrokeRoundedRect { width: 3.0, .. }
        )
    };
    assert!(!view.frame().commands.iter().any(is_focus));
    // At the first row, Up does not move focus but must reveal its decoration.
    view.handle(&m, InputEvent::KeyDown(Key::Up), &Measure)
        .unwrap();
    assert!(view.frame().commands.iter().any(is_focus));
    view.handle(&m, InputEvent::KeyDown(Key::Down), &Measure)
        .unwrap();
    assert_eq!(
        view.handle(&m, InputEvent::KeyDown(Key::Accept), &Measure)
            .unwrap(),
        Some(Action::OpenGame(1))
    );
}

#[test]
fn known_progress_shows_percent_and_long_filenames_stay_inside_the_dialog() {
    use nivora_platform::DrawCommand;
    let mut m = model();
    m.config.animations = false;
    m.busy = true;
    m.progress = Progress {
        phase: "backup",
        file: "存档目录/".repeat(60),
        done: 1024,
        total: 4096,
    };
    let mut view = View::new(&m).unwrap();
    view.start_progress(&m).unwrap();
    view.layout(&m, &Measure).unwrap();
    let frame = view.frame();
    assert!(
        frame
            .commands
            .iter()
            .any(|c| matches!(c, DrawCommand::Text { text, .. } if text.starts_with("25%")))
    );
    let bounds = frame
        .commands
        .iter()
        .find_map(|c| match c {
            DrawCommand::Text { text, bounds, .. } if text.starts_with("存档目录/") => {
                Some(bounds)
            }
            _ => None,
        })
        .unwrap();
    assert!(
        bounds.width > 400.0 && bounds.x >= 120.0 && bounds.x + bounds.width <= 840.0,
        "{bounds:?}"
    );
}

#[test]
fn cloud_rows_open_downloaded_backups_and_timestamps_omit_timezone_suffixes() {
    use nivora_platform::DrawCommand;
    use vita_save::{backup::Manifest, cloud::Remote};
    let mut m = model();
    m.config.animations = false;
    let id = "1704067200-abcdef".to_string();
    let game = &m.games[0];
    m.remote
        .insert(game.path.clone(), vec![Remote { id: id.clone() }]);
    m.details
        .get_mut(&game.path)
        .unwrap()
        .backups
        .push(Manifest {
            version: 1,
            id: id.clone(),
            title_id: game.title_id.clone(),
            save_id: game.save_id.clone(),
            title: game.name.clone(),
            created: 1704067200,
            automatic: true,
            directories: vec![],
            files: vec![],
        });
    let mut view = View::new(&m).unwrap();
    view.open(&m, Page::Cloud(0)).unwrap();
    view.layout(&m, &Measure).unwrap();
    assert_eq!(
        view.handle(&m, InputEvent::KeyDown(Key::Accept), &Measure)
            .unwrap(),
        Some(Action::Snapshot(0, id))
    );
    assert!(view.frame().commands.iter().any(
        |c| matches!(c, DrawCommand::Text { text, .. } if text == &view.text(&m, "in_local"))
    ));
    view.open(&m, Page::Backups(0)).unwrap();
    view.layout(&m, &Measure).unwrap();
    let frame = view.frame();
    assert!(
        frame
            .commands
            .iter()
            .any(|c| matches!(c, DrawCommand::Text { text, .. } if text == "2024-01-01 00:00:00"))
    );
    assert!(
        !frame
            .commands
            .iter()
            .any(|c| matches!(c, DrawCommand::Text { text, .. } if text.contains("UTC")))
    );
    assert!(frame.commands.iter().any(|c| matches!(c, DrawCommand::Text { text, .. } if text.contains(&view.text(&m, "automatic")))));
}
