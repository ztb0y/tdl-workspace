#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(not(windows))]
fn main() {
    eprintln!("tdl-web uninstaller only supports Windows x64");
}

#[cfg(windows)]
mod app {
    use std::{
        fs,
        path::{Path, PathBuf},
        process::Command,
        thread,
        time::Duration,
    };

    use anyhow::{Context, Result};
    use slint::ComponentHandle;
    use tdl_web_client::{
        UninstallWindow, paths,
        windows::{local_temp_dir, schedule_self_delete, unregister_install},
    };
    use uuid::Uuid;

    pub fn run() -> Result<()> {
        let args: Vec<String> = std::env::args().collect();
        if let Some(index) = args.iter().position(|arg| arg == "--worker") {
            let root = PathBuf::from(args.get(index + 1).context("missing uninstall root")?);
            return uninstall_worker(&root);
        }

        let ui = UninstallWindow::new()?;
        ui.on_uninstall(move || {
            if let Err(error) = spawn_worker() {
                eprintln!("{error:#}");
                return;
            }
            let _ = slint::quit_event_loop();
        });
        ui.run()?;
        Ok(())
    }

    fn spawn_worker() -> Result<()> {
        let root = paths::install_root()?;
        let temp_dir = local_temp_dir();
        fs::create_dir_all(&temp_dir)?;
        let worker = temp_dir.join(format!("uninstall-{}.exe", Uuid::new_v4()));
        fs::copy(std::env::current_exe()?, &worker)?;
        Command::new(worker).arg("--worker").arg(root).spawn()?;
        Ok(())
    }

    fn uninstall_worker(root: &Path) -> Result<()> {
        let _ = Command::new("taskkill.exe")
            .args(["/IM", "tdl-web-launcher.exe", "/F"])
            .status();
        thread::sleep(Duration::from_millis(500));
        let tdl = root.join("runtime").join("tdl.exe");
        if tdl.exists() {
            let _ = Command::new(tdl)
                .args(["extension", "remove", "web"])
                .status();
        }
        paths::clear_run_dirs()?;
        unregister_install()?;
        for attempt in 0..40 {
            if !root.exists() || fs::remove_dir_all(root).is_ok() {
                schedule_self_delete(&std::env::current_exe()?)?;
                return Ok(());
            }
            if attempt < 39 {
                thread::sleep(Duration::from_millis(250));
            }
        }
        fs::remove_dir_all(root).context("remove installation")
    }
}

#[cfg(windows)]
fn main() {
    if let Err(error) = app::run() {
        eprintln!("{error:#}");
    }
}
