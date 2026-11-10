pub mod dashboard;
pub mod files;
pub mod history;
pub mod metadata;
pub mod permissions;
pub mod python_venv;
pub mod runtime;

use anyhow::{Context, Result};
use std::path::PathBuf;

pub fn home() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("SAT_HOME") {
        return Ok(path.into());
    }
    Ok(PathBuf::from(
        std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .context("set SAT_HOME to a registry directory")?,
    )
    .join(".sat"))
}

mod grants;

mod credentials;
