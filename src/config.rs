use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub language: String,
    pub light_theme: bool,
    pub animations: bool,
    pub backup_before_restore: bool,
    pub webdav_url: String,
    pub webdav_user: String,
    pub webdav_password: String,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            language: "zh".into(),
            light_theme: false,
            animations: true,
            backup_before_restore: true,
            webdav_url: String::new(),
            webdav_user: String::new(),
            webdav_password: String::new(),
        }
    }
}
impl Config {
    pub fn load(root: &Path) -> Result<Self> {
        let path = root.join("config.toml");
        let path = if !path.exists() && root.join("config.previous").exists() {
            root.join("config.previous")
        } else {
            path
        };
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                ensure!(text.len() <= 16 * 1024, "Config exceeds 16 KiB");
                let config: Self = toml::from_str(&text).map_err(|e: toml::de::Error| {
                    anyhow::anyhow!(
                        "Invalid config.toml near byte {}",
                        e.span().map_or(0, |r| r.start)
                    )
                })?;
                ensure!(
                    ["zh", "en"].contains(&config.language.as_str()),
                    "Unsupported language"
                );
                Ok(config)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e.into()),
        }
    }
    pub fn save(&self, root: &Path) -> Result<()> {
        std::fs::create_dir_all(root)?;
        // Keep the previous version if FAT/newlib cannot replace an existing file.
        let text = toml::to_string_pretty(self)?;
        let tmp = root.join("config.new");
        crate::backup::write_synced(&tmp, text.as_bytes())?;
        let path = root.join("config.toml");
        let old = root.join("config.previous");
        if path.exists() && old.exists() {
            std::fs::remove_file(&old)?;
        }
        if path.exists() {
            std::fs::rename(&path, &old)?;
        }
        if let Err(e) = std::fs::rename(&tmp, &path) {
            if old.exists() {
                let _ = std::fs::rename(&old, &path);
            }
            return Err(e.into());
        }
        crate::backup::sync_parent(&path)?;
        Ok(())
    }
}
