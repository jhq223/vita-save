use std::{fs, io::Cursor};
use tempfile::TempDir;
use vita_save::{
    backup::{self, Journal, Store},
    cloud::archive,
    job::Control,
    saves::Game,
};

fn fixture() -> (TempDir, Game, Store) {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("savedata/SHARED001");
    fs::create_dir_all(path.join("sce_sys")).unwrap();
    fs::create_dir(path.join("empty")).unwrap();
    fs::write(path.join("data.bin"), vec![7; 150_000]).unwrap();
    fs::write(path.join("sce_sys/keystone"), b"device-only").unwrap();
    let game = Game {
        title_id: "PCSG00001".into(),
        save_id: "SHARED001".into(),
        name: "A game".into(),
        path,
        icon: None,
    };
    let store = Store::new(temp.path().join("store"));
    (temp, game, store)
}

#[test]
fn delete_removes_only_the_selected_backup_and_protects_recovery() {
    let (_tmp, game, store) = fixture();
    let c = Control::default();
    let first = store.create(&game, &game.path, false, &c).unwrap();
    let second = store.create(&game, &game.path, true, &c).unwrap();
    let journal = Journal {
        version: 2,
        game: game.clone(),
        rollback: Some(second.id.clone()),
        requested: first.id.clone(),
    };
    let path = store.journal_path(&game);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    backup::write_synced(&path, &postcard::to_stdvec(&journal).unwrap()).unwrap();
    assert!(store.delete(&game, &first.id, &c).is_err());
    assert!(store.delete(&game, &second.id, &c).is_err());
    fs::remove_file(path).unwrap();
    let cancelled = Control::default();
    cancelled.cancel();
    assert!(store.delete(&game, &first.id, &cancelled).is_err());
    store.verify(&first.id, &c).unwrap();
    let mut other = game.clone();
    other.save_id = "OTHER".into();
    assert!(store.delete(&other, &first.id, &c).is_err());
    store.delete(&game, &first.id, &c).unwrap();
    assert_eq!(store.list(&game).unwrap(), vec![second]);
    assert_eq!(
        fs::read(game.path.join("data.bin")).unwrap(),
        vec![7; 150_000]
    );
}
#[test]
fn backup_restore_truncates_removes_obsolete_and_preserves_keys() {
    let (_tmp, game, store) = fixture();
    let control = Control::default();
    let old = store.create(&game, &game.path, false, &control).unwrap();
    assert_eq!(old.bytes(), 150_000);
    assert_ne!(old.title_id, old.save_id);
    fs::write(game.path.join("data.bin"), vec![8; 170_000]).unwrap();
    fs::write(game.path.join("extra.bin"), b"new").unwrap();
    let automatic = store
        .restore(&game, &game.path, &old.id, None, true, &control)
        .unwrap()
        .unwrap();
    assert_eq!(
        fs::read(game.path.join("data.bin")).unwrap(),
        vec![7; 150_000]
    );
    assert!(!game.path.join("extra.bin").exists());
    assert_eq!(
        fs::read(game.path.join("sce_sys/keystone")).unwrap(),
        b"device-only"
    );
    assert!(game.path.join("empty").is_dir());
    assert!(store.verify(&automatic, &control).unwrap().automatic);
    assert!(store.pending(&game).unwrap().is_none());
}
#[test]
fn restore_without_automatic_backup_keeps_only_the_requested_snapshot() {
    let (_tmp, game, store) = fixture();
    let c = Control::default();
    let snapshot = store.create(&game, &game.path, false, &c).unwrap();
    fs::write(game.path.join("data.bin"), b"new progress").unwrap();
    fs::write(game.path.join("obsolete.bin"), b"extra").unwrap();
    assert!(
        store
            .restore(&game, &game.path, &snapshot.id, None, false, &c)
            .unwrap()
            .is_none()
    );
    assert_eq!(
        fs::read(game.path.join("data.bin")).unwrap(),
        vec![7; 150_000]
    );
    assert!(!game.path.join("obsolete.bin").exists());
    assert_eq!(
        fs::read(game.path.join("sce_sys/keystone")).unwrap(),
        b"device-only"
    );
    assert_eq!(store.list(&game).unwrap(), vec![snapshot]);
    assert!(store.pending(&game).unwrap().is_none());
}

#[test]
fn interrupted_restore_without_rollback_retries_the_requested_save_and_adjusts_account() {
    let (_tmp, game, store) = fixture();
    let c = Control::default();
    // Minimal valid SFO with an ACCOUNT_ID from another device.
    let mut sfo = vec![0; 56];
    sfo[..4].copy_from_slice(b"\0PSF");
    sfo[8..12].copy_from_slice(&36u32.to_le_bytes());
    sfo[12..16].copy_from_slice(&48u32.to_le_bytes());
    sfo[16..20].copy_from_slice(&1u32.to_le_bytes());
    sfo[24..28].copy_from_slice(&8u32.to_le_bytes());
    sfo[28..32].copy_from_slice(&8u32.to_le_bytes());
    sfo[36..47].copy_from_slice(b"ACCOUNT_ID\0");
    sfo[48..56].copy_from_slice(&123u64.to_le_bytes());
    fs::write(game.path.join("sce_sys/param.sfo"), &sfo).unwrap();
    let snapshot = store.create(&game, &game.path, false, &c).unwrap();
    let journal = Journal {
        version: 2,
        game: game.clone(),
        rollback: None,
        requested: snapshot.id.clone(),
    };
    let path = store.journal_path(&game);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    backup::write_synced(&path, &postcard::to_stdvec(&journal).unwrap()).unwrap();
    fs::write(game.path.join("data.bin"), b"partial").unwrap();
    fs::write(game.path.join("half.bin"), b"partial").unwrap();
    fs::write(game.path.join("sce_sys/param.sfo"), b"partial").unwrap();
    let reopened = Store::new(&store.root);
    assert_eq!(
        reopened.pending(&game).unwrap().unwrap().recovery(),
        backup::Recovery::Retry
    );
    assert!(reopened.delete(&game, &snapshot.id, &c).is_err());
    // Changing the setting cannot bypass the pending operation.
    assert!(
        reopened
            .restore(&game, &game.path, &snapshot.id, None, true, &c)
            .is_err()
    );
    let cancelled = Control::default();
    cancelled.cancel();
    assert!(
        reopened
            .recover(&game, &game.path, Some(456), &cancelled)
            .is_err()
    );
    assert!(path.exists());
    assert_eq!(fs::read(game.path.join("data.bin")).unwrap(), b"partial");
    reopened.recover(&game, &game.path, Some(456), &c).unwrap();
    assert_eq!(
        fs::read(game.path.join("data.bin")).unwrap(),
        vec![7; 150_000]
    );
    assert!(!game.path.join("half.bin").exists());
    sfo[48..56].copy_from_slice(&456u64.to_le_bytes());
    assert_eq!(fs::read(game.path.join("sce_sys/param.sfo")).unwrap(), sfo);
    assert!(!path.exists());
    assert_eq!(store.list(&game).unwrap(), vec![snapshot]);
}
#[test]
fn corrupt_backup_is_rejected_before_any_save_changes() {
    let (_tmp, game, store) = fixture();
    let c = Control::default();
    let m = store.create(&game, &game.path, false, &c).unwrap();
    fs::write(
        store.path(&m.id).unwrap().join("files/data.bin"),
        vec![9; 150_000],
    )
    .unwrap();
    for automatic in [false, true] {
        assert!(
            store
                .restore(&game, &game.path, &m.id, None, automatic, &c)
                .is_err()
        );
    }
    assert_eq!(
        fs::read(game.path.join("data.bin")).unwrap(),
        vec![7; 150_000]
    );
    assert!(store.pending(&game).unwrap().is_none());
    assert_eq!(store.list(&game).unwrap().len(), 1);
}
#[test]
fn interrupted_restore_recovers_after_reopening_store() {
    let (_tmp, game, store) = fixture();
    let c = Control::default();
    let m = store.create(&game, &game.path, true, &c).unwrap();
    let journal = Journal {
        version: 2,
        game: game.clone(),
        rollback: Some(m.id.clone()),
        requested: "interrupted".into(),
    };
    let path = store.journal_path(&game);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    backup::write_synced(&path, &postcard::to_stdvec(&journal).unwrap()).unwrap();
    fs::write(game.path.join("data.bin"), b"partial").unwrap();
    fs::write(game.path.join("half.bin"), b"partial").unwrap();
    let reopened = Store::new(&store.root);
    assert!(
        reopened
            .restore(&game, &game.path, &m.id, None, true, &c)
            .is_err()
    );
    reopened.recover(&game, &game.path, None, &c).unwrap();
    assert_eq!(
        fs::read(game.path.join("data.bin")).unwrap(),
        vec![7; 150_000]
    );
    assert!(!game.path.join("half.bin").exists());
    assert!(!path.exists());
}
#[test]
fn cancelled_backup_never_becomes_visible() {
    let (_tmp, game, store) = fixture();
    let c = Control::default();
    c.cancel();
    assert!(store.create(&game, &game.path, false, &c).is_err());
    assert!(store.list(&game).unwrap().is_empty());
}
#[test]
fn archive_round_trip_and_truncation_do_not_publish_partial_files() {
    let (tmp, game, store) = fixture();
    let c = Control::default();
    let m = store.create(&game, &game.path, false, &c).unwrap();
    let mut bytes = Vec::new();
    archive::export(&store, &m.id, &mut bytes, &c).unwrap();
    let remote = Store::new(tmp.path().join("download"));
    assert!(
        archive::import(
            &remote,
            &m.id,
            &game.title_id,
            &game.save_id,
            Cursor::new(&bytes[..bytes.len() - 1]),
            &c
        )
        .is_err()
    );
    assert!(!remote.path(&m.id).unwrap().exists());
    archive::import(
        &remote,
        &m.id,
        &game.title_id,
        &game.save_id,
        Cursor::new(&bytes),
        &c,
    )
    .unwrap();
    assert_eq!(remote.verify(&m.id, &c).unwrap(), m);
    assert!(
        archive::import(
            &remote,
            &m.id,
            &game.title_id,
            &game.save_id,
            Cursor::new(bytes),
            &c
        )
        .is_err()
    );
}
#[test]
fn archive_rejects_path_traversal_case_collisions_and_protected_metadata() {
    let (_tmp, game, store) = fixture();
    let c = Control::default();
    let mut m = store.create(&game, &game.path, false, &c).unwrap();
    for path in [
        "../escape",
        "/absolute",
        "C:/outside",
        "foo\\bar",
        "sce_sys/keystone",
        "sce_pfs/data",
        "file.",
        "dir//file",
    ] {
        m.files[0].path = path.into();
        assert!(m.validate().is_err(), "{path}");
    }
    m.files[0].path = "DATA.BIN".into();
    let mut other = m.files[0].clone();
    other.path = "data.bin".into();
    m.files.push(other);
    assert!(m.validate().is_err());
}
#[cfg(unix)]
#[test]
fn symlinks_are_not_followed_for_backup_or_restore() {
    use std::os::unix::fs::symlink;
    let (tmp, game, store) = fixture();
    let c = Control::default();
    let m = store.create(&game, &game.path, false, &c).unwrap();
    let outside = tmp.path().join("outside");
    fs::write(&outside, b"untouched").unwrap();
    fs::remove_file(game.path.join("data.bin")).unwrap();
    symlink(&outside, game.path.join("data.bin")).unwrap();
    assert!(store.create(&game, &game.path, false, &c).is_err());
    assert!(
        store
            .restore(&game, &game.path, &m.id, None, true, &c)
            .is_err()
    );
    assert_eq!(fs::read(outside).unwrap(), b"untouched");
}
#[test]
fn wrong_save_id_is_never_restored() {
    let (_tmp, game, store) = fixture();
    let c = Control::default();
    let m = store.create(&game, &game.path, false, &c).unwrap();
    let mut other = game.clone();
    other.save_id = "OTHER001".into();
    assert!(
        store
            .restore(&other, &game.path, &m.id, None, true, &c)
            .is_err()
    );
    assert_eq!(store.list(&game).unwrap().len(), 1);
}
#[test]
fn malformed_sfo_offsets_are_rejected_without_panicking() {
    for len in 0..128 {
        assert!(vita_save::saves::sfo::entries(&vec![0xff; len]).is_err());
    }
    let mut data = vec![0; 64];
    data[..4].copy_from_slice(b"\0PSF");
    data[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(vita_save::saves::sfo::entries(&data).is_err());
}

#[test]
fn incompatible_case_or_type_is_rejected_before_live_writes_or_journal() {
    for case_only in [false, true] {
        let (_tmp, game, store) = fixture();
        let control = Control::default();
        let snapshot = store.create(&game, &game.path, false, &control).unwrap();
        if case_only {
            fs::rename(game.path.join("data.bin"), game.path.join("DATA.BIN")).unwrap();
            fs::write(game.path.join("DATA.BIN"), b"keep-current").unwrap();
        } else {
            fs::remove_file(game.path.join("data.bin")).unwrap();
            fs::create_dir(game.path.join("data.bin")).unwrap();
            fs::write(game.path.join("data.bin/child"), b"keep-current").unwrap();
        }
        assert!(
            store
                .restore(&game, &game.path, &snapshot.id, None, true, &control)
                .is_err()
        );
        assert!(store.pending(&game).unwrap().is_none());
        assert_eq!(store.list(&game).unwrap().len(), 1);
        assert_eq!(
            fs::read(game.path.join(if case_only {
                "DATA.BIN"
            } else {
                "data.bin/child"
            }))
            .unwrap(),
            b"keep-current"
        );
    }
}
