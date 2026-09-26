//! Bounded copies and immutable snapshots. No writes to a live save without a recovery journal.
use crate::{job::Control, saves::Game};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

pub const MANIFEST_LIMIT: u64 = 8 * 1024 * 1024;
pub const MAX_BYTES: u64 = 32 * 1024 * 1024 * 1024;
const MAX_ENTRIES: usize = 50_000;
const BUFFER: usize = 64 * 1024;
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Entry {
    pub path: String,
    pub size: u64,
    pub sha256: [u8; 32],
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Manifest {
    pub version: u32,
    pub id: String,
    pub title_id: String,
    pub save_id: String,
    pub title: String,
    pub created: u64,
    pub automatic: bool,
    pub directories: Vec<String>,
    pub files: Vec<Entry>,
}
impl Manifest {
    pub fn bytes(&self) -> u64 {
        self.files.iter().map(|f| f.size).sum()
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.version == 1,
            "Unsupported snapshot version {}",
            self.version
        );
        ensure!(
            valid_component(&self.id)
                && valid_component(&self.title_id)
                && valid_component(&self.save_id),
            "Invalid snapshot identity"
        );
        ensure!(
            self.files.len() + self.directories.len() <= MAX_ENTRIES,
            "Too many snapshot entries"
        );
        let mut names = BTreeSet::new();
        let dirs: BTreeSet<_> = self.directories.iter().map(|d| d.to_lowercase()).collect();
        let mut total = 0u64;
        for (name, size) in self
            .directories
            .iter()
            .map(|s| (s, 0))
            .chain(self.files.iter().map(|f| (&f.path, f.size)))
        {
            ensure!(
                valid_relative(name) && !protected(name),
                "Unsafe snapshot path: {name}"
            );
            ensure!(
                names.insert(name.to_lowercase()),
                "Duplicate or case-colliding snapshot path: {name}"
            );
            if let Some((parent, _)) = name.rsplit_once('/') {
                ensure!(
                    dirs.contains(&parent.to_lowercase()),
                    "Missing parent directory: {name}"
                );
            }
            total = total.checked_add(size).context("Snapshot size overflow")?;
        }
        ensure!(total <= MAX_BYTES, "Snapshot exceeds 32 GiB");
        Ok(())
    }
}

#[derive(Clone)]
pub struct Store {
    pub root: PathBuf,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Journal {
    pub version: u32,
    pub game: Game,
    pub rollback: Option<String>,
    pub requested: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Recovery {
    Rollback,
    Retry,
}
impl Journal {
    pub fn recovery(&self) -> Recovery {
        if self.rollback.is_some() {
            Recovery::Rollback
        } else {
            Recovery::Retry
        }
    }
}
impl Store {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }
    pub fn path(&self, id: &str) -> Result<PathBuf> {
        ensure!(valid_component(id), "Invalid snapshot ID");
        Ok(self.root.join("snapshots").join(id))
    }
    pub fn read(&self, id: &str) -> Result<Manifest> {
        let path = self.path(id)?;
        safe_directory(&path)?;
        let file = checked_file(&path.join("manifest.bin"))?;
        let manifest: Manifest = decode(file, MANIFEST_LIMIT)?;
        manifest.validate()?;
        ensure!(manifest.id == id, "Snapshot ID mismatch");
        Ok(manifest)
    }
    pub fn list(&self, game: &Game) -> Result<Vec<Manifest>> {
        let root = self.root.join("snapshots");
        if !root.exists() {
            return Ok(Vec::new());
        }
        let mut manifests = Vec::new();
        for entry in fs::read_dir(root)? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            // An unreadable completed snapshot is an error, never silently shown as an empty list.
            let id = entry.file_name().to_string_lossy().into_owned();
            let m = self
                .read(&id)
                .with_context(|| format!("Read snapshot {id}"))?;
            if m.title_id == game.title_id && m.save_id == game.save_id {
                manifests.push(m);
            }
        }
        manifests.sort_by(|a, b| b.created.cmp(&a.created).then(b.id.cmp(&a.id)));
        Ok(manifests)
    }
    pub fn create(
        &self,
        game: &Game,
        mounted: &Path,
        automatic: bool,
        control: &Control,
    ) -> Result<Manifest> {
        let (directories, files) = inventory(mounted, control)?;
        let total: u64 = files.iter().map(|(_, s)| *s).sum();
        let id = new_id()?;
        let mut stage = self.stage(&id)?;
        let mut manifest = Manifest {
            version: 1,
            id: id.clone(),
            title_id: game.title_id.clone(),
            save_id: game.save_id.clone(),
            title: game.name.clone(),
            created: timestamp(),
            automatic,
            directories,
            files: Vec::new(),
        };
        for dir in &manifest.directories {
            fs::create_dir_all(stage.path.join("files").join(dir))?;
        }
        let mut done = 0;
        for (path, size) in files {
            let hash = copy_hash(
                &mounted.join(&path),
                &stage.path.join("files").join(&path),
                size,
                control,
                "backup",
                &path,
                &mut done,
                total,
            )?;
            manifest.files.push(Entry {
                path,
                size,
                sha256: hash,
            });
        }
        // Detect concurrent additions/removals and size changes before publishing a snapshot.
        let after = inventory(mounted, control)?;
        ensure!(
            after.0 == manifest.directories
                && after.1
                    == manifest
                        .files
                        .iter()
                        .map(|f| (f.path.clone(), f.size))
                        .collect::<Vec<_>>(),
            "Save changed during backup; close the game and retry"
        );
        manifest.validate()?;
        write_synced(
            &stage.path.join("manifest.bin"),
            &postcard::to_stdvec(&manifest)?,
        )?;
        self.commit(&mut stage, &manifest)?;
        Ok(manifest)
    }
    pub fn delete(&self, game: &Game, id: &str, control: &Control) -> Result<()> {
        control.check()?;
        let manifest = self.read(id)?;
        ensure!(
            manifest.title_id == game.title_id && manifest.save_id == game.save_id,
            "Snapshot belongs to a different game/save directory"
        );
        // Inspect all journals: two catalog entries may refer to the same save.
        let recovery = self.root.join("recovery");
        if recovery.exists() {
            safe_directory(&recovery)?;
            for entry in fs::read_dir(&recovery)? {
                let journal: Journal = decode(checked_file(&entry?.path())?, 64 * 1024)?;
                ensure!(journal.version == 2, "Unsupported recovery journal");
                ensure!(
                    journal.rollback.as_deref() != Some(id) && journal.requested != id,
                    "This backup is needed by an interrupted restore; recover the save first"
                );
            }
        }
        let source = self.path(id)?;
        safe_directory(&source)?;
        let trash = self.root.join("trash");
        fs::create_dir_all(&trash)?;
        safe_directory(&trash)?;
        let destination = trash.join(new_id()?);
        control.check()?;
        control.set("delete", "", 0, 0);
        // Remove the complete snapshot from the list before deleting any payload.
        // Cancellation is accepted before this commit, never halfway through it.
        fs::rename(&source, &destination)?;
        sync_parent(&source)?;
        sync_parent(&destination)?;
        fs::remove_dir_all(&destination)?;
        sync_parent(&destination)?;
        Ok(())
    }
    pub fn verify(&self, id: &str, control: &Control) -> Result<Manifest> {
        let manifest = self.read(id)?;
        verify_files(&self.path(id)?.join("files"), &manifest, control)?;
        Ok(manifest)
    }
    pub fn journal_path(&self, game: &Game) -> PathBuf {
        let digest = Sha256::digest(game.path.to_string_lossy().as_bytes());
        let name: String = digest.iter().map(|b| format!("{b:02x}")).collect();
        self.root.join("recovery").join(format!("{name}.bin"))
    }
    pub fn pending(&self, game: &Game) -> Result<Option<Journal>> {
        let path = self.journal_path(game);
        if !path.exists() {
            return Ok(None);
        }
        let journal: Journal = decode(checked_file(&path)?, 64 * 1024)?;
        ensure!(
            journal.version == 2
                && journal.game.path == game.path
                && journal.game.save_id == game.save_id
                && journal.game.title_id == game.title_id,
            "Recovery journal does not match this save"
        );
        Ok(Some(journal))
    }
    pub fn restore(
        &self,
        game: &Game,
        mounted: &Path,
        id: &str,
        account: Option<u64>,
        backup_before_restore: bool,
        control: &Control,
    ) -> Result<Option<String>> {
        ensure!(
            self.pending(game)?.is_none(),
            "An interrupted restore must be recovered first"
        );
        let manifest = self.verify(id, control)?;
        ensure!(
            manifest.title_id == game.title_id && manifest.save_id == game.save_id,
            "Snapshot belongs to a different game/save directory"
        );
        compatible_layout(&inventory(mounted, control)?, &manifest)?;
        // Validate account metadata before touching the destination.
        let adjusted = adjusted_sfo(&self.path(id)?.join("files"), &manifest, account)?;
        let rollback = if backup_before_restore {
            let snapshot = self.create(game, mounted, true, control)?;
            self.verify(&snapshot.id, control)?;
            Some(snapshot.id)
        } else {
            None
        };
        control.check()?;
        let journal = Journal {
            version: 2,
            game: game.clone(),
            rollback: rollback.clone(),
            requested: id.into(),
        };
        let journal_path = self.journal_path(game);
        fs::create_dir_all(journal_path.parent().unwrap())?;
        write_synced(&journal_path, &postcard::to_stdvec(&journal)?)?;
        sync_parent(&journal_path)?;
        apply_files(
            &self.path(id)?.join("files"),
            mounted,
            &manifest,
            adjusted.as_deref(),
            control,
        )
        .context(if backup_before_restore {
            "Restore interrupted. Recover the previous save from Local backups"
        } else {
            "Restore interrupted. Retry the restore from Local backups; no previous save was backed up"
        })?;
        fs::remove_file(&journal_path)?;
        sync_parent(&journal_path)?;
        Ok(rollback)
    }
    pub fn recover(
        &self,
        game: &Game,
        mounted: &Path,
        account: Option<u64>,
        control: &Control,
    ) -> Result<()> {
        let journal = self.pending(game)?.context("No pending recovery")?;
        let id = journal.rollback.as_deref().unwrap_or(&journal.requested);
        let manifest = self.verify(id, control)?;
        ensure!(
            manifest.title_id == game.title_id && manifest.save_id == game.save_id,
            "Recovery snapshot mismatch"
        );
        // A retry uses the requested snapshot and needs the same account adjustment
        // as the original restore. A rollback preserves the prior save exactly.
        let adjusted = if journal.rollback.is_none() {
            adjusted_sfo(&self.path(id)?.join("files"), &manifest, account)?
        } else {
            None
        };
        apply_files(
            &self.path(&manifest.id)?.join("files"),
            mounted,
            &manifest,
            adjusted.as_deref(),
            control,
        )?;
        let journal_path = self.journal_path(game);
        fs::remove_file(&journal_path)?;
        sync_parent(&journal_path)?;
        Ok(())
    }
    pub fn stage(&self, id: &str) -> Result<Stage> {
        ensure!(valid_component(id), "Invalid staging ID");
        let parent = self.root.join("staging");
        fs::create_dir_all(&parent)?;
        safe_directory(&parent)?;
        let path = parent.join(format!("{id}-{}", new_id()?));
        fs::create_dir(&path)?;
        fs::create_dir(path.join("files"))?;
        Ok(Stage {
            path,
            committed: false,
        })
    }
    pub fn commit(&self, stage: &mut Stage, manifest: &Manifest) -> Result<()> {
        let destination = self.path(&manifest.id)?;
        fs::create_dir_all(destination.parent().unwrap())?;
        ensure!(
            !destination.exists(),
            "Snapshot already exists: {}",
            manifest.id
        );
        fs::rename(&stage.path, &destination)?;
        sync_parent(&destination)?;
        stage.committed = true;
        Ok(())
    }
}

pub struct Stage {
    pub path: PathBuf,
    committed: bool,
}
impl Drop for Stage {
    fn drop(&mut self) {
        // Only this task's exclusively created staging directory is removed.
        if !self.committed {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}
pub fn valid_component(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s != "."
        && s != ".."
        && !s.ends_with(['.', ' '])
        && !s
            .chars()
            .any(|c| c.is_control() || "\\/:*?\"<>|".contains(c))
        && ![
            "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7",
            "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
        ]
        .contains(
            &s.split('.')
                .next()
                .unwrap_or("")
                .to_ascii_uppercase()
                .as_str(),
        )
}
pub fn valid_relative(s: &str) -> bool {
    s.len() <= 240 && s.split('/').all(valid_component)
}
pub fn protected(path: &str) -> bool {
    let path = path.to_ascii_lowercase();
    path == "sce_pfs"
        || path.starts_with("sce_pfs/")
        || [
            "sce_sys/keystone",
            "sce_sys/sealedkey",
            "sce_sys/safemem.dat",
        ]
        .contains(&path.as_str())
}
pub fn safe_directory(path: &Path) -> Result<()> {
    // Check ancestors too; a valid leaf under a symlink is still outside the intended tree.
    for ancestor in path.ancestors().filter(|p| !p.as_os_str().is_empty()) {
        let meta = fs::symlink_metadata(ancestor)
            .with_context(|| format!("Read {}", ancestor.display()))?;
        ensure!(
            meta.is_dir() && !meta.file_type().is_symlink(),
            "Not a regular directory: {}",
            ancestor.display()
        );
    }
    Ok(())
}
pub fn checked_file(path: &Path) -> Result<File> {
    safe_directory(path.parent().context("File without parent")?)?;
    let meta = fs::symlink_metadata(path)?;
    ensure!(
        meta.is_file() && !meta.file_type().is_symlink(),
        "Not a regular file: {}",
        path.display()
    );
    Ok(File::open(path)?)
}
pub fn decode<T: for<'a> Deserialize<'a>>(mut reader: impl Read, limit: u64) -> Result<T> {
    let mut data = Vec::new();
    (&mut reader).take(limit + 1).read_to_end(&mut data)?;
    ensure!(data.len() as u64 <= limit, "Metadata exceeds size limit");
    let (value, rest) = postcard::take_from_bytes(&data)?;
    ensure!(rest.is_empty(), "Trailing metadata bytes");
    Ok(value)
}
pub fn write_synced(path: &Path, bytes: &[u8]) -> Result<()> {
    if path.exists() {
        ensure!(
            fs::symlink_metadata(path)?.is_file()
                && !fs::symlink_metadata(path)?.file_type().is_symlink(),
            "Unsafe output path"
        );
    }
    let mut file = File::create(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}
pub fn timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
pub fn new_id() -> Result<String> {
    let mut bytes = [0u8; 16];
    getrandom::getrandom(&mut bytes).map_err(|e| anyhow::anyhow!("Random source: {e}"))?;
    Ok(format!(
        "{:010}-{}",
        timestamp(),
        bytes.iter().map(|b| format!("{b:02x}")).collect::<String>()
    ))
}

pub type Inventory = (Vec<String>, Vec<(String, u64)>);
pub fn inventory(root: &Path, control: &Control) -> Result<Inventory> {
    safe_directory(root)?;
    let mut dirs = Vec::new();
    let mut files = Vec::new();
    let mut pending = vec![String::new()];
    let mut total = 0u64;
    while let Some(parent) = pending.pop() {
        control.check()?;
        for entry in fs::read_dir(root.join(&parent))? {
            let entry = entry?;
            let name = entry
                .file_name()
                .to_str()
                .context("Non-UTF8 save filename")?
                .to_owned();
            let rel = if parent.is_empty() {
                name
            } else {
                format!("{parent}/{name}")
            };
            if protected(&rel) {
                continue;
            }
            ensure!(valid_relative(&rel), "Unsupported save path: {rel}");
            let kind = entry.file_type()?;
            if kind.is_dir() {
                pending.push(rel.clone());
                dirs.push(rel);
            } else if kind.is_file() {
                let size = entry.metadata()?.len();
                total = total.checked_add(size).context("Save size overflow")?;
                ensure!(total <= MAX_BYTES, "Save exceeds 32 GiB");
                files.push((rel, size));
            } else {
                bail!("Links and special files are not supported: {rel}");
            }
            ensure!(
                dirs.len() + files.len() <= MAX_ENTRIES,
                "Save has too many entries"
            );
        }
    }
    dirs.sort();
    files.sort();
    Ok((dirs, files))
}

#[allow(clippy::too_many_arguments)]
fn copy_hash(
    src: &Path,
    dst: &Path,
    size: u64,
    control: &Control,
    phase: &'static str,
    label: &str,
    done: &mut u64,
    total: u64,
) -> Result<[u8; 32]> {
    let mut input = checked_file(src)?;
    safe_directory(dst.parent().context("Missing parent")?)?;
    if dst.exists() {
        ensure!(
            fs::symlink_metadata(dst)?.is_file()
                && !fs::symlink_metadata(dst)?.file_type().is_symlink(),
            "Unsafe destination: {}",
            dst.display()
        );
    }
    let mut output = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(dst)?;
    let mut hash = Sha256::new();
    let mut buf = vec![0; BUFFER];
    let mut copied = 0;
    loop {
        control.check()?;
        let n = input.read(&mut buf)?;
        if n == 0 {
            break;
        }
        ensure!(copied + n as u64 <= size, "File grew during copy: {label}");
        output.write_all(&buf[..n])?;
        hash.update(&buf[..n]);
        copied += n as u64;
        *done += n as u64;
        control.set(phase, label, *done, total);
    }
    ensure!(copied == size, "File shrank during copy: {label}");
    output.sync_all()?;
    Ok(hash.finalize().into())
}

pub fn verify_files(root: &Path, manifest: &Manifest, control: &Control) -> Result<()> {
    let actual = inventory(root, control)?;
    ensure!(
        actual.0 == manifest.directories
            && actual.1
                == manifest
                    .files
                    .iter()
                    .map(|f| (f.path.clone(), f.size))
                    .collect::<Vec<_>>(),
        "Snapshot file inventory mismatch"
    );
    let mut done = 0;
    let mut buffer = vec![0; BUFFER];
    for entry in &manifest.files {
        let mut file = checked_file(&root.join(&entry.path))?;
        let mut hash = Sha256::new();
        loop {
            control.check()?;
            let n = file.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            hash.update(&buffer[..n]);
            done += n as u64;
            control.set("verify", &entry.path, done, manifest.bytes());
        }
        ensure!(
            <[u8; 32]>::from(hash.finalize()) == entry.sha256,
            "Checksum mismatch: {}",
            entry.path
        );
    }
    Ok(())
}
fn adjusted_sfo(root: &Path, manifest: &Manifest, account: Option<u64>) -> Result<Option<Vec<u8>>> {
    if let Some(account) = account {
        let entry = manifest
            .files
            .iter()
            .find(|f| f.path == "sce_sys/param.sfo")
            .context("Snapshot has no save param.sfo")?;
        ensure!(entry.size <= 1024 * 1024, "Save SFO exceeds 1 MiB");
        let mut bytes = Vec::new();
        checked_file(&root.join(&entry.path))?.read_to_end(&mut bytes)?;
        crate::saves::sfo::set_account(&mut bytes, account)?;
        Ok(Some(bytes))
    } else {
        Ok(None)
    }
}
// Reject ambiguous FAT paths before any writes. A case-only rename can otherwise
// make obsolete-file cleanup remove the newly restored file on the same device.
fn compatible_layout(current: &Inventory, manifest: &Manifest) -> Result<()> {
    let mut existing = std::collections::BTreeMap::new();
    for (path, directory) in current
        .0
        .iter()
        .map(|p| (p, true))
        .chain(current.1.iter().map(|(p, _)| (p, false)))
    {
        ensure!(
            existing
                .insert(path.to_ascii_lowercase(), (path, directory))
                .is_none(),
            "Save contains case-colliding paths"
        );
    }
    for (path, directory) in manifest
        .directories
        .iter()
        .map(|p| (p, true))
        .chain(manifest.files.iter().map(|f| (&f.path, false)))
    {
        if let Some((old_path, old_directory)) = existing.get(&path.to_ascii_lowercase()) {
            ensure!(
                *old_path == path && *old_directory == directory,
                "Restore path conflicts with existing name or type: {path}"
            );
        }
    }
    Ok(())
}

fn apply_files(
    source: &Path,
    target: &Path,
    manifest: &Manifest,
    sfo: Option<&[u8]>,
    control: &Control,
) -> Result<()> {
    let current = inventory(target, control)?;
    compatible_layout(&current, manifest)?;
    let (old_dirs, old_files) = current;
    for dir in &manifest.directories {
        control.check()?;
        fs::create_dir_all(target.join(dir))?;
        safe_directory(&target.join(dir))?;
    }
    let mut done = 0;
    for entry in &manifest.files {
        if entry.path == "sce_sys/param.sfo"
            && let Some(data) = sfo
        {
            control.check()?;
            write_synced(&target.join(&entry.path), data)?;
            ensure!(
                fs::read(target.join(&entry.path))? == data,
                "Restored SFO read-back mismatch"
            );
            continue;
        }
        let hash = copy_hash(
            &source.join(&entry.path),
            &target.join(&entry.path),
            entry.size,
            control,
            "restore",
            &entry.path,
            &mut done,
            manifest.bytes(),
        )?;
        ensure!(
            hash == entry.sha256,
            "Snapshot changed during restore: {}",
            entry.path
        );
    }
    // Remove obsolete payload files after the requested payload is present. Protected PFS metadata stays intact.
    let keep: BTreeSet<_> = manifest.files.iter().map(|f| f.path.as_str()).collect();
    for (path, _) in old_files {
        control.check()?;
        if !keep.contains(path.as_str()) {
            fs::remove_file(target.join(path))?;
        }
    }
    for dir in old_dirs.into_iter().rev() {
        if !manifest.directories.contains(&dir) {
            let path = target.join(dir);
            if fs::read_dir(&path)?.next().is_none() {
                fs::remove_dir(path)?;
            }
        }
    }
    // Verify destination bytes as well as source bytes. Account rebinding has its own expected hash.
    let mut expected = manifest.clone();
    if let Some(sfo) = sfo
        && let Some(entry) = expected
            .files
            .iter_mut()
            .find(|e| e.path == "sce_sys/param.sfo")
    {
        entry.sha256 = Sha256::digest(sfo).into();
    }
    verify_files(target, &expected, control)?;
    Ok(())
}

/// Make directory entries durable before changing a live save or reporting a committed snapshot.
pub fn sync_parent(path: &Path) -> Result<()> {
    #[cfg(not(target_os = "vita"))]
    {
        #[cfg(unix)]
        File::open(path.parent().context("Missing parent")?)?.sync_all()?;
        #[cfg(not(unix))]
        let _ = path;
    }
    #[cfg(target_os = "vita")]
    {
        let text = path.to_str().context("Invalid path")?;
        let device = text.split_once(':').context("Missing Vita device")?.0;
        let name = std::ffi::CString::new(format!("{device}:"))?;
        let code = unsafe { vitasdk_sys::sceIoSync(name.as_ptr(), 0) };
        ensure!(code >= 0, "Sync {device}: {code:#010x}");
    }
    Ok(())
}
