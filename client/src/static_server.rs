use std::{
    fs::File,
    net::{Ipv4Addr, TcpListener},
    path::{Component, Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use anyhow::{Context, Result, bail};
use tiny_http::{Header, Response, Server, StatusCode};

pub struct StaticServer {
    origin: String,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl StaticServer {
    pub fn start(root: PathBuf, api_origin: Arc<std::sync::RwLock<String>>) -> Result<Self> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        let address = listener.local_addr()?;
        let server =
            Server::from_listener(listener, None).map_err(|error| anyhow::anyhow!(error))?;
        let stop = Arc::new(AtomicBool::new(false));
        let stop_thread = stop.clone();
        let thread = thread::spawn(move || {
            while !stop_thread.load(Ordering::Relaxed) {
                let Ok(request) = server.recv_timeout(Duration::from_millis(200)) else {
                    continue;
                };
                let request = match request {
                    Some(request) => request,
                    None => continue,
                };
                let path = request.url().split('?').next().unwrap_or("/");
                if path == "/runtime-config.js" {
                    let origin = api_origin
                        .read()
                        .map(|value| value.clone())
                        .unwrap_or_default();
                    let value = serde_json::to_string(&origin).unwrap_or_else(|_| "\"\"".into());
                    let response = Response::from_string(format!(
                        "window.__TDL_WEB_CONFIG__={{apiBase:{value}}};"
                    ))
                    .with_header(content_type("application/javascript; charset=utf-8"));
                    let _ = request.respond(response);
                    continue;
                }
                let relative = if path == "/" {
                    "index.html"
                } else {
                    path.trim_start_matches('/')
                };
                match safe_join(&root, relative).and_then(|path| {
                    let file =
                        File::open(&path).with_context(|| format!("open {}", path.display()))?;
                    Ok(Response::from_file(file).with_header(content_type(mime_for(&path))))
                }) {
                    Ok(response) => {
                        let _ = request.respond(response);
                    }
                    Err(_) => {
                        let _ = request.respond(
                            Response::from_string("Not found").with_status_code(StatusCode(404)),
                        );
                    }
                }
            }
        });
        Ok(Self {
            origin: format!("http://{address}"),
            stop,
            thread: Some(thread),
        })
    }

    pub fn origin(&self) -> &str {
        &self.origin
    }
}

impl Drop for StaticServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn safe_join(root: &Path, relative: &str) -> Result<PathBuf> {
    let relative = Path::new(relative);
    if relative.is_absolute()
        || relative
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        bail!("unsafe static path");
    }
    Ok(root.join(relative))
}

fn content_type(value: &str) -> Header {
    Header::from_bytes("Content-Type", value).expect("valid content type")
}

fn mime_for(path: &Path) -> &'static str {
    match path.extension().and_then(|value| value.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "application/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        Some("woff2") => "font/woff2",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn random_listeners_receive_distinct_ports() {
        let first = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let second = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        assert_ne!(
            first.local_addr().unwrap().port(),
            second.local_addr().unwrap().port()
        );
    }

    #[test]
    fn static_paths_cannot_escape_root() {
        assert!(safe_join(Path::new("ui"), "assets/index.js").is_ok());
        assert!(safe_join(Path::new("ui"), "../secret").is_err());
    }
}
