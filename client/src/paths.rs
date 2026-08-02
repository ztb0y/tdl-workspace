use std::{env, path::PathBuf};

use anyhow::{Context, Result};
use uuid::Uuid;

pub fn install_root() -> Result<PathBuf> {
    if let Some(path) = env::var_os("TDL_WEB_INSTALL_ROOT") {
        return Ok(PathBuf::from(path));
    }
    let local = env::var_os("LOCALAPPDATA").context("LOCALAPPDATA is not set")?;
    Ok(PathBuf::from(local).join("Programs").join("tdl-web"))
}

pub fn app_data_root() -> Result<PathBuf> {
    let local = env::var_os("LOCALAPPDATA").context("LOCALAPPDATA is not set")?;
    Ok(PathBuf::from(local).join("tdl-web"))
}

pub fn create_run_dir() -> Result<PathBuf> {
    clear_run_dirs()?;
    let path = app_data_root()?
        .join("runs")
        .join(Uuid::new_v4().to_string());
    std::fs::create_dir_all(&path)?;
    Ok(path)
}

pub fn clear_run_dirs() -> Result<()> {
    let runs = app_data_root()?.join("runs");
    if !runs.exists() {
        return Ok(());
    }
    for entry in std::fs::read_dir(&runs)? {
        let path = entry?.path();
        if path.is_dir() {
            std::fs::remove_dir_all(path)?;
        } else {
            std::fs::remove_file(path)?;
        }
    }
    Ok(())
}

pub fn runtime_dir() -> Result<PathBuf> {
    Ok(install_root()?.join("runtime"))
}

pub fn downloads_dir() -> Result<PathBuf> {
    let profile = env::var_os("USERPROFILE").context("USERPROFILE is not set")?;
    let path = PathBuf::from(profile).join("Downloads").join("tdl-web");
    std::fs::create_dir_all(&path)?;
    Ok(path)
}
