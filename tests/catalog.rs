use std::fs;
use vita_save::{config::Config, job::Control, saves};
#[test]
fn catalog_uses_real_save_id_from_database() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("savedata");
    fs::create_dir_all(root.join("SHARED001")).unwrap();
    let path = tmp.path().join("app.db");
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("CREATE TABLE tbl_appinfo (titleid TEXT, key INTEGER, val TEXT); CREATE TABLE tbl_appinfo_icon (titleid TEXT, type INTEGER, title TEXT, iconpath TEXT); INSERT INTO tbl_appinfo VALUES ('PCSG00001',278217076,'SHARED001'); INSERT INTO tbl_appinfo_icon VALUES ('PCSG00001',0,'Game name',NULL);").unwrap();
    drop(db);
    let games = saves::scan(&[root], Some(&path), &Control::default()).unwrap();
    assert_eq!(games.len(), 1);
    assert_eq!(games[0].title_id, "PCSG00001");
    assert_eq!(games[0].save_id, "SHARED001");
    assert_eq!(games[0].name, "Game name");
    fs::write(tmp.path().join("app.db-wal"), b"busy").unwrap();
    assert!(saves::app_database(&path).is_err());
}
#[test]
fn interrupted_config_replace_recovers_last_committed_settings() {
    let tmp = tempfile::tempdir().unwrap();
    let config = Config {
        language: "en".into(),
        webdav_password: "test-only-secret".into(),
        ..Default::default()
    };
    config.save(tmp.path()).unwrap();
    fs::rename(
        tmp.path().join("config.toml"),
        tmp.path().join("config.previous"),
    )
    .unwrap();
    fs::write(tmp.path().join("config.new"), b"partial").unwrap();
    let loaded = Config::load(tmp.path()).unwrap();
    assert_eq!(loaded.language, "en");
    assert_eq!(loaded.webdav_password, "test-only-secret");
    loaded.save(tmp.path()).unwrap();
    assert_eq!(Config::load(tmp.path()).unwrap().language, "en");
}
#[test]
fn malformed_config_does_not_echo_credentials() {
    let tmp = tempfile::tempdir().unwrap();
    fs::write(
        tmp.path().join("config.toml"),
        "webdav_password = \"private-test-secret\n",
    )
    .unwrap();
    let error = match Config::load(tmp.path()) {
        Ok(_) => panic!("invalid config accepted"),
        Err(e) => format!("{e:#}"),
    };
    assert!(!error.contains("private-test-secret"));
}
