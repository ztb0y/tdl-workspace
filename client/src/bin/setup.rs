#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(not(windows))]
fn main() {
    eprintln!("tdl-web setup only supports Windows x64");
}

#[cfg(windows)]
mod app {
    use std::{
        fs::{self, File},
        io::{self, Cursor, Write},
        path::{Path, PathBuf},
        process::Command,
        sync::mpsc,
        thread,
        time::{Duration, SystemTime, UNIX_EPOCH},
    };

    use anyhow::{Context, Result};
    use slint::ComponentHandle;
    use tdl_web_client::{
        APP_VERSION, SetupWindow,
        manifest::{ComponentsManifest, verify_components},
        paths,
        windows::{
            center_window, choose_install_folder, configure_native_window, drag_window,
            minimize_window, register_install, replace_file, wait_for_pid,
        },
    };
    use uuid::Uuid;
    use zip::ZipArchive;

    const BUNDLE: &[u8] =
        include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/bundle/runtime.zip"));

    enum Event {
        Status(String, f32),
        Finished,
        Failed(String),
    }

    struct CleanupDir(PathBuf);

    impl Drop for CleanupDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    pub fn run() -> Result<()> {
        let args: Vec<String> = std::env::args().collect();
        let update = args.iter().any(|arg| arg == "--update");
        let install_root =
            argument_value(&args, "--install-root")?.unwrap_or(paths::default_install_root()?);
        if !install_root.is_absolute() {
            anyhow::bail!("install root must be an absolute path");
        }
        if let Some(index) = args.iter().position(|arg| arg == "--wait-pid") {
            let pid = args
                .get(index + 1)
                .context("missing --wait-pid value")?
                .parse()?;
            wait_for_pid(pid, Duration::from_secs(30))?;
        }

        let ui = SetupWindow::new()?;
        ui.set_version_text(APP_VERSION.into());
        ui.set_install_path(install_root.display().to_string().into());
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
        if update {
            ui.set_status_text("准备更新已安装的组合版本".into());
        }
        let (tx, rx) = mpsc::channel();
        {
            let tx = tx.clone();
            let weak = ui.as_weak();
            ui.on_install(move || {
                let Some(ui) = weak.upgrade() else { return };
                install_async(tx.clone(), PathBuf::from(ui.get_install_path().to_string()));
            });
        }
        {
            let weak = ui.as_weak();
            ui.on_choose_install_location(move || match choose_install_folder() {
                Ok(Some(path)) => {
                    let Some(ui) = weak.upgrade() else { return };
                    ui.set_install_path(path.display().to_string().into());
                    ui.set_failed(false);
                    ui.set_status_text("".into());
                }
                Ok(None) => {}
                Err(error) => {
                    let Some(ui) = weak.upgrade() else { return };
                    ui.set_status_text(format!("选择安装位置失败：{error:#}").into());
                    ui.set_failed(true);
                }
            });
        }
        {
            let weak = ui.as_weak();
            ui.on_launch(move || {
                let Some(ui) = weak.upgrade() else { return };
                let root = PathBuf::from(ui.get_install_path().to_string());
                let _ = Command::new(root.join("tdl-web-launcher.exe")).spawn();
                let _ = slint::quit_event_loop();
            });
        }

        {
            let weak = ui.as_weak();
            slint::Timer::single_shot(Duration::from_millis(100), move || {
                if let Some(ui) = weak.upgrade() {
                    configure_native_window(ui.window());
                    center_window(ui.window());
                }
            });
        }

        let weak = ui.as_weak();
        let timer = slint::Timer::default();
        timer.start(
            slint::TimerMode::Repeated,
            Duration::from_millis(100),
            move || {
                let Some(ui) = weak.upgrade() else { return };
                while let Ok(event) = rx.try_recv() {
                    match event {
                        Event::Status(text, progress) => {
                            ui.set_status_text(text.into());
                            ui.set_progress(progress);
                            ui.set_busy(true);
                            ui.set_failed(false);
                        }
                        Event::Finished => {
                            ui.set_status_text("安装完成，账号数据保持不变。".into());
                            ui.set_progress(1.0);
                            ui.set_busy(false);
                            ui.set_finished(true);
                            ui.set_failed(false);
                        }
                        Event::Failed(error) => {
                            ui.set_status_text(format!("安装失败：{error}").into());
                            ui.set_busy(false);
                            ui.set_failed(true);
                        }
                    }
                }
            },
        );

        if update {
            install_async(tx, install_root);
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

    fn install_async(tx: mpsc::Sender<Event>, root: PathBuf) {
        thread::spawn(move || {
            if let Err(error) = install(&tx, &root) {
                tx.send(Event::Failed(format!("{error:#}"))).ok();
            }
        });
    }

    fn install(tx: &mpsc::Sender<Event>, root: &Path) -> Result<()> {
        let log_path = paths::app_data_root()?.join("logs").join("setup.log");
        fs::create_dir_all(log_path.parent().context("setup log has no parent")?)?;
        let mut log = File::create(&log_path).context("create setup log")?;
        write_log(&mut log, &format!("tdl-web setup {APP_VERSION} started"))?;
        match install_logged(tx, root, &mut log) {
            Ok(()) => {
                write_log(&mut log, "installation completed")?;
                Ok(())
            }
            Err(error) => {
                let _ = write_log(&mut log, &format!("installation failed: {error:#}"));
                Err(error).with_context(|| format!("日志：{}", log_path.display()))
            }
        }
    }

    fn install_logged(tx: &mpsc::Sender<Event>, root: &Path, log: &mut File) -> Result<()> {
        tx.send(Event::Status("正在解压离线组件".into(), 0.15)).ok();
        let parent = root.parent().context("install root has no parent")?;
        fs::create_dir_all(parent)?;
        let staging = CleanupDir(parent.join(format!("tdl-web.installing-{}", Uuid::new_v4())));
        write_log(
            log,
            &format!("extracting bundle to {}", staging.0.display()),
        )?;
        extract_bundle(&staging.0)?;

        tx.send(Event::Status("正在校验组件 SHA-256".into(), 0.55))
            .ok();
        let manifest = ComponentsManifest::load(&staging.0.join("components.json"))?;
        if manifest.version != APP_VERSION {
            anyhow::bail!(
                "runtime version {} does not match setup {}",
                manifest.version,
                APP_VERSION
            );
        }
        verify_components(&staging.0, &manifest)?;
        write_log(log, "component verification completed")?;

        tx.send(Event::Status("正在原子替换运行组件".into(), 0.75))
            .ok();
        if root.exists() {
            write_log(log, &format!("updating installation at {}", root.display()))?;
            activate_in_place(&staging.0, &root, log)?;
        } else {
            write_log(
                log,
                &format!("activating installation at {}", root.display()),
            )?;
            rename_with_retry(&staging.0, &root, log).context("activate installation")?;
        }

        tx.send(Event::Status("正在创建快捷方式".into(), 0.9)).ok();
        write_log(log, "registering uninstall entry and shortcuts")?;
        register_install(&root, &manifest.version)?;
        tx.send(Event::Finished).ok();
        Ok(())
    }

    fn rename_with_retry(from: &Path, to: &Path, log: &mut File) -> io::Result<()> {
        let mut attempts = 0;
        loop {
            match fs::rename(from, to) {
                Ok(()) => {
                    if attempts > 0 {
                        let _ =
                            write_log(log, &format!("rename succeeded after {attempts} retries"));
                    }
                    return Ok(());
                }
                Err(error) if error.kind() == io::ErrorKind::PermissionDenied && attempts < 119 => {
                    attempts += 1;
                    let _ = write_log(
                        log,
                        &format!(
                            "rename retry {attempts}/119: {} -> {}: {error}",
                            from.display(),
                            to.display()
                        ),
                    );
                    thread::sleep(Duration::from_millis(250));
                }
                Err(error) => return Err(error),
            }
        }
    }

    fn activate_in_place(staging: &Path, root: &Path, log: &mut File) -> Result<()> {
        for entry in fs::read_dir(staging)? {
            let entry = entry?;
            if entry.file_name() == "components.json" {
                continue;
            }
            merge_entry(&entry.path(), &root.join(entry.file_name()), log)?;
        }
        replace_file_with_retry(
            &staging.join("components.json"),
            &root.join("components.json"),
            log,
        )
        .context("commit component manifest")
    }

    fn merge_entry(source: &Path, destination: &Path, log: &mut File) -> Result<()> {
        if source.is_dir() {
            fs::create_dir_all(destination)?;
            for entry in fs::read_dir(source)? {
                let entry = entry?;
                merge_entry(&entry.path(), &destination.join(entry.file_name()), log)?;
            }
            return Ok(());
        }
        replace_file_with_retry(source, destination, log)
            .with_context(|| format!("replace {}", destination.display()))
    }

    fn replace_file_with_retry(from: &Path, to: &Path, log: &mut File) -> io::Result<()> {
        let mut attempts = 0;
        loop {
            match replace_file(from, to) {
                Ok(()) => return Ok(()),
                Err(error) if error.kind() == io::ErrorKind::PermissionDenied && attempts < 119 => {
                    attempts += 1;
                    let _ = write_log(
                        log,
                        &format!(
                            "file retry {attempts}/119: {} -> {}: {error}",
                            from.display(),
                            to.display()
                        ),
                    );
                    thread::sleep(Duration::from_millis(250));
                }
                Err(error) => return Err(error),
            }
        }
    }

    fn write_log(log: &mut File, message: &str) -> io::Result<()> {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        writeln!(log, "{timestamp} {message}")?;
        log.flush()
    }

    fn extract_bundle(target: &Path) -> Result<()> {
        if target.exists() {
            fs::remove_dir_all(target)?;
        }
        fs::create_dir_all(target)?;
        let mut archive = ZipArchive::new(Cursor::new(BUNDLE)).context("open embedded runtime")?;
        for index in 0..archive.len() {
            let mut entry = archive.by_index(index)?;
            let relative = entry
                .enclosed_name()
                .context("unsafe path in embedded runtime")?
                .to_owned();
            let path = target.join(relative);
            if entry.is_dir() {
                fs::create_dir_all(path)?;
                continue;
            }
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut output = File::create(path)?;
            std::io::copy(&mut entry, &mut output)?;
            output.flush()?;
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
