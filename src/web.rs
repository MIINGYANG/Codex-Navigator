//! Loopback-only HTTP adapter. All Codex access stays in the existing read-only core.
use crate::{
    app::turn_text,
    config::Config,
    discovery,
    domain::{Session, SessionSummary, Turn, TurnItem, TurnStatus},
    index::SearchIndex,
    watch::{SessionWorker, Update},
};
use anyhow::{Context, Result};
use axum::{
    body::{Body, Bytes},
    extract::{Query, Request, State},
    http::{header, HeaderMap, HeaderValue, Method, StatusCode},
    response::Response,
    Router,
};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, VecDeque},
    future::{Future, IntoFuture},
    io::Write,
    path::PathBuf,
    pin::Pin,
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    task::{Context as TaskContext, Poll},
    thread,
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    sync::{oneshot, OwnedSemaphorePermit, Semaphore},
};

const MAX_OPEN: usize = 2;
const MAX_SESSIONS: usize = 20_000;
const CSP: &str = "default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self'; img-src 'self' data:; font-src 'self'; base-uri 'none'; frame-ancestors 'none'; form-action 'none'";

pub struct WebOptions {
    pub port: u16,
    pub no_open: bool,
    pub all: bool,
    pub session: Option<String>,
}

#[derive(Clone)]
struct HttpState {
    authority: String,
    token: String,
    sender: mpsc::SyncSender<ApiRequest>,
    capacity: Arc<Semaphore>,
}

struct ApiRequest {
    path: String,
    query: HashMap<String, String>,
    reply: oneshot::Sender<ApiReply>,
}

struct ApiReply {
    status: StatusCode,
    content_type: &'static str,
    body: Vec<u8>,
}

impl ApiReply {
    fn json(value: Value) -> Self {
        Self {
            status: StatusCode::OK,
            content_type: "application/json; charset=utf-8",
            body: serde_json::to_vec(&value).expect("JSON values serialize"),
        }
    }

    fn error(status: StatusCode, message: &str) -> Self {
        let mut reply = Self::json(json!({"error": message}));
        reply.status = status;
        reply
    }

    fn response(self) -> Response {
        self.response_with_permit(None)
    }

    fn response_with_permit(self, permit: Option<OwnedSemaphorePermit>) -> Response {
        // A large copy response cannot escape the request budget while a client reads slowly.
        let body_length = self.body.len();
        let stream = futures_util::stream::unfold(
            (Bytes::from(self.body), permit),
            |(mut bytes, permit)| async move {
                if bytes.is_empty() {
                    return None;
                }
                let chunk = bytes.split_to(bytes.len().min(16 * 1024));
                Some((Ok::<_, std::convert::Infallible>(chunk), (bytes, permit)))
            },
        );
        let mut response = Response::new(Body::from_stream(stream));
        *response.status_mut() = self.status;
        let headers = response.headers_mut();
        headers.insert(header::CONTENT_LENGTH, HeaderValue::from(body_length));
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static(self.content_type),
        );
        headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
        headers.insert(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static(CSP),
        );
        headers.insert(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        );
        headers.insert(
            header::REFERRER_POLICY,
            HeaderValue::from_static("no-referrer"),
        );
        headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
        response
    }
}

/// Starts an independent local reader, never a Codex subprocess or terminal wrapper.
pub fn run(home: PathBuf, cwd: PathBuf, config: Config, options: WebOptions) -> Result<()> {
    let mut entropy = [0_u8; 32];
    getrandom::fill(&mut entropy).map_err(|_| anyhow::anyhow!("Cannot obtain secure Web token"))?;
    let token: String = entropy.iter().map(|byte| format!("{byte:02x}")).collect();
    let mut backend = Backend::new(home, cwd, config, options.all);
    if let Some(session) = options.session {
        let path = discovery::resolve_session(&backend.home, &session, &backend.config)?;
        backend.initial_session = Some(backend.register(path, true)?);
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;
    runtime.block_on(async move {
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, options.port))
            .await
            .context("Cannot bind local Web port; use --port to select another port")?;
        let authority = listener.local_addr()?.to_string();
        let url = format!("http://{authority}/#token={token}");
        let (sender, receiver) = mpsc::sync_channel(4);
        let stopped = Arc::new(AtomicBool::new(false));
        let worker_stop = stopped.clone();
        let worker = thread::Builder::new()
            .name("web-state".into())
            .spawn(move || {
                backend.start_scan(backend.default_all);
                while !worker_stop.load(Ordering::Relaxed) {
                    backend.tick();
                    match receiver.recv_timeout(Duration::from_millis(100)) {
                        Ok(request) => {
                            let ApiRequest { path, query, reply } = request;
                            if !reply.is_closed() {
                                let _ = reply.send(backend.route(&path, &query));
                            }
                        }
                        Err(mpsc::RecvTimeoutError::Timeout) => (),
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    }
                }
            })?;
        let state = HttpState {
            authority,
            token,
            sender,
            capacity: Arc::new(Semaphore::new(4)),
        };
        let router = Router::new().fallback(handle).with_state(state);
        println!("Codex Navigator Web — 本机只读，Ctrl+C 停止\n{url}");
        std::io::stdout().flush()?;
        if !options.no_open {
            if let Err(error) = open_browser(&url) {
                eprintln!("无法自动打开浏览器（{error}），请复制上方地址。");
            }
        }
        let listener = LimitedListener {
            listener,
            capacity: Arc::new(Semaphore::new(32)),
        };
        let (halt, halted) = oneshot::channel::<()>();
        let mut server = Box::pin(
            axum::serve(listener, router)
                .with_graceful_shutdown(async {
                    let _ = halted.await;
                })
                .into_future(),
        );
        let result = tokio::select! {
            result = &mut server => result,
            _ = tokio::signal::ctrl_c() => {
                let _ = halt.send(());
                tokio::time::timeout(Duration::from_secs(3), &mut server).await.unwrap_or(Ok(()))
            }
        };
        drop(server);
        stopped.store(true, Ordering::Relaxed);
        let _ = worker.join();
        result.context("Local Web server stopped unexpectedly")
    })
}

/// Restrict slow/incomplete HTTP connections without implementing HTTP ourselves.
struct LimitedListener {
    listener: tokio::net::TcpListener,
    capacity: Arc<Semaphore>,
}

struct LimitedIo {
    stream: tokio::net::TcpStream,
    _permit: OwnedSemaphorePermit,
    deadline: Pin<Box<tokio::time::Sleep>>,
}

impl axum::serve::Listener for LimitedListener {
    type Io = LimitedIo;
    type Addr = std::net::SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        let permit = self
            .capacity
            .clone()
            .acquire_owned()
            .await
            .expect("connection semaphore remains open");
        loop {
            match self.listener.accept().await {
                Ok((stream, address)) => {
                    return (
                        LimitedIo {
                            stream,
                            _permit: permit,
                            deadline: Box::pin(tokio::time::sleep(Duration::from_secs(60))),
                        },
                        address,
                    )
                }
                Err(_) => tokio::time::sleep(Duration::from_millis(100)).await,
            }
        }
    }

    fn local_addr(&self) -> std::io::Result<Self::Addr> {
        self.listener.local_addr()
    }
}

impl LimitedIo {
    fn expired(&mut self, context: &mut TaskContext<'_>) -> std::io::Result<()> {
        if self.deadline.as_mut().poll(context).is_ready() {
            Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "local HTTP connection deadline",
            ))
        } else {
            Ok(())
        }
    }
}

impl AsyncRead for LimitedIo {
    fn poll_read(
        mut self: Pin<&mut Self>,
        context: &mut TaskContext<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        self.expired(context)?;
        Pin::new(&mut self.stream).poll_read(context, buffer)
    }
}

impl AsyncWrite for LimitedIo {
    fn poll_write(
        mut self: Pin<&mut Self>,
        context: &mut TaskContext<'_>,
        bytes: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        self.expired(context)?;
        Pin::new(&mut self.stream).poll_write(context, bytes)
    }
    fn poll_flush(
        mut self: Pin<&mut Self>,
        context: &mut TaskContext<'_>,
    ) -> Poll<std::io::Result<()>> {
        self.expired(context)?;
        Pin::new(&mut self.stream).poll_flush(context)
    }
    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        context: &mut TaskContext<'_>,
    ) -> Poll<std::io::Result<()>> {
        self.expired(context)?;
        Pin::new(&mut self.stream).poll_shutdown(context)
    }
}

fn open_browser(url: &str) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    let mut command = Command::new("open");
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = Command::new("rundll32");
        command.arg("url.dll,FileProtocolHandler");
        command
    };
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let mut command = Command::new("xdg-open");
    let mut child = command
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

fn unique_header<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    let mut values = headers.get_all(name).iter();
    let value = values.next()?.to_str().ok()?;
    values.next().is_none().then_some(value)
}

fn authorized(headers: &HeaderMap, state: &HttpState, api: bool) -> bool {
    if unique_header(headers, "host") != Some(state.authority.as_str()) {
        return false;
    }
    if headers.contains_key("origin")
        && unique_header(headers, "origin") != Some(format!("http://{}", state.authority).as_str())
    {
        return false;
    }
    // Public embedded assets contain no token/session data. A normal link or window.open
    // may be a cross-site top-level navigation; only the authenticated API needs this guard.
    if !api {
        return true;
    }
    if unique_header(headers, "sec-fetch-site") == Some("cross-site") {
        return false;
    }
    let Some(token) = unique_header(headers, "x-codex-nav-token") else {
        return false;
    };
    token.len() == state.token.len()
        && token
            .bytes()
            .zip(state.token.bytes())
            .fold(0_u8, |diff, (a, b)| diff | (a ^ b))
            == 0
}

async fn handle(State(state): State<HttpState>, request: Request) -> Response {
    if request.uri().to_string().len() > 8192 {
        return ApiReply::error(StatusCode::URI_TOO_LONG, "请求地址过长").response();
    }
    let path = request.uri().path();
    if !authorized(request.headers(), &state, path.starts_with("/api/")) {
        return ApiReply::error(
            StatusCode::FORBIDDEN,
            "请使用终端输出的完整本机地址打开页面",
        )
        .response();
    }
    if request.method() != Method::GET {
        return ApiReply::error(StatusCode::METHOD_NOT_ALLOWED, "仅支持只读 GET 请求").response();
    }
    if request.headers().contains_key(header::TRANSFER_ENCODING)
        || request
            .headers()
            .get(header::CONTENT_LENGTH)
            .is_some_and(|v| v != "0")
    {
        return ApiReply::error(StatusCode::BAD_REQUEST, "GET 请求不能携带正文").response();
    }
    let resource = match path {
        "/" | "/index.html" => Some((
            "text/html; charset=utf-8",
            include_str!("../web/index.html"),
        )),
        "/app.css" => Some(("text/css; charset=utf-8", include_str!("../web/app.css"))),
        "/app.js" => Some((
            "text/javascript; charset=utf-8",
            include_str!("../web/app.js"),
        )),
        "/state.mjs" => Some((
            "text/javascript; charset=utf-8",
            include_str!("../web/state.mjs"),
        )),
        _ => None,
    };
    if let Some((content_type, body)) = resource {
        return ApiReply {
            status: StatusCode::OK,
            content_type,
            body: body.as_bytes().to_vec(),
        }
        .response();
    }
    if !path.starts_with("/api/") {
        return ApiReply::error(StatusCode::NOT_FOUND, "页面不存在").response();
    }
    let Ok(permit) = state.capacity.clone().try_acquire_owned() else {
        return ApiReply::error(StatusCode::SERVICE_UNAVAILABLE, "请求繁忙，请稍后重试").response();
    };
    let Ok(Query(query)) = Query::<HashMap<String, String>>::try_from_uri(request.uri()) else {
        return ApiReply::error(StatusCode::BAD_REQUEST, "查询参数无效").response();
    };
    let (reply, receiver) = oneshot::channel();
    if state
        .sender
        .try_send(ApiRequest {
            path: path.to_string(),
            query,
            reply,
        })
        .is_err()
    {
        return ApiReply::error(StatusCode::SERVICE_UNAVAILABLE, "服务暂不可用").response();
    }
    match tokio::time::timeout(Duration::from_secs(10), receiver).await {
        Ok(Ok(reply)) => reply.response_with_permit(Some(permit)),
        _ => ApiReply::error(StatusCode::SERVICE_UNAVAILABLE, "读取超时，请重试").response(),
    }
}

struct Registered {
    path: PathBuf,
    explicit: bool,
}

struct OpenSession {
    worker: SessionWorker,
    session: Session,
    index: SearchIndex,
    generation: u64,
    initialized: bool,
    offset: u64,
    total: u64,
    error: Option<String>,
}

impl OpenSession {
    fn apply(&mut self, update: Update) {
        match update {
            Update::Error(error) => self.error = Some(error),
            Update::Batch {
                meta,
                stats,
                revision,
                turns,
                reset,
                offset,
                total,
            } => {
                if reset {
                    self.session = Session::default();
                    self.index = SearchIndex::default();
                    if self.initialized {
                        self.generation += 1;
                    }
                }
                self.initialized = true;
                self.error = None;
                self.offset = offset;
                self.total = total;
                self.session.meta = meta;
                self.session.parse_stats = stats;
                self.session.revision = revision;
                for (i, turn) in turns {
                    self.index.update(i, &turn);
                    if self.session.turns.len() <= i {
                        self.session.turns.resize(i + 1, Turn::default());
                    }
                    self.session.turns[i] = turn;
                }
            }
        }
    }
}

type ScanResult = (bool, Result<Vec<SessionSummary>>);

struct Backend {
    home: PathBuf,
    cwd: PathBuf,
    config: Config,
    default_all: bool,
    initial_session: Option<String>,
    registered: HashMap<String, Registered>,
    paths: HashMap<PathBuf, String>,
    listings: [Option<Vec<Value>>; 2],
    scan_error: [Option<String>; 2],
    pending: [bool; 2],
    scan: Option<(bool, mpsc::Receiver<ScanResult>)>,
    open: HashMap<String, OpenSession>,
    lru: VecDeque<String>,
    next_generation: u64,
}

impl Backend {
    fn new(home: PathBuf, cwd: PathBuf, config: Config, default_all: bool) -> Self {
        Self {
            home,
            cwd,
            config,
            default_all,
            initial_session: None,
            registered: HashMap::new(),
            paths: HashMap::new(),
            listings: [None, None],
            scan_error: [None, None],
            pending: [false, false],
            scan: None,
            open: HashMap::new(),
            lru: VecDeque::new(),
            next_generation: 1,
        }
    }

    fn register(&mut self, path: PathBuf, explicit: bool) -> Result<String> {
        let path = path
            .canonicalize()
            .context("Cannot resolve selected session")?;
        if !path.is_file() {
            anyhow::bail!("Session is not a regular file");
        }
        if !explicit && !path.starts_with(self.home.join("sessions").canonicalize()?) {
            anyhow::bail!("Session is outside the allowed directory");
        }
        if let Some(key) = self.paths.get(&path) {
            return Ok(key.clone());
        }
        if self.registered.len() >= MAX_SESSIONS {
            anyhow::bail!("Session registry is full");
        }
        let key = format!("s{:x}", self.registered.len() + 1);
        self.paths.insert(path.clone(), key.clone());
        self.registered
            .insert(key.clone(), Registered { path, explicit });
        Ok(key)
    }

    fn safe_target(&self, key: &str) -> bool {
        self.registered.get(key).is_some_and(|registered| {
            let Ok(current) = registered.path.canonicalize() else {
                return false;
            };
            current == registered.path
                && current.is_file()
                && (registered.explicit
                    || self
                        .home
                        .join("sessions")
                        .canonicalize()
                        .is_ok_and(|root| current.starts_with(root)))
        })
    }

    fn start_scan(&mut self, all: bool) {
        if let Some((active, _)) = &self.scan {
            if *active != all {
                self.pending[usize::from(all)] = true;
            }
            return;
        }
        self.scan_error[usize::from(all)] = None;
        let (home, cwd, config) = (self.home.clone(), self.cwd.clone(), self.config.clone());
        let (sender, receiver) = mpsc::sync_channel(1);
        thread::spawn(move || {
            let _ = sender.send((
                all,
                discovery::discover(&home, &cwd, all, &config).map(discovery::main_sessions),
            ));
        });
        self.scan = Some((all, receiver));
    }

    fn tick(&mut self) {
        let result = self
            .scan
            .as_ref()
            .and_then(|(_, receiver)| receiver.try_recv().ok());
        if let Some((all, result)) = result {
            self.scan = None;
            match result {
                Ok(sessions) => {
                    let mut listing = Vec::new();
                    let mut truncated = sessions.len() > MAX_SESSIONS;
                    for summary in sessions.into_iter().take(MAX_SESSIONS) {
                        if let Ok(key) = self.register(summary.path, false) {
                            listing.push(json!({"key":key,"id":summary.id,"title":summary.title,"cwd":summary.cwd.map(|p|p.to_string_lossy().into_owned()),"updated_at":summary.updated_at.map(|t|t.to_rfc3339()),"turn_count":summary.turn_count,"first_prompt":summary.first_prompt}));
                        } else if self.registered.len() >= MAX_SESSIONS {
                            truncated = true;
                        }
                    }
                    self.listings[usize::from(all)] = Some(listing);
                    self.scan_error[usize::from(all)] = truncated.then(|| "会话登记达到 20,000 项上限，部分会话未显示；请重启服务，或使用 --session 直接打开。".into());
                }
                Err(_) => {
                    self.scan_error[usize::from(all)] =
                        Some("无法读取会话目录，请检查本地权限后刷新。".into())
                }
            }
            if let Some(next) = self.pending.iter().position(|pending| *pending) {
                self.pending[next] = false;
                self.start_scan(next == 1);
            }
        }
        let keys: Vec<_> = self.open.keys().cloned().collect();
        for key in keys {
            if !self.safe_target(&key) {
                self.open.remove(&key);
                self.lru.retain(|entry| entry != &key);
                continue;
            }
            let open = self.open.get_mut(&key).expect("registered open session");
            while let Ok(update) = open.worker.updates.try_recv() {
                open.apply(update);
            }
        }
    }

    fn get_session(&mut self, key: &str, refresh: bool) -> Result<&mut OpenSession> {
        if !self.safe_target(key) {
            anyhow::bail!("会话不存在或文件目标已改变，请刷新会话列表");
        }
        if !self.open.contains_key(key) {
            if self.open.len() >= MAX_OPEN {
                if let Some(oldest) = self.lru.pop_front() {
                    self.open.remove(&oldest);
                }
            }
            let path = self.registered[key].path.clone();
            let generation = self.next_generation;
            // Reserve a wide epoch for resets before the next cache incarnation.
            self.next_generation += 1 << 32;
            self.open.insert(
                key.into(),
                OpenSession {
                    worker: SessionWorker::start(path, self.config.clone()),
                    session: Session::default(),
                    index: SearchIndex::default(),
                    generation,
                    initialized: false,
                    offset: 0,
                    total: 0,
                    error: None,
                },
            );
        }
        self.lru.retain(|entry| entry != key);
        self.lru.push_back(key.into());
        let open = self.open.get_mut(key).expect("session inserted");
        if refresh {
            open.worker.refresh();
        }
        while let Ok(update) = open.worker.updates.try_recv() {
            open.apply(update);
        }
        Ok(open)
    }

    fn route(&mut self, path: &str, query: &HashMap<String, String>) -> ApiReply {
        self.tick();
        if path == "/api/info" {
            return ApiReply::json(
                json!({"version":env!("CARGO_PKG_VERSION"),"watch":self.config.watch,"default_all":self.default_all,"initial_session":self.initial_session,"refresh_ms":750}),
            );
        }
        if path == "/api/sessions" {
            let all = match query.get("all").map(String::as_str) {
                None => self.default_all,
                Some("0") => false,
                Some("1") => true,
                _ => return ApiReply::error(StatusCode::BAD_REQUEST, "all 必须是 0 或 1"),
            };
            let i = usize::from(all);
            if query.get("refresh").is_some_and(|s| s == "1")
                || (self.listings[i].is_none() && self.scan_error[i].is_none())
            {
                self.start_scan(all);
            }
            return ApiReply::json(
                json!({"loading":self.scan.as_ref().is_some_and(|(active,_)| *active == all)||self.pending[i],"error":self.scan_error[i],"sessions":self.listings[i].as_deref().unwrap_or(&[])}),
            );
        }
        let parts: Vec<_> = path.trim_start_matches('/').split('/').collect();
        if parts.len() < 3 || parts[0] != "api" || parts[1] != "session" {
            return ApiReply::error(StatusCode::NOT_FOUND, "接口不存在");
        }
        if parts.len() > 6 {
            return ApiReply::error(StatusCode::NOT_FOUND, "接口不存在");
        }
        let key = parts[2];
        let watch = self.config.watch;
        let open = match self.get_session(key, query.get("refresh").is_some_and(|s| s == "1")) {
            Ok(open) => open,
            Err(_) => {
                return ApiReply::error(
                    StatusCode::NOT_FOUND,
                    "会话不存在或文件不可读取，请刷新会话列表",
                )
            }
        };
        let session = &open.session;
        if parts.len() == 3 {
            let stats = &session.parse_stats;
            return ApiReply::json(
                json!({"key":key,"meta":{"id":session.meta.id,"cwd":session.meta.cwd.as_ref().map(|p|p.to_string_lossy()),"kind":session.meta.identity.kind.label()},"generation":open.generation,"revision":session.revision,"loading":(!open.initialized || open.offset < open.total)&&open.error.is_none(),"offset":open.offset,"total_bytes":open.total,"turn_count":session.turns.len(),"latest_active":session.latest_active(),"stats":{"records":stats.records,"malformed_records":stats.malformed_records,"skipped_oversize_records":stats.skipped_oversize_records,"omitted_text_bytes":stats.omitted_text_bytes,"unknown_records":stats.unknown_records},"error":open.error,"watch":watch}),
            );
        }
        if parts.len() == 4 && parts[3] == "turns" {
            let (offset, limit) = match pagination(query, 100) {
                Ok(v) => v,
                Err(e) => return e,
            };
            let indices = open.index.search(query.get("q").map_or("", String::as_str));
            let turns: Vec<_> = indices
                .iter()
                .skip(offset)
                .take(limit)
                .map(|&i| turn_summary(i, &session.turns[i]))
                .collect();
            return ApiReply::json(
                json!({"generation":open.generation,"revision":session.revision,"total":indices.len(),"offset":offset,"turns":turns}),
            );
        }
        if parts.len() >= 5 && parts[3] == "turn" {
            let Some((index, turn)) = parts[4]
                .parse::<usize>()
                .ok()
                .and_then(|index| session.turns.get(index).map(|turn| (index, turn)))
            else {
                return ApiReply::error(StatusCode::NOT_FOUND, "轮次不存在或尚未加载");
            };
            if parts.len() == 6 && parts[5] == "text" {
                return ApiReply {
                    status: StatusCode::OK,
                    content_type: "text/plain; charset=utf-8",
                    body: turn_text(turn).into_bytes(),
                };
            }
            if parts.len() != 5 {
                return ApiReply::error(StatusCode::NOT_FOUND, "接口不存在");
            }
            let (offset, limit) = match pagination(query, 8) {
                Ok(v) => v,
                Err(e) => return e,
            };
            let items: Vec<_> = turn
                .items
                .iter()
                .enumerate()
                .skip(offset)
                .take(limit)
                .map(|(i, item)| item_value(i, item))
                .collect();
            let next = offset.saturating_add(items.len());
            let final_answer = turn
                .items
                .iter()
                .enumerate()
                .rev()
                .find_map(|(index, item)| match item {
                    TurnItem::AgentMessage {
                        text,
                        phase: Some(phase),
                    } if phase == "final_answer" => {
                        Some(json!({"index":index,"text":text,"phase":phase}))
                    }
                    _ => None,
                });
            let activity = &turn.activity;
            return ApiReply::json(
                json!({"generation":open.generation,"revision":session.revision,"turn":{"index":index,"ordinal":turn.ordinal,"id":turn.id,"revision":turn.revision,"prompt":{"text":turn.prompt.text,"preview":turn.prompt.preview,"images_count":turn.prompt.images_count,"omitted_bytes":turn.prompt.omitted_bytes},"status":status_name(turn.status),"activity":{"commands":activity.commands,"tool_calls":activity.tool_calls,"files_read":activity.files_read,"files_changed":activity.files_changed,"errors":activity.errors},"started_at":turn.started_at.map(|t|t.to_rfc3339()),"completed_at":turn.completed_at.map(|t|t.to_rfc3339())},"items":items,"items_total":turn.items.len(),"next_offset":(next<turn.items.len()).then_some(next),"final_answer":final_answer}),
            );
        }
        ApiReply::error(StatusCode::NOT_FOUND, "接口不存在")
    }
}

fn pagination(
    query: &HashMap<String, String>,
    maximum: usize,
) -> std::result::Result<(usize, usize), ApiReply> {
    let parse = |name: &str, default: usize| -> std::result::Result<usize, ApiReply> {
        query.get(name).map_or(Ok(default), |value| {
            value
                .parse()
                .map_err(|_| ApiReply::error(StatusCode::BAD_REQUEST, "分页参数必须是非负整数"))
        })
    };
    let offset = parse("offset", 0)?;
    let limit = parse("limit", maximum)?.clamp(1, maximum);
    Ok((offset, limit))
}

fn status_name(status: TurnStatus) -> &'static str {
    match status {
        TurnStatus::InProgress => "in_progress",
        TurnStatus::Completed => "completed",
        TurnStatus::Failed => "failed",
        TurnStatus::Interrupted => "interrupted",
        TurnStatus::Unknown => "unknown",
        TurnStatus::RolledBack => "rolled_back",
    }
}

fn turn_summary(index: usize, turn: &Turn) -> Value {
    json!({"index":index,"ordinal":turn.ordinal,"preview":turn.prompt.preview,"status":status_name(turn.status),"errors":turn.activity.errors,"revision":turn.revision,"started_at":turn.started_at.map(|t|t.to_rfc3339()),"has_final":turn.items.iter().any(|item| matches!(item,TurnItem::AgentMessage {phase:Some(phase),..} if phase=="final_answer")),"omitted_bytes":turn.prompt.omitted_bytes})
}

fn item_value(index: usize, item: &TurnItem) -> Value {
    match item {
        TurnItem::AgentMessage { text, phase } => {
            json!({"index":index,"type":"agent_message","text":text,"phase":phase})
        }
        TurnItem::ToolCall { name, summary } => {
            json!({"index":index,"type":"tool_call","name":name,"summary":summary})
        }
        TurnItem::ToolOutput { summary, is_error } => {
            json!({"index":index,"type":"tool_output","summary":summary,"is_error":is_error})
        }
        TurnItem::FileActivity { path, kind } => {
            json!({"index":index,"type":"file_activity","path":path,"kind":kind})
        }
        TurnItem::Notice { text } => json!({"index":index,"type":"notice","text":text}),
        TurnItem::Omitted => json!({"index":index,"type":"omitted"}),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn http_state() -> HttpState {
        let (sender, _) = mpsc::sync_channel(1);
        HttpState {
            authority: "127.0.0.1:8765".into(),
            token: "a".repeat(64),
            sender,
            capacity: Arc::new(Semaphore::new(1)),
        }
    }

    #[test]
    fn authentication_rejects_cross_origin_duplicate_and_missing_headers() {
        let state = http_state();
        let mut headers = HeaderMap::new();
        headers.insert("host", HeaderValue::from_static("127.0.0.1:8765"));
        assert!(authorized(&headers, &state, false));
        assert!(!authorized(&headers, &state, true));
        headers.insert("x-codex-nav-token", state.token.parse().unwrap());
        assert!(authorized(&headers, &state, true));
        headers.insert("origin", HeaderValue::from_static("http://evil.invalid"));
        assert!(!authorized(&headers, &state, true));
        headers.insert("origin", HeaderValue::from_static("http://127.0.0.1:8765"));
        assert!(authorized(&headers, &state, true));
        headers.append("x-codex-nav-token", state.token.parse().unwrap());
        assert!(!authorized(&headers, &state, true));
    }

    #[test]
    fn public_navigation_allows_cross_site_but_api_still_rejects_it() {
        let state = http_state();
        let mut headers = HeaderMap::new();
        headers.insert("host", HeaderValue::from_static("127.0.0.1:8765"));
        headers.insert("sec-fetch-site", HeaderValue::from_static("cross-site"));
        headers.insert("sec-fetch-mode", HeaderValue::from_static("navigate"));
        headers.insert("sec-fetch-dest", HeaderValue::from_static("document"));
        assert!(authorized(&headers, &state, false));
        headers.insert("x-codex-nav-token", state.token.parse().unwrap());
        assert!(!authorized(&headers, &state, true));
        headers.insert("origin", HeaderValue::from_static("http://evil.invalid"));
        assert!(!authorized(&headers, &state, false));
        headers.remove("origin");
        headers.insert("host", HeaderValue::from_static("evil.invalid"));
        assert!(!authorized(&headers, &state, false));
    }

    #[test]
    fn pagination_is_bounded_and_rejects_negative_or_overflow_values() {
        let mut query = HashMap::new();
        query.insert("limit".into(), "9999".into());
        assert!(matches!(pagination(&query, 8), Ok((0, 8))));
        query.insert("offset".into(), "-1".into());
        assert!(pagination(&query, 8).is_err());
        query.insert("offset".into(), "999999999999999999999999999999999".into());
        assert!(pagination(&query, 8).is_err());
    }

    #[test]
    fn lifecycle_and_warnings_and_final_markers_are_independent() {
        let mut turn = Turn {
            status: TurnStatus::Completed,
            ..Turn::default()
        };
        turn.activity.errors = 2;
        turn.items.push(TurnItem::AgentMessage {
            text: "not final".into(),
            phase: None,
        });
        let summary = turn_summary(7, &turn);
        assert_eq!(summary["status"], "completed");
        assert_eq!(summary["errors"], 2);
        assert_eq!(summary["has_final"], false);
        turn.items.push(TurnItem::AgentMessage {
            text: "final".into(),
            phase: Some("final_answer".into()),
        });
        assert_eq!(turn_summary(7, &turn)["has_final"], true);
    }

    #[test]
    fn registered_files_cannot_escape_discovery_root() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir(temp.path().join("sessions")).unwrap();
        let outside = temp.path().join("secret.jsonl");
        std::fs::write(&outside, b"secret").unwrap();
        let mut backend = Backend::new(
            temp.path().into(),
            temp.path().into(),
            Config::default(),
            true,
        );
        assert!(backend.register(outside.clone(), false).is_err());
        let key = backend.register(outside, true).unwrap();
        assert!(backend.safe_target(&key));
        assert!(!backend.safe_target("../../secret.jsonl"));
    }

    #[test]
    fn batch_reset_changes_generation_and_replaces_index() {
        let temp = tempfile::NamedTempFile::new().unwrap();
        let mut open = OpenSession {
            worker: SessionWorker::start(temp.path().into(), Config::default()),
            session: Session::default(),
            index: SearchIndex::default(),
            generation: 9,
            initialized: false,
            offset: 0,
            total: 0,
            error: None,
        };
        let mut turn = Turn::default();
        turn.prompt.text = "old prompt".into();
        let batch = |turns| Update::Batch {
            meta: Default::default(),
            stats: Default::default(),
            revision: 1,
            turns,
            reset: true,
            offset: 10,
            total: 10,
        };
        open.apply(batch(vec![(0, turn)]));
        assert_eq!(open.generation, 9);
        assert_eq!(open.index.search("old"), vec![0]);
        open.apply(batch(vec![]));
        assert_eq!(open.generation, 10);
        assert!(open.session.turns.is_empty());
        assert!(open.index.search("old").is_empty());
    }

    #[test]
    fn cache_evicts_least_recent_session_and_uses_new_generation() {
        let temp = tempfile::tempdir().unwrap();
        let mut backend = Backend::new(
            temp.path().into(),
            temp.path().into(),
            Config::default(),
            true,
        );
        let keys: Vec<_> = (0..3)
            .map(|i| {
                let path = temp.path().join(format!("session-{i}.jsonl"));
                std::fs::write(&path, b"").unwrap();
                backend.register(path, true).unwrap()
            })
            .collect();
        let original = backend.get_session(&keys[0], false).unwrap().generation;
        backend.get_session(&keys[1], false).unwrap();
        backend.get_session(&keys[2], false).unwrap();
        assert_eq!(backend.open.len(), MAX_OPEN);
        assert!(!backend.open.contains_key(&keys[0]));
        assert_ne!(
            backend.get_session(&keys[0], false).unwrap().generation,
            original
        );
    }

    #[test]
    fn omitted_activity_and_prompt_remain_explicit() {
        let turn = Turn {
            prompt: crate::domain::UserPrompt {
                preview: "retained preview".into(),
                omitted_bytes: 123,
                ..Default::default()
            },
            items: vec![TurnItem::Omitted],
            ..Default::default()
        };
        assert_eq!(turn_summary(0, &turn)["omitted_bytes"], 123);
        assert_eq!(item_value(0, &turn.items[0])["type"], "omitted");
    }

    #[tokio::test]
    async fn response_budget_is_held_until_body_is_consumed_or_dropped() {
        use futures_util::StreamExt;
        let capacity = Arc::new(Semaphore::new(1));
        let permit = capacity.clone().acquire_owned().await.unwrap();
        let response = ApiReply {
            status: StatusCode::OK,
            content_type: "text/plain",
            body: vec![b'x'; 128 * 1024],
        }
        .response_with_permit(Some(permit));
        assert_eq!(capacity.available_permits(), 0);
        let mut body = response.into_body().into_data_stream();
        assert_eq!(body.next().await.unwrap().unwrap().len(), 16 * 1024);
        assert_eq!(capacity.available_permits(), 0);
        drop(body);
        assert_eq!(capacity.available_permits(), 1);
    }

    #[tokio::test]
    async fn connection_deadline_expires_even_without_client_data() {
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let client = tokio::net::TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        let (stream, _) = listener.accept().await.unwrap();
        let capacity = Arc::new(Semaphore::new(1));
        let mut io = LimitedIo {
            stream,
            _permit: capacity.clone().acquire_owned().await.unwrap(),
            deadline: Box::pin(tokio::time::sleep(Duration::from_millis(1))),
        };
        let mut buffer = [0_u8; 1];
        let mut read_buffer = ReadBuf::new(&mut buffer);
        let error = futures_util::future::poll_fn(|context| {
            Pin::new(&mut io).poll_read(context, &mut read_buffer)
        })
        .await
        .unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
        drop(io);
        drop(client);
        assert_eq!(capacity.available_permits(), 1);
    }

    #[test]
    fn exhausted_process_registry_is_reported_after_refresh() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir(temp.path().join("sessions")).unwrap();
        let path = temp.path().join("sessions/new.jsonl");
        std::fs::write(&path, b"").unwrap();
        let mut backend = Backend::new(
            temp.path().into(),
            temp.path().into(),
            Config::default(),
            true,
        );
        for i in 0..MAX_SESSIONS {
            backend.registered.insert(
                format!("old-{i}"),
                Registered {
                    path: temp.path().join(format!("old-{i}")),
                    explicit: false,
                },
            );
        }
        let (sender, receiver) = mpsc::sync_channel(1);
        sender
            .send((
                true,
                Ok(vec![SessionSummary {
                    path,
                    ..Default::default()
                }]),
            ))
            .unwrap();
        backend.scan = Some((true, receiver));
        backend.tick();
        assert!(backend.listings[1].as_ref().unwrap().is_empty());
        assert!(backend.scan_error[1].as_ref().unwrap().contains("20,000"));
    }
}
