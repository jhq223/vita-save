use anyhow::{Context, Result, ensure};
use std::collections::BTreeMap;
use std::ops::Range;

pub fn entries(data: &[u8]) -> Result<BTreeMap<String, Range<usize>>> {
    ensure!(data.get(..4) == Some(b"\0PSF"), "Invalid SFO signature");
    let u32_at = |at: usize| -> Result<usize> {
        Ok(u32::from_le_bytes(data.get(at..at + 4).context("Truncated SFO")?.try_into()?) as usize)
    };
    let keys = u32_at(8)?;
    let values = u32_at(12)?;
    let count = u32_at(16)?;
    ensure!(
        count <= 4096 && keys >= 20 + count * 16 && values >= keys && values <= data.len(),
        "Invalid SFO tables"
    );
    let mut result = BTreeMap::new();
    for i in 0..count {
        let pos = 20 + i * 16;
        let key_offset = u16::from_le_bytes(
            data.get(pos..pos + 2)
                .context("Truncated SFO index")?
                .try_into()?,
        ) as usize;
        let key_start = keys
            .checked_add(key_offset)
            .context("SFO offset overflow")?;
        let key = data
            .get(key_start..values)
            .context("SFO key out of bounds")?;
        let key = std::str::from_utf8(
            &key[..key
                .iter()
                .position(|b| *b == 0)
                .context("Unterminated SFO key")?],
        )?;
        let len = u32_at(pos + 4)?;
        let max = u32_at(pos + 8)?;
        let start = values
            .checked_add(u32_at(pos + 12)?)
            .context("SFO overflow")?;
        let end = start.checked_add(len).context("SFO overflow")?;
        ensure!(
            len <= max && start.checked_add(max).is_some_and(|v| v <= data.len()),
            "SFO value out of bounds"
        );
        ensure!(
            result.insert(key.to_owned(), start..end).is_none(),
            "Duplicate SFO key"
        );
    }
    Ok(result)
}

pub fn text(data: &[u8], key: &str) -> Option<String> {
    let entries = entries(data).ok()?;
    let bytes = &data[entries.get(key)?.clone()];
    Some(
        std::str::from_utf8(bytes.split(|b| *b == 0).next()?)
            .ok()?
            .replace(['\r', '\n'], " "),
    )
}

pub fn set_account(data: &mut [u8], account: u64) -> Result<()> {
    let entries = entries(data)?;
    let range = entries
        .get("ACCOUNT_ID")
        .context("Save SFO has no ACCOUNT_ID")?;
    ensure!(range.len() == 8, "Invalid ACCOUNT_ID length");
    data[range.clone()].copy_from_slice(&account.to_le_bytes());
    Ok(())
}
