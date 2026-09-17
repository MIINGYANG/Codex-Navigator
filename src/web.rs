//! Loopback-only reader with explicitly authenticated session management actions.
use crate::{
    app::turn_text,
    config::Config,
    discovery,
    domain::{Session, SessionSummary, Turn, TurnItem, TurnStatus},
    favorites,
    index::SearchIndex,
    management,
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
    collections::{HashMap, HashSet, VecDeque},
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

#[path = "trail.rs"]
mod trail;

const MAX_OPEN: usize = 2;
const MAX_API_REQUESTS: usize = 4;
const MAX_EVENT_CONNECTIONS: usize = 8;
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
    events_capacity: Arc<Semaphore>,
}

struct ApiRequest {
    path: String,
    query: HashMap<String, String>,
    reply: oneshot::Sender<ApiReply>,
    mutation: Option<Value>,
}

fn request_channel() -> (mpsc::SyncSender<ApiRequest>, mpsc::Receiver<ApiRequest>) {
    // Both admission classes enqueue one state request at a time. Reserve space
    // for every admitted API request even when all SSE polls arrive together.
    mpsc::sync_channel(MAX_API_REQUESTS + MAX_EVENT_CONNECTIONS)
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
    run_inner(home, cwd, config, options)
}

/// Question Trail shares the same transport and session parser.
pub fn run_trail(home: PathBuf, cwd: PathBuf, config: Config, options: WebOptions) -> Result<()> {
    run(home, cwd, config, options)
}

fn run_inner(home: PathBuf, cwd: PathBuf, config: Config, options: WebOptions) -> Result<()> {
    let mut entropy = [0_u8; 32];
    getrandom::fill(&mut entropy).map_err(|_| anyhow::anyhow!("Cannot obtain secure Web token"))?;
    let token: String = entropy.iter().map(|byte| format!("{byte:02x}")).collect();
    // The web session list always spans all projects and dates. `all` remains a
    // compatible option for callers; the terminal picker keeps its own scope.
    let mut backend = Backend::new(home, cwd, config, true);
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
        let (sender, receiver) = request_channel();
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
                            let ApiRequest { path, query, reply, mutation } = request;
                            if !reply.is_closed() {
                                let response = match mutation {
                                    Some(body) => backend.mutate(&path, body),
                                    None => backend.route(&path, &query),
                                };
                                let _ = reply.send(response);
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
            capacity: Arc::new(Semaphore::new(MAX_API_REQUESTS)),
            events_capacity: Arc::new(Semaphore::new(MAX_EVENT_CONNECTIONS)),
        };
        let router = Router::new().fallback(handle).with_state(state);
        println!(
            "Codex Navigator Web · Question Trail — 本机浏览与会话管理，无模型调用，Ctrl+C 停止\n{url}"
        );
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
    if request.method() == Method::POST && management_target(path).is_some() {
        return manage_request(state, request).await;
    }
    if request.method() != Method::GET {
        return ApiReply::error(StatusCode::METHOD_NOT_ALLOWED, "此地址仅支持 GET 请求").response();
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
        "/" | "/index.html" | "/trail/" | "/trail" | "/trail/index.html" => Some((
            "text/html; charset=utf-8",
            include_str!("../trail/dist/index.html"),
        )),
        "/trail/assets/trail.js" => Some((
            "text/javascript; charset=utf-8",
            include_str!("../trail/dist/assets/trail.js"),
        )),
        "/trail/assets/trail.css" => Some((
            "text/css; charset=utf-8",
            include_str!("../trail/dist/assets/trail.css"),
        )),
        "/trail/theme-init.js" | "/theme-init.js" => Some((
            "text/javascript; charset=utf-8",
            include_str!("../trail/dist/theme-init.js"),
        )),
        "/trail/favicon.svg" | "/favicon.svg" => {
            Some(("image/svg+xml", include_str!("../trail/dist/favicon.svg")))
        }
        _ => None,
    };
    if let Some((content_type, body)) = resource {
        let mut response = ApiReply {
            status: StatusCode::OK,
            content_type,
            body: body.as_bytes().to_vec(),
        }
        .response();
        response.headers_mut().insert(header::CONTENT_SECURITY_POLICY, HeaderValue::from_static("default-src 'none'; script-src 'self'; style-src 'self'; style-src-attr 'unsafe-inline'; connect-src 'self'; img-src 'self' data:; font-src 'self'; base-uri 'none'; frame-ancestors 'none'; form-action 'none'"));
        return response;
    }
    if !path.starts_with("/api/") {
        return ApiReply::error(StatusCode::NOT_FOUND, "页面不存在").response();
    }
    if path == "/api/trail/events" {
        return trail_events(state, request).await;
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
            mutation: None,
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

fn management_target(path: &str) -> Option<(&str, &str)> {
    let rest = path.strip_prefix("/api/session/")?;
    let (key, action) = rest.split_once('/')?;
    if key.is_empty()
        || key.len() > 64
        || !key.bytes().all(|v| v.is_ascii_alphanumeric())
        || !matches!(action, "rename" | "trash" | "favorite")
    {
        return None;
    }
    Some((key, action))
}

async fn manage_request(state: HttpState, request: Request) -> Response {
    if unique_header(request.headers(), "origin")
        != Some(format!("http://{}", state.authority).as_str())
    {
        return ApiReply::error(StatusCode::FORBIDDEN, "管理操作必须来自本机网页").response();
    }
    if request.uri().query().is_some()
        || unique_header(request.headers(), "content-type")
            .is_none_or(|v| v.split(';').next().map(str::trim) != Some("application/json"))
    {
        return ApiReply::error(
            StatusCode::BAD_REQUEST,
            "管理操作需要 JSON 正文且不能携带查询参数",
        )
        .response();
    }
    let Ok(permit) = state.capacity.clone().try_acquire_owned() else {
        return ApiReply::error(StatusCode::SERVICE_UNAVAILABLE, "请求繁忙，请稍后重试").response();
    };
    let path = request.uri().path().to_owned();
    let bytes = match tokio::time::timeout(
        Duration::from_secs(5),
        axum::body::to_bytes(request.into_body(), 4096),
    )
    .await
    {
        Ok(Ok(bytes)) => bytes,
        _ => {
            return ApiReply::error(StatusCode::BAD_REQUEST, "请求正文无效或超过 4 KiB").response()
        }
    };
    let Ok(body) = serde_json::from_slice::<Value>(&bytes) else {
        return ApiReply::error(StatusCode::BAD_REQUEST, "JSON 正文无效").response();
    };
    let (reply, receiver) = oneshot::channel();
    if state
        .sender
        .try_send(ApiRequest {
            path,
            query: HashMap::new(),
            reply,
            mutation: Some(body),
        })
        .is_err()
    {
        return ApiReply::error(StatusCode::SERVICE_UNAVAILABLE, "服务暂不可用").response();
    }
    match tokio::time::timeout(Duration::from_secs(30), receiver).await {
        Ok(Ok(reply)) => reply.response_with_permit(Some(permit)),
        _ => ApiReply::error(
            StatusCode::SERVICE_UNAVAILABLE,
            "管理操作超时，请刷新核对会话状态后重试",
        )
        .response(),
    }
}

struct Registered {
    path: PathBuf,
    explicit: bool,
}

async fn trail_events(state: HttpState, request: Request) -> Response {
    let Ok(Query(query)) = Query::<HashMap<String, String>>::try_from_uri(request.uri()) else {
        return ApiReply::error(StatusCode::BAD_REQUEST, "查询参数无效").response();
    };
    let Some(key) = query.get("session").filter(|key| {
        !key.is_empty() && key.len() <= 64 && key.bytes().all(|c| c.is_ascii_alphanumeric())
    }) else {
        return ApiReply::error(StatusCode::BAD_REQUEST, "需要有效的 session").response();
    };
    let Ok(permit) = state.events_capacity.clone().try_acquire_owned() else {
        return ApiReply::error(StatusCode::SERVICE_UNAVAILABLE, "实时连接已达到上限").response();
    };
    let path = format!("/api/session/{key}");
    let started = std::time::Instant::now();
    let stream = futures_util::stream::unfold(
        (state, path, None::<Vec<u8>>, started, permit),
        |(state, path, last, started, permit)| async move {
            if started.elapsed() > Duration::from_secs(50) {
                return None;
            }
            if last.is_some() {
                tokio::time::sleep(Duration::from_millis(750)).await;
            }
            let (reply, receiver) = oneshot::channel();
            if state
                .sender
                .try_send(ApiRequest {
                    path: path.clone(),
                    query: HashMap::new(),
                    reply,
                    mutation: None,
                })
                .is_err()
            {
                return None;
            }
            let Ok(Ok(reply)) = tokio::time::timeout(Duration::from_secs(5), receiver).await else {
                return None;
            };
            if reply.status != StatusCode::OK {
                return None;
            }
            let changed = last.as_ref() != Some(&reply.body);
            let data = if changed {
                format!(
                    "event: change\ndata: {}\n\n",
                    String::from_utf8_lossy(&reply.body)
                )
            } else {
                ": keep-alive\n\n".into()
            };
            Some((
                Ok::<_, std::convert::Infallible>(Bytes::from(data)),
                (state, path, Some(reply.body), started, permit),
            ))
        },
    );
    let mut response = ApiReply {
        status: StatusCode::OK,
        content_type: "text/event-stream; charset=utf-8",
        body: Vec::new(),
    }
    .response();
    response.headers_mut().remove(header::CONTENT_LENGTH);
    response
        .headers_mut()
        .insert("x-accel-buffering", HeaderValue::from_static("no"));
    *response.body_mut() = Body::from_stream(stream);
    response
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
                events,
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
                self.session.events = events;
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
    trashed: HashSet<String>,
    names: HashMap<String, String>,
    paths: HashMap<PathBuf, String>,
    listings: [Option<Vec<Value>>; 2],
    scan_error: [Option<String>; 2],
    pending: [bool; 2],
    scan: Option<(bool, mpsc::Receiver<ScanResult>)>,
    open: HashMap<String, OpenSession>,
    lru: VecDeque<String>,
    next_generation: u64,
    search: trail::SearchSnapshot,
    search_worker: Option<mpsc::Receiver<(trail::SearchSnapshot, bool)>>,
    search_signature: HashMap<String, (u64, Option<std::time::SystemTime>)>,
    search_checked: std::time::Instant,
    search_deferred: bool,
    search_refresh_requested: bool,
}

impl Backend {
    fn ensure_search(&mut self) {
        if self.search_worker.is_some()
            || (!self.config.watch
                && !self.search_refresh_requested
                && !self.search_deferred
                && !self.search_signature.is_empty())
            || (!self.search_signature.is_empty()
                && !self.search_refresh_requested
                && self.search_checked.elapsed() < Duration::from_secs(3))
        {
            return;
        }
        self.search_checked = std::time::Instant::now();
        self.search_deferred = false;
        if self.listings[1].is_none() {
            self.start_scan(true);
            return;
        }
        self.search_refresh_requested = false;
        let keys: Vec<_> = self.listings[1]
            .as_ref()
            .into_iter()
            .flatten()
            .filter_map(|v| v["key"].as_str().map(str::to_owned))
            .collect();
        let mut signature = HashMap::new();
        let mut paths = Vec::new();
        let mut reusable = std::collections::HashSet::new();
        for key in keys {
            if !self.safe_target(&key) {
                continue;
            }
            let path = &self.registered[&key].path;
            if let Ok(meta) = path.metadata() {
                let stamp = (meta.len(), meta.modified().ok());
                if self
                    .search_signature
                    .get(&key)
                    .is_none_or(|(old_len, _)| *old_len < stamp.0)
                {
                    reusable.insert(key.clone());
                }
                if self.search_signature.get(&key) != Some(&stamp) {
                    paths.push((key.clone(), path.clone()));
                }
                signature.insert(key, stamp);
            }
        }
        if signature != self.search_signature {
            self.search
                .rows
                .retain(|row| signature.contains_key(&row.key));
            self.search
                .counts
                .retain(|key, _| signature.contains_key(key));
            self.search
                .event_summaries
                .retain(|key, _| signature.contains_key(key));
            self.search_signature = signature;
            paths.retain(|(key, _)| {
                let deferred = reusable.contains(key)
                    && self.open.get(key).is_some_and(|open| {
                        open.error.is_none()
                            && (!open.initialized
                                || open.offset != open.total
                                || !self
                                    .search_signature
                                    .get(key)
                                    .is_some_and(|(len, _)| *len == open.total))
                    });
                if deferred {
                    // Preserve and budget the old rows while the existing reader catches up.
                    self.search_signature.remove(key);
                    self.search_deferred = true;
                }
                !deferred
            });
            let changed: std::collections::HashSet<_> =
                paths.iter().map(|(key, _)| key.clone()).collect();
            self.search
                .event_summaries
                .retain(|key, _| !changed.contains(key));
            let unchanged = self
                .search
                .rows
                .iter()
                .filter(|row| !changed.contains(&row.key));
            let mut retained = unchanged.clone().map(|row| row.searchable.len()).sum();
            let mut rows = unchanged.count();
            paths.retain(|(key, _)| {
                if let Some(open) = self.open.get(key).filter(|open| {
                    reusable.contains(key)
                        && open.initialized
                        && open.offset == open.total
                        && open.error.is_none()
                        && self
                            .search_signature
                            .get(key)
                            .is_some_and(|(len, _)| *len == open.total)
                }) {
                    let snapshot = trail::project(&open.session, key, &mut retained, &mut rows);
                    self.search.rows.retain(|row| row.key != *key);
                    self.search.rows.extend(snapshot.rows);
                    self.search.counts.extend(snapshot.counts);
                    self.search.event_summaries.extend(snapshot.event_summaries);
                    self.search.truncated |= snapshot.truncated;
                    false
                } else {
                    true
                }
            });
            if !paths.is_empty() {
                self.search_worker = Some(trail::start_index(
                    paths,
                    self.config.clone(),
                    retained,
                    rows,
                ));
            }
        }
    }
    fn new(home: PathBuf, cwd: PathBuf, config: Config, default_all: bool) -> Self {
        Self {
            home,
            cwd,
            config,
            default_all,
            initial_session: None,
            registered: HashMap::new(),
            trashed: HashSet::new(),
            names: HashMap::new(),
            paths: HashMap::new(),
            listings: [None, None],
            scan_error: [None, None],
            pending: [false, false],
            scan: None,
            open: HashMap::new(),
            lru: VecDeque::new(),
            next_generation: 1,
            search: trail::SearchSnapshot::default(),
            search_worker: None,
            search_signature: HashMap::new(),
            search_checked: std::time::Instant::now() - Duration::from_secs(10),
            search_deferred: false,
            search_refresh_requested: false,
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
        !self.trashed.contains(key)
            && self.registered.get(key).is_some_and(|registered| {
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
                Ok(mut sessions) => {
                    self.names = discovery::session_names(&self.home);
                    sessions.sort_by(|a, b| {
                        b.updated_at
                            .cmp(&a.updated_at)
                            .then_with(|| a.id.cmp(&b.id))
                    });
                    let mut listing = Vec::new();
                    let mut truncated = sessions.len() > MAX_SESSIONS;
                    for summary in sessions.into_iter().take(MAX_SESSIONS) {
                        if let Ok(key) = self.register(summary.path, false) {
                            if self.trashed.contains(&key) || !self.safe_target(&key) {
                                continue;
                            }
                            listing.push(json!({"key":key,"id":summary.id,"title":self.names.get(&summary.id).or(summary.title.as_ref()),"cwd":summary.cwd.map(|p|p.to_string_lossy().into_owned()),"updated_at":summary.updated_at.map(|t|t.to_rfc3339()),"turn_count":summary.turn_count,"first_prompt":summary.first_prompt}));
                        } else if self.registered.len() >= MAX_SESSIONS {
                            truncated = true;
                        }
                    }
                    self.listings[usize::from(all)] = Some(listing);
                    self.search_refresh_requested = true;
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
        while let Some((snapshot, done)) = self
            .search_worker
            .as_ref()
            .and_then(|rx| rx.try_recv().ok())
        {
            self.search
                .rows
                .retain(|row| !snapshot.counts.contains_key(&row.key));
            self.search
                .event_summaries
                .retain(|key, _| !snapshot.counts.contains_key(key));
            self.search.rows.extend(snapshot.rows);
            self.search.counts.extend(snapshot.counts);
            self.search.event_summaries.extend(snapshot.event_summaries);
            self.search.truncated |= snapshot.truncated;
            if done {
                self.search_worker = None;
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

    fn restart_search_index(&mut self) {
        // Dropping an in-flight index also drops other sessions' pending results.
        // Forget every signature so unchanged survivors are scheduled again.
        self.search_worker = None;
        self.search_signature.clear();
        self.search_refresh_requested = true;
    }

    fn favorite(&mut self, key: &str, path: &std::path::Path, body: Value) -> ApiReply {
        let Some(fields) = body.as_object() else {
            return ApiReply::error(StatusCode::BAD_REQUEST, "收藏参数无效");
        };
        let Some(favorite) = fields.get("favorite").and_then(Value::as_bool) else {
            return ApiReply::error(StatusCode::BAD_REQUEST, "需要 favorite 布尔值");
        };
        let question = fields.contains_key("node_id");
        if fields.len() != if question { 3 } else { 1 }
            || fields
                .keys()
                .any(|key| !matches!(key.as_str(), "favorite" | "node_id" | "generation"))
        {
            return ApiReply::error(
                StatusCode::BAD_REQUEST,
                "收藏参数无效，问题收藏需要 generation",
            );
        }
        let target = if question {
            let Some(node_id) = fields.get("node_id").and_then(Value::as_str) else {
                return ApiReply::error(StatusCode::BAD_REQUEST, "问题标识无效");
            };
            let Some(index) = node_id
                .strip_prefix('q')
                .filter(|n| {
                    !n.starts_with('0') && n.len() <= 6 && n.bytes().all(|c| c.is_ascii_digit())
                })
                .and_then(|n| n.parse::<usize>().ok())
                .and_then(|n| n.checked_sub(1))
            else {
                return ApiReply::error(StatusCode::BAD_REQUEST, "问题标识无效");
            };
            let Some(generation) = fields.get("generation").and_then(Value::as_u64) else {
                return ApiReply::error(StatusCode::BAD_REQUEST, "问题收藏需要有效 generation");
            };
            let Ok(open) = self.get_session(key, false) else {
                return ApiReply::error(StatusCode::NOT_FOUND, "会话不存在");
            };
            if open.generation != generation
                || !open.initialized
                || open.offset < open.total
                || open.error.is_some()
            {
                return ApiReply::error(
                    StatusCode::CONFLICT,
                    "会话已变化或尚未加载，请刷新后再收藏",
                );
            }
            let Some(turn) = open.session.turns.get(index) else {
                return ApiReply::error(StatusCode::NOT_FOUND, "问题不存在");
            };
            favorites::session_key(path, &open.session.meta.id)
                .and_then(|session| favorites::question_key(&session, turn))
        } else {
            discovery::read_summary(path, &self.config)
                .and_then(|summary| favorites::session_key(path, &summary.id))
        };
        let saved = target.and_then(|target| {
            favorites::path().and_then(|path| favorites::set(&path, &target, favorite))
        });
        match saved {
            Ok(()) => ApiReply::json(if question {
                json!({"favorite":favorite,"node_id":fields["node_id"],"generation":fields["generation"]})
            } else {
                json!({"favorite":favorite})
            }),
            Err(error) => ApiReply::error(StatusCode::CONFLICT, &error.to_string()),
        }
    }

    fn mutate(&mut self, route: &str, body: Value) -> ApiReply {
        self.tick();
        let Some((key, action)) = management_target(route) else {
            return ApiReply::error(StatusCode::NOT_FOUND, "管理接口不存在");
        };
        if !self.safe_target(key) {
            return ApiReply::error(StatusCode::NOT_FOUND, "会话不存在或路径已改变，请刷新列表");
        }
        let path = self.registered[key].path.clone();
        if action == "favorite" {
            return self.favorite(key, &path, body);
        }
        let Some(fields) = body.as_object().filter(|v| v.len() == 1) else {
            return ApiReply::error(StatusCode::BAD_REQUEST, "管理参数无效");
        };
        let response = if action == "rename" {
            let Some(name) = fields.get("name").and_then(Value::as_str) else {
                return ApiReply::error(StatusCode::BAD_REQUEST, "需要有效的会话名称");
            };
            if name.trim().is_empty()
                || name.trim().chars().count() > 100
                || name.chars().any(char::is_control)
            {
                return ApiReply::error(
                    StatusCode::BAD_REQUEST,
                    "名称须为 1–100 个字符，不能包含控制字符",
                );
            }
            let Ok(summary) = discovery::read_summary(&path, &self.config) else {
                return ApiReply::error(StatusCode::CONFLICT, "无法核对会话身份，请刷新列表");
            };
            match management::rename(&self.home, &path, &summary.id, name) {
                Ok(name) => {
                    self.names.insert(summary.id, name.clone());
                    for listing in self.listings.iter_mut().flatten() {
                        for session in listing.iter_mut().filter(|s| s["key"] == key) {
                            session["title"] = json!(name);
                        }
                    }
                    ApiReply::json(json!({"name":name}))
                }
                Err(error) => return ApiReply::error(StatusCode::CONFLICT, &error.to_string()),
            }
        } else {
            if fields.get("confirm") != Some(&Value::Bool(true)) {
                return ApiReply::error(StatusCode::BAD_REQUEST, "移到回收站前必须确认");
            }
            if let Err(error) = management::trash(&self.home, &path) {
                return ApiReply::error(StatusCode::CONFLICT, &error.to_string());
            }
            self.trashed.insert(key.to_owned());
            self.open.remove(key);
            self.lru.retain(|v| v != key);
            for listing in self.listings.iter_mut().flatten() {
                listing.retain(|s| s["key"] != key);
            }
            self.search.rows.retain(|row| row.key != key);
            self.search.counts.remove(key);
            self.search.event_summaries.remove(key);
            self.restart_search_index();
            if self.initial_session.as_deref() == Some(key) {
                self.initial_session = None;
            }
            ApiReply::json(json!({"trashed":true}))
        };
        // Discard scans captured before the mutation; their worker exits when the receiver drops.
        self.scan = None;
        self.pending = [false; 2];
        self.start_scan(true);
        response
    }

    fn route(&mut self, path: &str, query: &HashMap<String, String>) -> ApiReply {
        self.tick();
        if path == "/api/health" {
            return ApiReply::json(json!({"ok":true,"readOnly":false,"localOnly":true}));
        }
        if path == "/api/trail/search" {
            self.ensure_search();
            self.search
                .rows
                .retain(|row| !self.trashed.contains(&row.key));
            let mut results =
                trail::search(&self.search, query.get("q").map_or("", String::as_str));
            for result in &mut results {
                if let Some(summary) = self
                    .listings
                    .iter()
                    .flatten()
                    .flatten()
                    .find(|s| s["key"] == result["sessionKey"])
                {
                    if summary["title"]
                        .as_str()
                        .is_some_and(|v| !v.trim().is_empty())
                    {
                        result["sessionTitle"] = summary["title"].clone();
                    }
                }
            }
            return ApiReply::json(
                json!({"loading":self.search_worker.is_some() || self.search_deferred || self.scan.is_some(),"results":results,"truncated":self.search.truncated || results.len() >= 100}),
            );
        }
        if let Some(key) = path
            .strip_prefix("/api/trail/session/")
            .filter(|key| !key.contains('/'))
        {
            let watch = self.config.watch;
            let favorite_store = favorites::path().and_then(|path| favorites::load(&path));
            let registered_path = self.registered.get(key).map(|entry| entry.path.clone());
            let Ok(open) = self.get_session(key, query.get("refresh").is_some_and(|s| s == "1"))
            else {
                return ApiReply::error(
                    StatusCode::NOT_FOUND,
                    "会话不存在或文件不可读取，请刷新会话列表",
                );
            };
            let (mut nodes, edges, notices) = trail::graph(&open.session);
            let identity = registered_path
                .as_ref()
                .and_then(|path| favorites::session_key(path, &open.session.meta.id).ok());
            let mut favorite_error = favorite_store.as_ref().err().map(ToString::to_string);
            if identity.is_none() {
                favorite_error = Some("无法建立稳定的收藏身份".into());
            }
            let favorite = identity
                .as_ref()
                .zip(favorite_store.as_ref().ok())
                .is_some_and(|(key, data)| data.contains(key));
            for (node, turn) in nodes.iter_mut().zip(&open.session.turns) {
                node["favorite"] = json!(identity
                    .as_ref()
                    .zip(favorite_store.as_ref().ok())
                    .is_some_and(|(session, data)| {
                        favorites::question_key(session, turn).is_ok_and(|key| data.contains(&key))
                    }));
            }
            let stats = &open.session.parse_stats;
            let id = open.session.meta.id.clone();
            let mut response = json!({"key":key,"meta":{"id":id,"cwd":open.session.meta.cwd.as_ref().map(|p|p.to_string_lossy())},"generation":open.generation,"revision":open.session.revision,"loading":(!open.initialized||open.offset<open.total)&&open.error.is_none(),"error":open.error,"watch":watch,"stats":{"records":stats.records,"malformed_records":stats.malformed_records,"unknown_records":stats.unknown_records,"skipped_oversize_records":stats.skipped_oversize_records,"omitted_text_bytes":stats.omitted_text_bytes},"nodes":nodes,"edges":edges,"notices":notices});
            response["events"] = json!(open.session.visible_events().collect::<Vec<_>>());
            response["favorite"] = json!(favorite);
            response["favorites_error"] = json!(favorite_error);
            response["meta"]["title"] = json!(self.names.get(&id));
            return ApiReply::json(response);
        }
        if path == "/api/info" {
            return ApiReply::json(
                json!({"version":env!("CARGO_PKG_VERSION"),"watch":self.config.watch,"default_all":self.default_all,"initial_session":self.initial_session,"refresh_ms":750}),
            );
        }
        if path == "/api/sessions" {
            self.ensure_search();
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
            let mut sessions = self.listings[i].clone().unwrap_or_default();
            let favorite_store = favorites::path().and_then(|path| favorites::load(&path));
            for summary in &mut sessions {
                summary["favorite"] = json!(summary["key"]
                    .as_str()
                    .and_then(|key| self.registered.get(key))
                    .zip(favorite_store.as_ref().ok())
                    .is_some_and(|(entry, data)| {
                        favorites::session_key(&entry.path, summary["id"].as_str().unwrap_or(""))
                            .is_ok_and(|key| data.contains(&key))
                    }));
                let event_summary = summary["key"]
                    .as_str()
                    .and_then(|key| self.search.event_summaries.get(key));
                summary["events_loading"] = json!(event_summary.is_none());
                summary["commit_count"] = json!(event_summary.map(|events| events.commit_count));
                summary["compaction_count"] =
                    json!(event_summary.map(|events| events.compaction_count));
                summary["last_commit"] =
                    json!(event_summary.and_then(|events| events.last_commit.as_ref()));
                if let Some(count) = summary["key"]
                    .as_str()
                    .and_then(|key| self.search.counts.get(key))
                {
                    summary["turn_count"] = json!(count);
                }
            }
            return ApiReply::json(
                json!({"loading":self.scan.as_ref().is_some_and(|(active,_)| *active == all)||self.pending[i],"error":self.scan_error[i],"favorites_error":favorite_store.as_ref().err().map(ToString::to_string),"sessions":sessions}),
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
            let mut indices = open.index.search(query.get("q").map_or("", String::as_str));
            match query.get("order").map(String::as_str) {
                Some("chronological") => indices.sort_unstable(),
                None | Some("relevance") => {}
                _ => {
                    return ApiReply::error(
                        StatusCode::BAD_REQUEST,
                        "order 必须是 chronological 或 relevance",
                    )
                }
            }
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
            events_capacity: Arc::new(Semaphore::new(1)),
        }
    }

    #[test]
    fn admitted_api_requests_have_queue_space_during_a_full_sse_burst() {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let (sender, receiver) = request_channel();
                let mut state = http_state();
                state.sender = sender.clone();
                state.capacity = Arc::new(Semaphore::new(MAX_API_REQUESTS));
                // Freeze the consumer until all eight SSE polls and four API
                // handlers have enqueued. Scheduling speed cannot hide overflow.
                let mut event_replies = Vec::new();
                for _ in 0..MAX_EVENT_CONNECTIONS {
                    let (reply, pending) = oneshot::channel();
                    event_replies.push(pending);
                    assert!(sender.try_send(ApiRequest {
                        path: "/api/session/s1".into(),
                        query: HashMap::new(),
                        mutation: None,
                        reply,
                    }).is_ok());
                }
                let mut handlers = Vec::new();
                for _ in 0..MAX_API_REQUESTS {
                    let request = Request::builder()
                        .uri("/api/health")
                        .header("host", &state.authority)
                        .header("x-codex-nav-token", &state.token)
                        .body(Body::empty()).unwrap();
                    handlers.push(Box::pin(handle(State(state.clone()), request)));
                }
                std::future::poll_fn(|cx| {
                    for handler in &mut handlers {
                        assert!(handler.as_mut().poll(cx).is_pending(), "admitted API request must wait for the consumer, not fail from SSE queue saturation");
                    }
                    Poll::Ready(())
                }).await;
                let pending: Vec<_> = receiver.try_iter().collect();
                assert_eq!(pending.len(), MAX_EVENT_CONNECTIONS + MAX_API_REQUESTS);
                for request in pending {
                    assert!(request.reply.send(ApiReply::json(json!({"ok":true}))).is_ok());
                }
                for handler in handlers {
                    assert_eq!(handler.await.status(), StatusCode::OK);
                }
            });
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
            events: Vec::new(),
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
    fn cancelled_index_restarts_unchanged_surviving_sessions() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("sessions");
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("rollout-survivor.jsonl");
        std::fs::write(&path, "{\"type\":\"event_msg\",\"payload\":{\"type\":\"user_message\",\"message\":\"surviving question\"}}\n").unwrap();
        let mut backend = Backend::new(
            root.path().to_owned(),
            root.path().to_owned(),
            Config::default(),
            true,
        );
        let key = backend.register(path, false).unwrap();
        backend.listings[1] = Some(vec![json!({"key":key})]);
        backend.ensure_search();
        assert!(backend.search_worker.is_some());
        assert!(!backend.search_signature.is_empty());
        assert!(backend.search.rows.is_empty());
        backend.restart_search_index();
        backend.ensure_search();
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while backend.search_worker.is_some() && std::time::Instant::now() < deadline {
            backend.tick();
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(trail::search(&backend.search, "surviving").len(), 1);
    }

    #[test]
    fn trail_index_replaces_only_changed_file_and_reuses_loaded_append() {
        use std::io::Write;
        let temp = tempfile::tempdir().unwrap();
        let mut backend = Backend::new(
            temp.path().into(),
            temp.path().into(),
            Config::default(),
            true,
        );
        let mut keys = Vec::new();
        let mut paths = Vec::new();
        for text in ["unchanged marker", "changing marker"] {
            let path = temp.path().join(format!("{}.jsonl", paths.len()));
            std::fs::write(
                &path,
                format!(
                    "{}\n",
                    json!({"type":"event_msg","payload":{"type":"user_message","message":text}})
                ),
            )
            .unwrap();
            keys.push(backend.register(path.clone(), true).unwrap());
            paths.push(path);
        }
        backend.listings[1] = Some(keys.iter().map(|key| json!({"key":key})).collect());
        fn finish(backend: &mut Backend) {
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            while backend.search_worker.is_some() {
                backend.tick();
                assert!(std::time::Instant::now() < deadline);
                thread::sleep(Duration::from_millis(5));
            }
        }
        backend.ensure_search();
        finish(&mut backend);
        let original = backend
            .search
            .rows
            .iter()
            .find(|row| row.key == keys[0])
            .unwrap()
            .searchable
            .as_ptr();
        let append = |text: &str| {
            writeln!(
                std::fs::OpenOptions::new()
                    .append(true)
                    .open(&paths[1])
                    .unwrap(),
                "{}",
                json!({"type":"event_msg","payload":{"type":"user_message","message":text}})
            )
            .unwrap();
        };
        append("second change");
        backend.search_checked -= Duration::from_secs(4);
        backend.ensure_search();
        assert_eq!(
            backend.search.rows.len(),
            2,
            "old rows stay available during background replacement"
        );
        finish(&mut backend);
        assert_eq!(backend.search.rows.len(), 3);
        assert_eq!(
            backend
                .search
                .rows
                .iter()
                .find(|row| row.key == keys[0])
                .unwrap()
                .searchable
                .as_ptr(),
            original,
            "unchanged prompt allocation must be retained, not rebuilt"
        );
        assert_eq!(backend.search.counts[&keys[1]], 2);
        append("third change from live model");
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            let open = backend.get_session(&keys[1], false).unwrap();
            if open.initialized && open.session.turns.len() == 3 && open.offset == open.total {
                break;
            }
            assert!(std::time::Instant::now() < deadline);
            thread::sleep(Duration::from_millis(5));
        }
        backend.search_checked -= Duration::from_secs(4);
        backend.ensure_search();
        assert!(
            backend.search_worker.is_none(),
            "fully loaded changed file must not start another disk reader"
        );
        assert_eq!(backend.search.counts[&keys[1]], 3);
        assert_eq!(backend.search.rows.len(), 4);
        assert_eq!(
            backend
                .search
                .rows
                .iter()
                .find(|row| row.key == keys[0])
                .unwrap()
                .searchable
                .as_ptr(),
            original
        );
    }

    #[test]
    fn trail_index_defers_to_an_active_incremental_reader_instead_of_rescanning() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("active.jsonl");
        std::fs::write(
            &path,
            format!(
                "{}\n",
                json!({"type":"event_msg","payload":{"type":"user_message","message":"loading"}})
            ),
        )
        .unwrap();
        let mut backend = Backend::new(
            temp.path().into(),
            temp.path().into(),
            Config::default(),
            true,
        );
        let key = backend.register(path, true).unwrap();
        backend.listings[1] = Some(vec![json!({"key":key})]);
        backend.get_session(&key, false).unwrap().initialized = false;
        backend.ensure_search();
        assert!(backend.search_deferred);
        assert!(backend.search_worker.is_none());
        assert!(
            !backend.search_signature.contains_key(&key),
            "the signature must remain pending until the active model catches up"
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
