//! Reading and writing the checked-in contract in `contract/`.

use std::path::Path;

use genzo_model::typescript::{self, ContractLock, check_bump};

pub const TS_FILE: &str = "genzo.ts";
pub const LOCK_FILE: &str = "contract.lock";

#[derive(Debug, thiserror::Error)]
pub enum ContractError {
    #[error("{0}")]
    Drift(String),
    #[error("{0}")]
    Io(#[from] std::io::Error),
}

/// Checks `dir` against this build. Fails on drift or a missing bump.
pub fn check(dir: &Path) -> Result<(), ContractError> {
    let ts = std::fs::read_to_string(dir.join(TS_FILE)).unwrap_or_default();
    let lock = std::fs::read_to_string(dir.join(LOCK_FILE)).unwrap_or_default();
    typescript::check_checked_in(&ts, &lock).map_err(|d| ContractError::Drift(d.to_string()))
}

/// Regenerates `dir`. Refuses when the types changed without a version bump.
/// Returns whether anything changed.
pub fn write(dir: &Path) -> Result<bool, ContractError> {
    if let Ok(text) = std::fs::read_to_string(dir.join(LOCK_FILE))
        && let Ok(previous) = ContractLock::from_text(&text)
    {
        check_bump(&previous).map_err(|d| ContractError::Drift(d.to_string()))?;
    }
    std::fs::create_dir_all(dir)?;
    let ts = typescript::generate();
    let lock = ContractLock::current().to_text();
    let old_ts = std::fs::read_to_string(dir.join(TS_FILE)).ok();
    let old_lock = std::fs::read_to_string(dir.join(LOCK_FILE)).ok();
    if old_ts.as_deref() == Some(&ts) && old_lock.as_deref() == Some(&lock) {
        return Ok(false);
    }
    std::fs::write(dir.join(TS_FILE), ts)?;
    std::fs::write(dir.join(LOCK_FILE), lock)?;
    Ok(true)
}
