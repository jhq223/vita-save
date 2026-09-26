use vita_save::{
    app::App,
    config::Config,
    platform::Environment,
    text_edit::{EditText, WebDavField},
    ui::{Action, Page, Tab},
};

#[test]
fn utf16_positions_preserve_surrogates_and_composition() {
    let utf16: Vec<_> = "a😀中文z".encode_utf16().collect();
    let text = EditText::from_utf16(&utf16, 4, 3..5).unwrap();
    assert_eq!(text.caret, "a😀中".len());
    let mut state = text.ui_state(false);
    assert_eq!(state.value(), "a😀z");
    state.handle(nivora_platform::TextInputEvent::Commit("中文".into()));
    assert_eq!(state.value(), "a😀中文z");
    assert!(EditText::from_utf16(&utf16, 2, 0..0).is_err());
    assert!(EditText::from_utf16(&utf16, 8, 0..0).is_err());
    assert!(EditText::from_utf16(&utf16, 0, 0..99).is_err());
    let composed: Vec<_> = "a\u{301}b".encode_utf16().collect();
    assert_eq!(
        EditText::from_utf16(&composed, 1, 0..0)
            .unwrap()
            .ui_state(false)
            .cursor(),
        0
    );
}

#[test]
fn submission_reads_committed_utf16_without_reusing_preedit_or_stale_suffix() {
    use vita_save::text_edit::committed_utf16;
    let mut buffer: Vec<u16> = "用户😀".encode_utf16().collect();
    buffer.extend([0, 0xD800]);
    assert_eq!(committed_utf16(&buffer).unwrap(), "用户😀");
    assert_eq!(committed_utf16(&[0]).unwrap(), "");
    assert!(committed_utf16(&[65]).is_err());
    assert!(committed_utf16(&[0xD800, 0]).is_err());
}

#[test]
fn passwords_are_masked_and_not_trimmed_or_echoed_by_validation() {
    let text = EditText::new(" secret😀 ".into());
    let state = text.ui_state(true);
    assert_eq!(state.value(), "•••••••••");
    let config = WebDavField::Password
        .apply(&Config::default(), text.value)
        .unwrap();
    assert_eq!(config.webdav_password, " secret😀 ");
    let bad = WebDavField::Password.apply(&config, "private-secret\n".into());
    let error = bad.err().unwrap().to_string();
    assert!(!error.contains("private-secret"));
    assert!(
        WebDavField::Password
            .apply(&config, "😀".repeat(128))
            .is_ok()
    );
    assert!(
        WebDavField::Password
            .apply(&config, "😀".repeat(129))
            .is_err()
    );
    for url in [
        "file:///ux0",
        "https://user:secret@example.com/",
        "https://example.com/?token=secret",
    ] {
        assert!(WebDavField::Url.apply(&config, url.into()).is_err());
    }
    assert!(
        WebDavField::User
            .apply(&config, "user:name".into())
            .is_err()
    );
}

fn ready(app: &mut App) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while app.model.busy {
        assert!(std::time::Instant::now() < deadline, "scan did not finish");
        app.poll().unwrap();
        std::thread::yield_now();
    }
}

#[test]
fn editing_saves_only_on_accept_and_blocks_underlying_navigation() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir(tmp.path().join("savedata")).unwrap();
    let env = Environment::host(tmp.path());
    let mut app = App::new(env.clone()).unwrap();
    app.dispatch(Action::Settings).unwrap();
    app.dispatch(Action::SettingsTab(Tab::WebDav)).unwrap();
    ready(&mut app);
    assert_eq!(app.view.page(), &Page::Settings(Tab::WebDav));

    app.dispatch(Action::EditWebDav(WebDavField::Url)).unwrap();
    app.dispatch(Action::Back).unwrap();
    app.dispatch(Action::Settings).unwrap();
    assert_eq!(app.view.page(), &Page::Edit(WebDavField::Url));
    app.update_edit(EditText::new("https://discard.example/".into()))
        .unwrap();
    app.finish_edit(None).unwrap();
    assert_eq!(app.view.page(), &Page::Settings(Tab::WebDav));
    assert!(Config::load(&env.data).unwrap().webdav_url.is_empty());

    app.dispatch(Action::EditWebDav(WebDavField::Url)).unwrap();
    app.finish_edit(Some("https://example.com/dav/".into()))
        .unwrap();
    app.dispatch(Action::EditWebDav(WebDavField::Password))
        .unwrap();
    app.finish_edit(Some(" secret ".into())).unwrap();
    let saved = Config::load(&env.data).unwrap();
    assert_eq!(saved.webdav_url, "https://example.com/dav/");
    assert_eq!(saved.webdav_password, " secret ");

    app.dispatch(Action::EditWebDav(WebDavField::Url)).unwrap();
    assert!(app.finish_edit(Some("bad-url".into())).is_err());
    assert!(app.model.edit.is_none());
    assert_eq!(app.model.config.webdav_url, saved.webdav_url);
    assert_eq!(
        Config::load(&env.data).unwrap().webdav_url,
        saved.webdav_url
    );
    app.dispatch(Action::EditWebDav(WebDavField::Password))
        .unwrap();
    app.finish_edit(Some(String::new())).unwrap();
    assert!(Config::load(&env.data).unwrap().webdav_password.is_empty());
}

#[test]
fn failed_save_keeps_previous_config_in_memory_and_on_disk() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir(tmp.path().join("savedata")).unwrap();
    let env = Environment::host(tmp.path());
    let mut app = App::new(env.clone()).unwrap();
    ready(&mut app);
    std::fs::create_dir(env.data.join("config.new")).unwrap();
    app.dispatch(Action::EditWebDav(WebDavField::Password))
        .unwrap();
    assert!(app.finish_edit(Some("do-not-save".into())).is_err());
    assert!(app.model.config.webdav_password.is_empty());
    assert!(Config::load(&env.data).unwrap().webdav_password.is_empty());
}
