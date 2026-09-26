use crate::{
    backup::{self, MANIFEST_LIMIT, Manifest, Store},
    job::Control,
};
use anyhow::{Context, Result, ensure};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{Read, Write},
};
const MAGIC: &[u8; 8] = b"VTSAVE01";

pub fn export(store: &Store, id: &str, mut output: impl Write, control: &Control) -> Result<()> {
    let manifest = store.verify(id, control)?;
    let bytes = postcard::to_stdvec(&manifest)?;
    ensure!(bytes.len() as u64 <= MANIFEST_LIMIT, "Manifest too large");
    output.write_all(MAGIC)?;
    output.write_all(&(bytes.len() as u32).to_le_bytes())?;
    output.write_all(&bytes)?;
    let mut buffer = vec![0; 64 * 1024];
    let mut done = 0;
    for entry in &manifest.files {
        let mut input = backup::checked_file(&store.path(id)?.join("files").join(&entry.path))?;
        let mut hash = Sha256::new();
        let mut count = 0;
        loop {
            control.check()?;
            let n = input.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            ensure!(
                count + n as u64 <= entry.size,
                "Snapshot changed while exporting"
            );
            output.write_all(&buffer[..n])?;
            hash.update(&buffer[..n]);
            count += n as u64;
            done += n as u64;
            control.set("pack", &entry.path, done, manifest.bytes());
        }
        ensure!(
            count == entry.size && <[u8; 32]>::from(hash.finalize()) == entry.sha256,
            "Snapshot changed while exporting"
        );
    }
    Ok(())
}

pub fn import(
    store: &Store,
    expected_id: &str,
    title_id: &str,
    save_id: &str,
    mut input: impl Read,
    control: &Control,
) -> Result<Manifest> {
    let mut header = [0u8; 12];
    input.read_exact(&mut header)?;
    ensure!(&header[..8] == MAGIC, "Not a Vita Save archive");
    let len = u32::from_le_bytes(header[8..12].try_into()?) as u64;
    ensure!(len <= MANIFEST_LIMIT, "Archive manifest exceeds size limit");
    let mut data = vec![0; len as usize];
    input.read_exact(&mut data)?;
    let manifest: Manifest = backup::decode(data.as_slice(), MANIFEST_LIMIT)?;
    manifest.validate()?;
    ensure!(
        manifest.id == expected_id && manifest.title_id == title_id && manifest.save_id == save_id,
        "Downloaded snapshot identity mismatch"
    );
    ensure!(
        !store.path(expected_id)?.exists(),
        "Snapshot is already local"
    );
    let mut stage = store.stage(expected_id)?;
    for dir in &manifest.directories {
        std::fs::create_dir_all(stage.path.join("files").join(dir))?;
    }
    let mut buffer = vec![0; 64 * 1024];
    let mut done = 0;
    for entry in &manifest.files {
        let mut file = File::create(stage.path.join("files").join(&entry.path))?;
        let mut left = entry.size;
        let mut hash = Sha256::new();
        while left > 0 {
            control.check()?;
            let amount = left.min(buffer.len() as u64) as usize;
            let n = input.read(&mut buffer[..amount])?;
            ensure!(n > 0, "Truncated archive: {}", entry.path);
            file.write_all(&buffer[..n])?;
            hash.update(&buffer[..n]);
            left -= n as u64;
            done += n as u64;
            control.set("download", &entry.path, done, manifest.bytes());
        }
        file.sync_all()?;
        ensure!(
            <[u8; 32]>::from(hash.finalize()) == entry.sha256,
            "Downloaded checksum mismatch: {}",
            entry.path
        );
    }
    let mut trailing = [0; 1];
    ensure!(input.read(&mut trailing)? == 0, "Trailing archive data");
    backup::write_synced(&stage.path.join("manifest.bin"), &data)?;
    backup::verify_files(&stage.path.join("files"), &manifest, control)
        .context("Verify downloaded files")?;
    store.commit(&mut stage, &manifest)?;
    Ok(manifest)
}
