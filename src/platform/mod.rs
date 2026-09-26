use crate::saves::Game;
use anyhow::Result;
use std::path::{Path, PathBuf};
#[cfg(target_os = "vita")]
mod ime;
#[cfg(target_os = "vita")]
mod input;
#[cfg(target_os = "vita")]
mod mount;
#[cfg(target_os = "vita")]
mod pvr;
#[cfg(target_os = "vita")]
pub(crate) mod sqlite;
pub mod time;
#[cfg(target_os = "vita")]
mod video;
#[cfg(target_os = "vita")]
pub use video::run;

#[derive(Clone)]
pub struct Environment {
    pub data: PathBuf,
    pub save_roots: Vec<PathBuf>,
    pub app_db: Option<PathBuf>,
}
impl Environment {
    pub fn vita() -> Self {
        Self {
            data: "ux0:data/vita-save".into(),
            save_roots: vec!["ux0:user/00/savedata".into(), "grw0:savedata".into()],
            app_db: Some("ur0:shell/db/app.db".into()),
        }
    }
    pub fn host(root: &Path) -> Self {
        Self {
            data: root.join("vita-save"),
            save_roots: vec![root.join("savedata")],
            app_db: None,
        }
    }
}

pub fn with_save<T>(game: &Game, work: impl FnOnce(&Path, Option<u64>) -> Result<T>) -> Result<T> {
    #[cfg(target_os = "vita")]
    {
        mount::with_save(game, work)
    }
    #[cfg(not(target_os = "vita"))]
    {
        anyhow::ensure!(
            !game.path.join("sce_pfs").exists(),
            "Encrypted saves require Vita hardware"
        );
        work(&game.path, None)
    }
}
