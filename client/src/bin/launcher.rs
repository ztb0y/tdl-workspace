#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(not(windows))]
fn main() {
    eprintln!("tdl-web launcher only supports Windows x64");
}

#[cfg(windows)]
mod app {
    use std::{
        fs::{self, OpenOptions},
        io::{BufRead, BufReader, Write},
        path::PathBuf,
        process::{Child, Command, Stdio},
        sync::{
            Arc, Mutex, RwLock,
            atomic::{AtomicBool, Ordering},
            mpsc,
        },
        thread,
        time::{Duration, Instant},
    };

    use anyhow::{Context, Result, bail};
    use slint::ComponentHandle;
    use tdl_web_client::{
        APP_VERSION, LauncherWindow,
        manifest::{ComponentsManifest, ReleaseManifest, verify_components},
        paths,
        ready::ReadyState,
        static_server::StaticServer,
        update,
        windows::{
            Job, SingleInstance, center_window, configure_native_window, drag_window,
            minimize_window, open_browser, system_proxy,
        },
    };

    struct Services {
        child: Child,
        job: Job,
        server: StaticServer,
        _run_dir: RunDirectory,
        ui_origin: String,
    }

    struct RunDirectory(PathBuf);

    impl Drop for RunDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    struct DownloadedUpdate {
        manifest: ReleaseManifest,
        setup: PathBuf,
    }

    #[derive(Default)]
    struct LauncherState {
        services: Mutex<Option<Services>>,
        update: Mutex<Option<DownloadedUpdate>>,
        starting: AtomicBool,
        cancel_start: AtomicBool,
    }

    enum UiEvent {
        Status(String),
        Log(String),
        Started { api: String, ui: String },
        Stopped,
        Update(String),
        UpdateReady(String),
        Quit,
    }

    pub fn run() -> Result<()> {
        let _instance = SingleInstance::acquire("Local\\ztb0y.tdl-web.launcher")?;
        let ui = LauncherWindow::new()?;
        ui.set_version_text(format!("组合版本 {APP_VERSION}").into());

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

        {
            let weak = ui.as_weak();
            slint::Timer::single_shot(Duration::from_millis(100), move || {
                if let Some(ui) = weak.upgrade() {
                    configure_native_window(ui.window());
                    center_window(ui.window());
                }
            });
        }

        let state = Arc::new(LauncherState::default());
        let (events_tx, events_rx) = mpsc::channel();
        let event_log = Arc::new(Mutex::new(Vec::<String>::new()));

        {
            let state = state.clone();
            let tx = events_tx.clone();
            ui.on_start_service(move || start_async(state.clone(), tx.clone()));
        }
        {
            let state = state.clone();
            let tx = events_tx.clone();
            ui.on_stop_service(move || stop_services(&state, &tx));
        }
        {
            let state = state.clone();
            let tx = events_tx.clone();
            ui.on_open_web(move || {
                let result = state
                    .services
                    .lock()
                    .ok()
                    .and_then(|services| {
                        services.as_ref().map(|services| services.ui_origin.clone())
                    })
                    .context("UI is not running")
                    .and_then(|origin| open_browser(&origin));
                if let Err(error) = result {
                    let _ = tx.send(UiEvent::Log(format!("打开浏览器失败：{error:#}")));
                }
            });
        }
        {
            let state = state.clone();
            let tx = events_tx.clone();
            ui.on_check_update(move || check_update_async(state.clone(), tx.clone()));
        }
        {
            let state = state.clone();
            let tx = events_tx.clone();
            ui.on_install_update(move || install_update(&state, &tx));
        }

        let weak = ui.as_weak();
        let monitor_state = state.clone();
        let timer = slint::Timer::default();
        timer.start(
            slint::TimerMode::Repeated,
            Duration::from_millis(100),
            move || {
                let Some(ui) = weak.upgrade() else { return };
                while let Ok(event) = events_rx.try_recv() {
                    match event {
                        UiEvent::Status(value) => ui.set_status_text(value.into()),
                        UiEvent::Log(value) => {
                            if let Ok(mut logs) = event_log.lock() {
                                logs.push(value);
                                if logs.len() > 200 {
                                    logs.remove(0);
                                }
                                ui.set_log_text(logs.join("\n").into());
                            }
                        }
                        UiEvent::Started { api, ui: ui_origin } => {
                            ui.set_running(true);
                            ui.set_status_text("运行中".into());
                            ui.set_api_origin(format!("API  {api}").into());
                            ui.set_ui_origin(format!("UI   {ui_origin}").into());
                        }
                        UiEvent::Stopped => {
                            ui.set_running(false);
                            ui.set_status_text("已停止".into());
                            ui.set_api_origin("API 尚未启动".into());
                            ui.set_ui_origin("UI 尚未启动".into());
                        }
                        UiEvent::Update(value) => ui.set_update_text(value.into()),
                        UiEvent::UpdateReady(value) => {
                            ui.set_update_ready(true);
                            ui.set_update_text(value.into());
                        }
                        UiEvent::Quit => {
                            let _ = slint::quit_event_loop();
                        }
                    }
                }
                if let Some(status) = take_exited_services(&monitor_state) {
                    ui.set_running(false);
                    ui.set_status_text(format!("tdl 已退出：{status}").into());
                    ui.set_api_origin("API 尚未启动".into());
                    ui.set_ui_origin("UI 尚未启动".into());
                }
            },
        );

        start_async(state.clone(), events_tx.clone());
        check_update_async(state.clone(), events_tx.clone());
        let result = ui.run();
        stop_services(&state, &events_tx);
        result?;
        Ok(())
    }

    fn start_async(state: Arc<LauncherState>, tx: mpsc::Sender<UiEvent>) {
        if state.starting.swap(true, Ordering::AcqRel) {
            return;
        }
        let already_running = state
            .services
            .lock()
            .map(|services| services.is_some())
            .unwrap_or(true);
        if already_running {
            state.starting.store(false, Ordering::Release);
            return;
        }
        state.cancel_start.store(false, Ordering::Release);
        thread::spawn(move || {
            let result = start_services(&state, &tx);
            state.starting.store(false, Ordering::Release);
            if let Err(error) = result {
                let _ = tx.send(UiEvent::Status("启动失败".into()));
                let _ = tx.send(UiEvent::Log(format!("启动失败：{error:#}")));
                stop_services(&state, &tx);
            }
        });
    }

    fn start_services(state: &LauncherState, tx: &mpsc::Sender<UiEvent>) -> Result<()> {
        tx.send(UiEvent::Status("正在校验组件".into())).ok();
        let install_root = paths::install_root()?;
        let manifest = ComponentsManifest::load(&install_root.join("components.json"))?;
        if manifest.version != APP_VERSION {
            bail!(
                "runtime version {} does not match launcher {}",
                manifest.version,
                APP_VERSION
            );
        }
        verify_components(&install_root, &manifest)?;
        ensure_starting(state)?;
        tx.send(UiEvent::Log("组件 SHA-256 校验通过".into())).ok();

        let runtime = install_root.join("runtime");
        let tdl = runtime.join("tdl.exe");
        let extension = runtime.join("tdl-web.exe");
        let ui_root = runtime.join("ui");
        let run_dir = RunDirectory(paths::create_run_dir()?);
        let ready_file = run_dir.0.join("api-ready.json");
        let api_origin = Arc::new(RwLock::new(String::new()));
        let server = StaticServer::start(ui_root, api_origin.clone())?;
        let ui_origin = server.origin().to_owned();
        tx.send(UiEvent::Log(format!("UI 已绑定随机端口：{ui_origin}")))
            .ok();

        tx.send(UiEvent::Status("正在安装扩展".into())).ok();
        let output = Command::new(&tdl)
            .args(["extension", "install", "--force"])
            .arg(&extension)
            .current_dir(&install_root)
            .output()
            .context("install tdl-web extension")?;
        if !output.status.success() {
            bail!("安装扩展失败：{}", String::from_utf8_lossy(&output.stderr));
        }
        ensure_starting(state)?;

        tx.send(UiEvent::Status("正在启动 API".into())).ok();
        let log_dir = paths::app_data_root()?.join("logs");
        fs::create_dir_all(&log_dir)?;
        let log_file = log_dir.join("latest.log");
        let mut command = Command::new(&tdl);
        command
            .arg("web")
            .current_dir(paths::downloads_dir()?)
            .env("TDL_WEB_ADDR", "127.0.0.1:0")
            .env("TDL_WEB_READY_FILE", &ready_file)
            .env("TDL_WEB_TAKEOUT", "1")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        tx.send(UiEvent::Log("已启用 Telegram Takeout 会话".into()))
            .ok();
        if std::env::var_os("TDL_PROXY").is_none()
            && let Some(proxy) = system_proxy()
        {
            command.env("TDL_PROXY", proxy);
            tx.send(UiEvent::Log("已使用 Windows 系统代理".into())).ok();
        }
        let mut child = command.spawn().context("start tdl web")?;
        let job = Job::new()?;
        job.assign(&child)?;
        if let Some(stdout) = child.stdout.take() {
            forward_logs(stdout, log_file.clone(), tx.clone());
        }
        if let Some(stderr) = child.stderr.take() {
            forward_logs(stderr, log_file, tx.clone());
        }

        let deadline = Instant::now() + Duration::from_secs(30);
        let ready = loop {
            ensure_starting(state)?;
            if let Some(status) = child.try_wait()? {
                bail!("tdl web 提前退出：{status}");
            }
            if let Ok(data) = fs::read(&ready_file) {
                break ReadyState::parse(&data)?;
            }
            if Instant::now() >= deadline {
                bail!("等待 API 随机端口超时");
            }
            thread::sleep(Duration::from_millis(100));
        };
        ensure_starting(state)?;
        *api_origin
            .write()
            .map_err(|_| anyhow::anyhow!("API origin lock poisoned"))? = ready.api_origin.clone();

        let services = Services {
            child,
            job,
            server,
            _run_dir: run_dir,
            ui_origin: ui_origin.clone(),
        };
        let mut slot = state
            .services
            .lock()
            .map_err(|_| anyhow::anyhow!("service lock poisoned"))?;
        if slot.is_some() {
            bail!("services already running");
        }
        *slot = Some(services);
        drop(slot);

        tx.send(UiEvent::Started {
            api: ready.api_origin,
            ui: ui_origin.clone(),
        })
        .ok();
        if let Err(error) = open_browser(&ui_origin) {
            tx.send(UiEvent::Log(format!("自动打开浏览器失败：{error:#}")))
                .ok();
        }
        Ok(())
    }

    fn stop_services(state: &LauncherState, tx: &mpsc::Sender<UiEvent>) {
        state.cancel_start.store(true, Ordering::Release);
        let Ok(mut slot) = state.services.lock() else {
            return;
        };
        if let Some(mut services) = slot.take() {
            let _ = services.child.kill();
            let _ = services.child.wait();
            drop(services.job);
            drop(services.server);
            tx.send(UiEvent::Log("服务已停止".into())).ok();
        }
        tx.send(UiEvent::Stopped).ok();
    }

    fn ensure_starting(state: &LauncherState) -> Result<()> {
        if state.cancel_start.load(Ordering::Acquire) {
            bail!("启动已取消");
        }
        Ok(())
    }

    fn take_exited_services(state: &LauncherState) -> Option<std::process::ExitStatus> {
        let mut slot = state.services.lock().ok()?;
        let status = slot.as_mut()?.child.try_wait().ok()??;
        let services = slot.take();
        drop(slot);
        drop(services);
        Some(status)
    }

    fn forward_logs(
        reader: impl std::io::Read + Send + 'static,
        path: PathBuf,
        tx: mpsc::Sender<UiEvent>,
    ) {
        thread::spawn(move || {
            let mut file = OpenOptions::new().create(true).append(true).open(path).ok();
            for line in BufReader::new(reader).lines().map_while(Result::ok) {
                let line = strip_ansi(&line);
                if let Some(file) = file.as_mut() {
                    let _ = writeln!(file, "{line}");
                }
                tx.send(UiEvent::Log(line)).ok();
            }
        });
    }

    fn strip_ansi(value: &str) -> String {
        let mut result = String::with_capacity(value.len());
        let mut characters = value.chars().peekable();
        while let Some(character) = characters.next() {
            if character == '\u{1b}' && characters.next_if_eq(&'[').is_some() {
                for character in characters.by_ref() {
                    if ('@'..='~').contains(&character) {
                        break;
                    }
                }
            } else {
                result.push(character);
            }
        }
        result
    }

    fn check_update_async(state: Arc<LauncherState>, tx: mpsc::Sender<UiEvent>) {
        thread::spawn(move || {
            tx.send(UiEvent::Update("正在检查 GitHub Release".into()))
                .ok();
            let result = (|| -> Result<()> {
                let manifest = update::fetch_release_manifest()?;
                if !update::is_newer(APP_VERSION, &manifest.version)? {
                    tx.send(UiEvent::Update("当前已是最新组合版本".into())).ok();
                    return Ok(());
                }
                let update_dir = paths::app_data_root()?
                    .join("updates")
                    .join(&manifest.version);
                fs::create_dir_all(&update_dir)?;
                let setup = update_dir.join(&manifest.setup.filename);
                tx.send(UiEvent::Update(format!(
                    "正在下载组合版本 {}",
                    manifest.version
                )))
                .ok();
                update::download_setup(&manifest, &setup)?;
                let version = manifest.version.clone();
                *state
                    .update
                    .lock()
                    .map_err(|_| anyhow::anyhow!("update lock poisoned"))? =
                    Some(DownloadedUpdate { manifest, setup });
                tx.send(UiEvent::UpdateReady(format!(
                    "版本 {version} 已校验，等待安装"
                )))
                .ok();
                Ok(())
            })();
            if let Err(error) = result {
                tx.send(UiEvent::Update(format!("更新检查不可用：{error:#}")))
                    .ok();
            }
        });
    }

    fn install_update(state: &LauncherState, tx: &mpsc::Sender<UiEvent>) {
        let update = state
            .update
            .lock()
            .ok()
            .and_then(|mut update| update.take());
        let Some(update) = update else { return };
        stop_services(state, tx);
        let result = (|| -> Result<()> {
            let root = paths::install_root()?;
            Command::new(&update.setup)
                .arg("--update")
                .arg("--wait-pid")
                .arg(std::process::id().to_string())
                .arg("--install-root")
                .arg(root)
                .spawn()
                .context("launch update setup")?;
            Ok(())
        })();
        match result {
            Ok(_) => {
                tx.send(UiEvent::Log(format!(
                    "正在安装组合版本 {}",
                    update.manifest.version
                )))
                .ok();
                tx.send(UiEvent::Quit).ok();
            }
            Err(error) => {
                tx.send(UiEvent::Log(format!("启动更新安装器失败：{error}")))
                    .ok();
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::strip_ansi;

        #[test]
        fn strips_ansi_sequences_from_log_lines() {
            assert_eq!(strip_ansi("\u{1b}[32mready\u{1b}[0m"), "ready");
        }
    }
}

#[cfg(windows)]
fn main() {
    if let Err(error) = app::run() {
        let _ = std::fs::create_dir_all(tdl_web_client::paths::app_data_root().unwrap_or_default());
        eprintln!("{error:#}");
    }
}
