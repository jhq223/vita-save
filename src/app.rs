use crate::{
    backup::{Manifest, Recovery, Store},
    cloud::{Remote, WebDav},
    config::Config,
    job::{Control, Progress},
    platform::{self, Environment},
    saves::{self, Game},
    text_edit::{Edit, EditText},
    ui::{Action, Page, Tab, View},
};
use anyhow::{Context, Result, ensure};
use std::{collections::BTreeMap, path::PathBuf, thread::JoinHandle};

#[derive(Clone, Default)]
pub struct Detail {
    pub size: u64,
    pub backups: Vec<Manifest>,
    pub recovery: Option<Recovery>,
}
pub struct Model {
    pub config: Config,
    pub games: Vec<Game>,
    pub catalog_revision: u64,
    pub details: BTreeMap<PathBuf, Detail>,
    pub remote: BTreeMap<PathBuf, Vec<Remote>>,
    pub busy: bool,
    pub cancelling: bool,
    pub scanning: bool,
    pub progress: Progress,
    pub message: String,
    pub edit: Option<Edit>,
}
impl Model {
    pub fn detail(&self, index: usize) -> Option<&Detail> {
        self.games
            .get(index)
            .and_then(|g| self.details.get(&g.path))
    }
}
enum Outcome {
    Games(Vec<Game>),
    Detail(Game, Detail),
    Remote(Game, Vec<Remote>),
    Done,
}
struct Task {
    control: Control,
    worker: JoinHandle<Result<Outcome>>,
    dialog: bool,
    success: &'static str,
}
pub struct App {
    pub model: Model,
    pub view: View,
    env: Environment,
    task: Option<Task>,
    pub exit: bool,
}
impl App {
    pub fn new(env: Environment) -> Result<Self> {
        std::fs::create_dir_all(&env.data)?;
        let config = Config::load(&env.data)?;
        if !env.data.join("config.toml").exists() {
            config.save(&env.data)?;
        }
        let model = Model {
            config,
            games: Vec::new(),
            catalog_revision: 0,
            details: BTreeMap::new(),
            remote: BTreeMap::new(),
            busy: false,
            cancelling: false,
            scanning: false,
            progress: Progress::default(),
            message: String::new(),
            edit: None,
        };
        let view = View::new(&model)?;
        let mut app = Self {
            model,
            view,
            env,
            task: None,
            exit: false,
        };
        app.dispatch(Action::Refresh)?;
        Ok(app)
    }
    fn start(
        &mut self,
        success: Option<&'static str>,
        work: impl FnOnce(Control) -> Result<Outcome> + Send + 'static,
    ) -> Result<()> {
        ensure!(
            self.task.is_none(),
            "{}",
            self.view.text(&self.model, "busy")
        );
        let control = Control::default();
        let worker_control = control.clone();
        let worker = std::thread::Builder::new()
            .name("vita-save-worker".into())
            .stack_size(1024 * 1024)
            .spawn(move || work(worker_control))?;
        self.task = Some(Task {
            control,
            worker,
            dialog: success.is_some(),
            success: success.unwrap_or("done"),
        });
        self.model.busy = true;
        self.model.cancelling = false;
        self.model.message.clear();
        self.model.progress = Progress::default();
        self.view.rebuild(&self.model)?;
        if success.is_some() {
            self.view.start_progress(&self.model)?;
        }
        Ok(())
    }
    pub fn poll(&mut self) -> Result<bool> {
        let Some(task) = &self.task else {
            return Ok(false);
        };
        self.model.progress = task.control.progress();
        self.view.update_progress(&self.model)?;
        if !task.worker.is_finished() {
            return Ok(true);
        }
        let task = self.task.take().unwrap();
        self.model.busy = false;
        self.model.scanning = false;
        let cancelled = task.control.is_cancelled();
        self.model.cancelling = false;
        self.view.dismiss_dialog();
        let result = task
            .worker
            .join()
            .map_err(|_| {
                anyhow::anyhow!("Background task panicked; restart before another save operation")
            })
            .and_then(|result| result);
        let failed = result.is_err();
        match result {
            Ok(outcome) => {
                match outcome {
                    Outcome::Games(games) => {
                        self.model.games = games;
                        self.model.catalog_revision = self.model.catalog_revision.wrapping_add(1);
                        self.model.details.clear();
                        self.model.remote.clear();
                        if !matches!(self.view.page(), Page::Settings(_)) {
                            self.view.home(&self.model)?;
                        }
                    }
                    Outcome::Detail(game, detail) => {
                        self.model.details.insert(game.path, detail);
                    }
                    Outcome::Remote(game, versions) => {
                        self.model.remote.insert(game.path, versions);
                    }
                    Outcome::Done => {}
                }
                self.model.message = self.view.text(&self.model, task.success);
            }
            Err(error) => {
                self.model.message = if cancelled {
                    format!(
                        "{}\n{error:#}",
                        self.view.text(&self.model, "cancel_complete")
                    )
                } else {
                    format!("{}\n{error:#}", self.view.text(&self.model, "error"))
                };
                // Recovery state must be refreshed even if a restore failed mid-write.
                let store = Store::new(&self.env.data);
                for (path, detail) in &mut self.model.details {
                    if let Some(game) = self.model.games.iter().find(|g| &g.path == path) {
                        detail.recovery = store.pending(game)?.map(|journal| journal.recovery());
                        detail.backups = store.list(game)?;
                    }
                }
            }
        }
        self.view.rebuild(&self.model)?;
        if task.dialog || failed {
            self.view.result(&self.model)?;
        }
        Ok(true)
    }
    fn game(&self, index: usize) -> Result<Game> {
        self.model
            .games
            .get(index)
            .cloned()
            .context("Game no longer exists")
    }
    pub fn dispatch(&mut self, action: Action) -> Result<()> {
        // The native IME owns navigation until it closes.
        if self.model.edit.is_some() {
            return Ok(());
        }
        let store = Store::new(&self.env.data);
        match action {
            Action::Refresh => {
                if self.view.page() != &Page::Library || self.model.busy {
                    return Ok(());
                }
                let env = self.env.clone();
                self.model.scanning = true;
                if let Err(error) = self.start(None, move |c| {
                    Ok(Outcome::Games(saves::scan(
                        &env.save_roots,
                        env.app_db.as_deref(),
                        &c,
                    )?))
                }) {
                    self.model.scanning = false;
                    return Err(error);
                }
            }
            Action::OpenGame(i) => {
                ensure!(!self.model.busy, "{}", self.view.text(&self.model, "busy"));
                let game = self.game(i)?;
                self.view.open(&self.model, Page::Game(i))?;
                if !self.model.busy {
                    self.start(None, move |c| {
                        Ok(Outcome::Detail(
                            game.clone(),
                            read_detail(&store, &game, &c)?,
                        ))
                    })?;
                }
            }
            Action::Backups(i) => self.view.open(&self.model, Page::Backups(i))?,
            Action::Snapshot(i, id) => self.view.open(&self.model, Page::Snapshot(i, id))?,
            Action::AskBackup(i) => {
                self.view
                    .confirm(&self.model, "backup_confirm", Action::Backup(i))?
            }
            Action::AskDelete(i, id) => {
                let manifest = self
                    .model
                    .detail(i)
                    .and_then(|detail| detail.backups.iter().find(|m| m.id == id))
                    .context("Backup no longer exists")?;
                let message = format!(
                    "{}\n{}",
                    platform::time::stamp(manifest.created),
                    self.view.text(&self.model, "delete_confirm")
                );
                self.view
                    .confirm_message(&self.model, message, Action::Delete(i, id))?;
            }
            Action::DeleteSelected => {
                if let Some((i, id)) = self.view.selected_backup(&self.model) {
                    self.dispatch(Action::AskDelete(i, id))?;
                }
            }
            Action::Delete(i, id) => {
                let game = self.game(i)?;
                let size = self.model.detail(i).map_or(0, |detail| detail.size);
                self.view.open(&self.model, Page::Backups(i))?;
                self.start(Some("deleted"), move |c| {
                    store.delete(&game, &id, &c)?;
                    // Deleting a local version does not need to mount the live save.
                    // Once deletion commits, a late cancel must not report it undone.
                    let detail = Detail {
                        size,
                        backups: store.list(&game)?,
                        recovery: store.pending(&game)?.map(|journal| journal.recovery()),
                    };
                    Ok(Outcome::Detail(game, detail))
                })?;
            }
            Action::Backup(i) => {
                let game = self.game(i)?;
                self.start(Some("backup_done"), move |c| {
                    platform::with_save(&game, |path, _| {
                        store.create(&game, path, false, &c)?;
                        Ok(())
                    })?;
                    Ok(Outcome::Detail(
                        game.clone(),
                        read_detail(&store, &game, &c)?,
                    ))
                })?;
            }
            Action::AskRestore(i, id) => {
                let key = if self.model.config.backup_before_restore {
                    "restore_confirm"
                } else {
                    "restore_confirm_no_backup"
                };
                self.view
                    .confirm(&self.model, key, Action::Restore(i, id))?
            }
            Action::Restore(i, id) => {
                let game = self.game(i)?;
                let backup_before_restore = self.model.config.backup_before_restore;
                self.start(Some("done"), move |c| {
                    platform::with_save(&game, |path, account| {
                        store.restore(&game, path, &id, account, backup_before_restore, &c)?;
                        Ok(())
                    })?;
                    Ok(Outcome::Detail(
                        game.clone(),
                        read_detail(&store, &game, &c)?,
                    ))
                })?;
            }
            Action::Recover(i) => {
                let game = self.game(i)?;
                self.start(Some("done"), move |c| {
                    platform::with_save(&game, |path, account| {
                        store.recover(&game, path, account, &c)
                    })?;
                    Ok(Outcome::Detail(
                        game.clone(),
                        read_detail(&store, &game, &c)?,
                    ))
                })?;
            }
            Action::Cloud(i) => {
                ensure!(!self.model.busy, "{}", self.view.text(&self.model, "busy"));
                let game = self.game(i)?;
                let cloud = WebDav::new(&self.model.config)?;
                self.view.open(&self.model, Page::Cloud(i))?;
                self.start(None, move |c| {
                    Ok(Outcome::Remote(game.clone(), cloud.list(&game, &c)?))
                })?;
            }
            Action::Upload(i, id) => {
                let game = self.game(i)?;
                let cloud = WebDav::new(&self.model.config)?;
                self.start(Some("uploaded"), move |c| {
                    cloud.upload(&store, &game, &id, &c)?;
                    Ok(Outcome::Done)
                })?;
            }
            Action::Download(i, id) => {
                let game = self.game(i)?;
                let cloud = WebDav::new(&self.model.config)?;
                self.start(Some("downloaded"), move |c| {
                    cloud.download(&store, &game, &id, &c)?;
                    Ok(Outcome::Detail(
                        game.clone(),
                        read_detail(&store, &game, &c)?,
                    ))
                })?;
            }
            Action::TestConnection => {
                let cloud = WebDav::new(&self.model.config)?;
                self.start(Some("connected"), move |c| {
                    cloud.test(&c)?;
                    Ok(Outcome::Done)
                })?;
            }
            Action::Cancel => {
                if let Some(task) = &self.task {
                    task.control.cancel();
                    self.model.cancelling = true;
                    self.view.update_progress(&self.model)?;
                }
            }
            Action::Settings => self
                .view
                .open(&self.model, Page::Settings(Tab::Interface))?,
            Action::SettingsTab(tab) => self.view.replace(&self.model, Page::Settings(tab))?,
            Action::Language | Action::Theme => self.view.dropdown(&self.model, action)?,
            Action::SetLanguage(code) => {
                let mut next = self.model.config.clone();
                next.language = code.into();
                self.save_config(next)?;
            }
            Action::SetTheme(light) => {
                let mut next = self.model.config.clone();
                next.light_theme = light;
                self.save_config(next)?;
            }
            Action::Animations => {
                let mut next = self.model.config.clone();
                next.animations = !next.animations;
                self.save_config(next)?;
            }
            Action::BackupBeforeRestore => {
                let mut next = self.model.config.clone();
                next.backup_before_restore = !next.backup_before_restore;
                self.save_config(next)?;
            }
            Action::EditWebDav(field) => {
                ensure!(!self.model.busy, "{}", self.view.text(&self.model, "busy"));
                let edit = Edit::new(field, &self.model.config)?;
                self.view.open(&self.model, Page::Settings(Tab::WebDav))?;
                self.model.edit = Some(edit);
                self.view.open(&self.model, Page::Edit(field))?;
            }
            Action::Back => {
                self.view.back(&self.model)?;
            }
            Action::PageUp => self.view.page_step(-4),
            Action::PageDown => self.view.page_step(4),
            Action::AskExit => {
                ensure!(!self.model.busy, "{}", self.view.text(&self.model, "busy"));
                self.view.confirm(&self.model, "exit", Action::Exit)?;
            }
            Action::Exit => {
                ensure!(!self.model.busy, "Task still running");
                self.exit = true;
            }
            Action::Close => self.view.dismiss_dialog(),
        }
        Ok(())
    }
    pub fn update_edit(&mut self, text: EditText) -> Result<()> {
        if let Some(edit) = &mut self.model.edit {
            edit.text = text;
            self.view.update_editor(&self.model)?;
        }
        Ok(())
    }
    pub fn finish_edit(&mut self, value: Option<String>) -> Result<()> {
        let Some(edit) = self.model.edit.take() else {
            return Ok(());
        };
        self.view.back(&self.model)?;
        if let Some(value) = value {
            let next = edit.field.apply(&self.model.config, value)?;
            self.save_config(next)?;
        }
        Ok(())
    }
    fn save_config(&mut self, config: Config) -> Result<()> {
        config.save(&self.env.data)?;
        self.model.config = config;
        self.view.rebuild(&self.model)?;
        Ok(())
    }
    pub fn error(&mut self, error: anyhow::Error) -> Result<()> {
        self.view.error(&self.model, &format!("{error:#}"))?;
        Ok(())
    }
}
impl Drop for App {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.control.cancel();
            let _ = task.worker.join();
        }
    }
}
fn read_detail(store: &Store, game: &Game, control: &Control) -> Result<Detail> {
    let backups = store.list(game)?;
    let recovery = store.pending(game)?.map(|journal| journal.recovery());
    let size = platform::with_save(game, |path, _| {
        Ok(crate::backup::inventory(path, control)?
            .1
            .iter()
            .map(|(_, s)| s)
            .sum())
    })?;
    Ok(Detail {
        size,
        backups,
        recovery,
    })
}
