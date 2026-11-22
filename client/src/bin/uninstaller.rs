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
        APP_VERSION, UninstallWindow, paths,
        windows::{
            center_window, configure_native_window, drag_window, local_temp_dir, minimize_window,
            schedule_self_delete, unregister_install,
        },
    };
    use uuid::Uuid;

    pub fn run() -> Result<()> {
        let args: Vec<String> = std::env::args().collect();
        if let Some(index) = args.iter().position(|arg| arg == "--worker") {
            let root = PathBuf::from(args.get(index + 1).context("missing uninstall root")?);
            return uninstall_worker(&root);
        }
        let root = argument_value(&args, "--root")?.unwrap_or(paths::install_root()?);
        validate_install_root(&root)?;

        let ui = UninstallWindow::new()?;
        ui.set_version_text(format!("版本 {APP_VERSION}").into());
        {
            let weak = ui.as_weak();
            ui.on_drag_window(move || {
                if let Some(ui) = weak.upgrade() {
                    drag_window(ui.window());
                }
            });
        }
        {
            let weak = ui.as_weak();
            ui.on_minimize_window(move || {
                if let Some(ui) = weak.upgrade() {
                    minimize_window(ui.window());
                }
            });
        }
        ui.on_close_window(|| {
            let _ = slint::quit_event_loop();
        });
        ui.on_uninstall(move || {
            if let Err(error) = spawn_worker(&root) {
                eprintln!("{error:#}");
                return;
            }
            let _ = slint::quit_event_loop();
        });
        {
            let weak = ui.as_weak();
            slint::Timer::single_shot(Duration::from_millis(100), move || {
                if let Some(ui) = weak.upgrade() {
                    configure_native_window(ui.window());
                    center_window(ui.window());
                }
            });
        }
        ui.run()?;
        Ok(())
    }

    fn argument_value(args: &[String], name: &str) -> Result<Option<PathBuf>> {
        let Some(index) = args.iter().position(|argument| argument == name) else {
            return Ok(None);
        };
        Ok(Some(PathBuf::from(
            args.get(index + 1)
                .with_context(|| format!("missing {name} value"))?,
        )))
    }

    fn spawn_worker(root: &Path) -> Result<()> {
        let temp_dir = local_temp_dir();
        fs::create_dir_all(&temp_dir)?;
        let worker = temp_dir.join(format!("uninstall-{}.exe", Uuid::new_v4()));
        fs::copy(std::env::current_exe()?, &worker)?;
        Command::new(worker).arg("--worker").arg(root).spawn()?;
        Ok(())
    }

    fn uninstall_worker(root: &Path) -> Result<()> {
        validate_install_root(root)?;
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

    fn validate_install_root(root: &Path) -> Result<()> {
        if !root.is_absolute()
            || !root.join("components.json").is_file()
            || !root.join("tdl-web-uninstaller.exe").is_file()
        {
            anyhow::bail!("invalid tdl-web installation root: {}", root.display());
        }
        Ok(())
    }
}

#[cfg(windows)]
fn main() {
    if let Err(error) = app::run() {
        eprintln!("{error:#}");
    }
}
