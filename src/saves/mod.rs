pub mod sfo;
use crate::job::Control;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Game {
    pub title_id: String,
    pub save_id: String,
    pub name: String,
    pub path: PathBuf,
    pub icon: Option<PathBuf>,
}

/// Read an in-memory, read-only app.db snapshot; SQLite never writes to the shell database.
pub type TitleMap = BTreeMap<String, (String, String, Option<PathBuf>)>;
pub fn app_database(path: &Path) -> Result<TitleMap> {
    for suffix in ["-wal", "-journal"] {
        let sidecar = PathBuf::from(format!("{}{suffix}", path.display()));
        ensure!(
            !sidecar.exists() || fs::metadata(sidecar)?.len() == 0,
            "Shell database is busy; refresh after LiveArea finishes updating"
        );
    }
    let mut file = fs::File::open(path)?;
    let len = file.metadata()?.len();
    ensure!(len <= 64 * 1024 * 1024, "app.db exceeds 64 MiB");
    #[cfg(not(target_os = "vita"))]
    let mut db = rusqlite::Connection::open_in_memory()?;
    #[cfg(target_os = "vita")]
    let mut db = crate::platform::sqlite::memory()?;
    db.deserialize_read_exact(rusqlite::MAIN_DB, &mut file, len as usize, true)?;
    let check: String = db.query_row("PRAGMA quick_check", [], |row| row.get(0))?;
    ensure!(check == "ok", "Inconsistent app.db: {check}");
    let mut query = db.prepare("SELECT a.titleid, a.val, i.title, i.iconpath FROM tbl_appinfo a LEFT JOIN tbl_appinfo_icon i ON i.titleid=a.titleid AND i.type=0 WHERE a.key=278217076 ORDER BY a.titleid")?;
    let mut rows = query.query([])?;
    let mut result = BTreeMap::new();
    while let Some(row) = rows.next()? {
        let title: String = row.get(0)?;
        let save: String = row.get(1)?;
        if crate::backup::valid_component(&title) && crate::backup::valid_component(&save) {
            let name = row
                .get::<_, Option<String>>(2)?
                .unwrap_or_else(|| title.clone())
                .replace(['\n', '\r'], " ");
            let icon = row.get::<_, Option<String>>(3)?.map(PathBuf::from);
            result.entry(save).or_insert((title, name, icon));
        }
    }
    Ok(result)
}

pub fn scan(roots: &[PathBuf], database: Option<&Path>, control: &Control) -> Result<Vec<Game>> {
    control.set("scan", "", 0, 0);
    let titles = if let Some(path) = database {
        app_database(path).context("Read game metadata")?
    } else {
        BTreeMap::new()
    };
    let mut result = Vec::new();
    for root in roots {
        if !root.exists() {
            continue;
        }
        for entry in fs::read_dir(root).with_context(|| format!("Read {}", root.display()))? {
            control.check()?;
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let save_id = entry
                .file_name()
                .to_str()
                .context("Non-UTF8 save directory")?
                .to_owned();
            if !crate::backup::valid_component(&save_id) {
                continue;
            }
            let path = entry.path();
            let (title_id, name, icon) = titles.get(&save_id).cloned().unwrap_or_else(|| {
                let sfo = fs::read(path.join("sce_sys/param.sfo")).unwrap_or_default();
                let title = sfo::text(&sfo, "TITLE_ID")
                    .filter(|v| crate::backup::valid_component(v))
                    .unwrap_or_else(|| save_id.clone());
                let name = sfo::text(&sfo, "TITLE").unwrap_or_else(|| save_id.clone());
                (title, name, None)
            });
            result.push(Game {
                title_id,
                save_id,
                name,
                path,
                icon,
            });
        }
    }
    result.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then(a.path.cmp(&b.path))
    });
    Ok(result)
}
