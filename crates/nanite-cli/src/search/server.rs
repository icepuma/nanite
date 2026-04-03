use crate::context::ContextState;
use crate::search::assets;
use crate::search::service::{
    IndexBuildReport, IndexPhase, IndexProgress, RefreshMode, RepoSummary, SearchEngine,
    SearchResponse, parse_search_request, refresh_index,
};
use crate::util::command_available;
use anyhow::{Result, anyhow};
use serde::Serialize;
use std::io::Cursor;
use std::net::{IpAddr, SocketAddr};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};
use url::form_urlencoded;

pub fn serve(context: &ContextState, host: &str, port: u16) -> Result<()> {
    let address = format!("{host}:{port}");
    let server =
        Server::http(&address).map_err(|error| anyhow!("failed to bind {address}: {error}"))?;
    let bound = server
        .server_addr()
        .to_ip()
        .ok_or_else(|| anyhow!("failed to determine server address"))?;
    let mut runtime = SearchRuntime::new(context)?;
    runtime.start_refresh(context, RefreshMode::RefreshIfNeeded)?;

    println!("serving search UI at {}", display_url(bound));

    for request in server.incoming_requests() {
        if let Err(error) = handle_request(context, &mut runtime, request) {
            eprintln!("{error:#}");
        }
    }

    Ok(())
}

fn handle_request(
    context: &ContextState,
    runtime: &mut SearchRuntime,
    request: Request,
) -> Result<()> {
    runtime.sync(context)?;
    let response = route_request(context, runtime, request.method(), request.url())?;
    request.respond(response)?;
    Ok(())
}

fn route_request(
    context: &ContextState,
    runtime: &mut SearchRuntime,
    method: &Method,
    url: &str,
) -> Result<Response<Cursor<Vec<u8>>>> {
    let (path, raw_query) = split_url(url);

    match (method, path) {
        (&Method::Get, "/") => Ok(html_response(StatusCode(200), assets::html())),
        (&Method::Get, "/api/status") => json_response(StatusCode(200), &runtime.status()),
        (&Method::Get, "/api/repos") => json_response(StatusCode(200), &runtime.repo_summaries()),
        (&Method::Get, "/api/search") => {
            let params = QueryParams::parse(raw_query);
            let response = if params.is_empty() {
                SearchResponse {
                    query: parse_search_request("", None, None, None, None, 50),
                    total_hits: 0,
                    truncated: false,
                    hits: Vec::new(),
                }
            } else {
                let limit = params
                    .limit
                    .as_deref()
                    .and_then(|value| value.parse::<usize>().ok())
                    .unwrap_or(50);
                let query = parse_search_request(
                    params.query.as_deref().unwrap_or_default(),
                    params.repo,
                    params.path,
                    params.file,
                    params.lang,
                    limit,
                );
                if let Some(response) = runtime.search(&query)? {
                    response
                } else {
                    return Ok(text_response(
                        StatusCode(409),
                        "text/plain; charset=utf-8",
                        "search index is still building",
                    ));
                }
            };
            json_response(StatusCode(200), &response)
        }
        (&Method::Get, "/api/file") => {
            let params = QueryParams::parse(raw_query);
            let repo = params
                .repo
                .ok_or_else(|| anyhow!("missing `repo` query parameter"))?;
            let path = params
                .path
                .ok_or_else(|| anyhow!("missing `path` query parameter"))?;
            runtime.file_view(&repo, &path)?.map_or_else(
                || {
                    Ok(text_response(
                        StatusCode(409),
                        "text/plain; charset=utf-8",
                        "search index is still building",
                    ))
                },
                |view| json_response(StatusCode(200), &view),
            )
        }
        (&Method::Post, "/api/reindex") => {
            runtime.start_refresh(context, RefreshMode::ForceRebuild)?;
            json_response(StatusCode(202), &runtime.status())
        }
        (&Method::Post, "/api/open") => route_open_request(context, runtime, raw_query),
        _ => Ok(text_response(
            StatusCode(404),
            "text/plain; charset=utf-8",
            "not found",
        )),
    }
}

fn split_url(url: &str) -> (&str, &str) {
    url.split_once('?')
        .map_or((url, ""), |(path, query)| (path, query))
}

fn display_url(bound: SocketAddr) -> String {
    match bound.ip() {
        IpAddr::V4(ip) if ip.is_unspecified() => format!("http://127.0.0.1:{}/", bound.port()),
        IpAddr::V6(ip) if ip.is_unspecified() => format!("http://[::1]:{}/", bound.port()),
        IpAddr::V4(ip) => format!("http://{ip}:{}/", bound.port()),
        IpAddr::V6(ip) => format!("http://[{ip}]:{}/", bound.port()),
    }
}

fn zed_help_text(zed_binary: &str) -> String {
    if command_available(zed_binary) {
        "Opens the viewed file in Zed, rooted at its repository, at line 1 without forcing a new workspace."
            .to_owned()
    } else {
        format!(
            "Disabled because `{zed_binary}` was not found when the server started. Install the Zed CLI or set NANITE_ZED, then restart `nanite search --web`."
        )
    }
}

fn route_open_request(
    context: &ContextState,
    runtime: &SearchRuntime,
    raw_query: &str,
) -> Result<Response<Cursor<Vec<u8>>>> {
    if !runtime.zed_available() {
        return Ok(text_response(
            StatusCode(503),
            "text/plain; charset=utf-8",
            "Zed CLI is not available for this server process.",
        ));
    }

    let params = QueryParams::parse(raw_query);
    let repo = params
        .repo
        .ok_or_else(|| anyhow!("missing `repo` query parameter"))?;
    let path = params
        .path
        .ok_or_else(|| anyhow!("missing `path` query parameter"))?;
    let line = params
        .line
        .as_deref()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(1);
    let column = params
        .column
        .as_deref()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(1);

    runtime
        .open_in_zed(context, &repo, &path, line, column)?
        .map_or_else(
            || {
                Ok(text_response(
                    StatusCode(409),
                    "text/plain; charset=utf-8",
                    "search index is still building",
                ))
            },
            |response| json_response(StatusCode(202), &response),
        )
}

fn html_response(status: StatusCode, body: &str) -> Response<Cursor<Vec<u8>>> {
    Response::from_string(body.to_owned())
        .with_status_code(status)
        .with_header(header("Content-Type", "text/html; charset=utf-8"))
}

fn text_response(status: StatusCode, content_type: &str, body: &str) -> Response<Cursor<Vec<u8>>> {
    Response::from_string(body.to_owned())
        .with_status_code(status)
        .with_header(header("Content-Type", content_type))
}

fn json_response<T: Serialize>(status: StatusCode, body: &T) -> Result<Response<Cursor<Vec<u8>>>> {
    Ok(Response::from_string(serde_json::to_string(body)?)
        .with_status_code(status)
        .with_header(header("Content-Type", "application/json; charset=utf-8")))
}

fn header(name: &str, value: &str) -> Header {
    Header::from_bytes(name.as_bytes(), value.as_bytes())
        .unwrap_or_else(|()| unreachable!("header literals are valid ASCII"))
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
struct IndexStatus {
    phase: ServePhase,
    ready: bool,
    indexing: bool,
    message: String,
    zed_available: bool,
    zed_help: String,
    files_scanned: usize,
    files_total: Option<usize>,
    files_indexed: usize,
    lines_indexed: usize,
    skipped_binary: usize,
    skipped_large: usize,
    last_report: Option<IndexBuildReport>,
    error: Option<String>,
}

impl IndexStatus {
    fn initial(ready: bool, zed_available: bool, zed_help: String) -> Self {
        Self {
            phase: if ready {
                ServePhase::Ready
            } else {
                ServePhase::Starting
            },
            ready,
            indexing: false,
            message: if ready {
                "Loaded the current workspace index.".to_owned()
            } else {
                "Preparing the first workspace index…".to_owned()
            },
            zed_available,
            zed_help,
            files_scanned: 0,
            files_total: None,
            files_indexed: 0,
            lines_indexed: 0,
            skipped_binary: 0,
            skipped_large: 0,
            last_report: None,
            error: None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum ServePhase {
    Starting,
    Scanning,
    Building,
    Ready,
    Failed,
}

struct SearchRuntime {
    engine: Option<SearchEngine>,
    repos: Vec<RepoSummary>,
    status: IndexStatus,
    worker: Option<Receiver<WorkerEvent>>,
}

#[derive(Debug, Clone, Serialize)]
struct OpenFileResponse {
    repo: String,
    path: String,
    line: u64,
    column: u64,
}

impl SearchRuntime {
    fn new(context: &ContextState) -> Result<Self> {
        let engine = SearchEngine::open_existing(context)?;
        let repos = engine
            .as_ref()
            .map_or_else(Vec::new, SearchEngine::repo_summaries);
        let status = IndexStatus::initial(
            engine.is_some(),
            command_available(&context.zed_binary),
            zed_help_text(&context.zed_binary),
        );
        Ok(Self {
            engine,
            repos,
            status,
            worker: None,
        })
    }

    fn start_refresh(&mut self, context: &ContextState, mode: RefreshMode) -> Result<()> {
        self.sync(context)?;
        if self.worker.is_some() {
            return Ok(());
        }

        self.status.indexing = true;
        self.status.error = None;
        if self.status.ready {
            "Refreshing the workspace index in the background…"
                .clone_into(&mut self.status.message);
        } else {
            self.status.phase = ServePhase::Starting;
            "Building the first workspace index…".clone_into(&mut self.status.message);
        }

        let worker_context = context.clone();
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let progress_sender = sender.clone();
            let result = refresh_index(&worker_context, mode, move |progress| {
                let _ = progress_sender.send(WorkerEvent::Progress(progress));
            });

            match result {
                Ok(report) => {
                    let _ = sender.send(WorkerEvent::Finished(report));
                }
                Err(error) => {
                    let _ = sender.send(WorkerEvent::Failed(format!("{error:#}")));
                }
            }
        });
        self.worker = Some(receiver);
        Ok(())
    }

    fn sync(&mut self, context: &ContextState) -> Result<()> {
        let Some(receiver) = self.worker.take() else {
            return Ok(());
        };

        let mut finished = false;
        loop {
            match receiver.try_recv() {
                Ok(WorkerEvent::Progress(progress)) => self.apply_progress(progress),
                Ok(WorkerEvent::Finished(report)) => {
                    self.apply_finish(context, report)?;
                    finished = true;
                }
                Ok(WorkerEvent::Failed(message)) => {
                    self.apply_failure(message);
                    finished = true;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    finished = true;
                    break;
                }
            }
        }

        if !finished {
            self.worker = Some(receiver);
        }

        Ok(())
    }

    fn status(&self) -> IndexStatus {
        self.status.clone()
    }

    fn repo_summaries(&self) -> Vec<RepoSummary> {
        self.engine
            .as_ref()
            .map_or_else(|| self.repos.clone(), SearchEngine::repo_summaries)
    }

    const fn zed_available(&self) -> bool {
        self.status.zed_available
    }

    fn search(
        &self,
        request: &crate::search::service::SearchRequest,
    ) -> Result<Option<SearchResponse>> {
        self.engine
            .as_ref()
            .map(|engine| engine.search(request))
            .transpose()
    }

    fn file_view(
        &mut self,
        repo: &str,
        path: &str,
    ) -> Result<Option<crate::search::service::FileView>> {
        self.engine
            .as_mut()
            .map(|engine| engine.file_view(repo, path))
            .transpose()
    }

    fn open_in_zed(
        &self,
        context: &ContextState,
        repo: &str,
        path: &str,
        line: u64,
        column: u64,
    ) -> Result<Option<OpenFileResponse>> {
        self.engine.as_ref().map_or_else(
            || Ok(None),
            |engine| {
                engine.open_in_zed(&context.zed_binary, repo, path, line, column)?;
                Ok(Some(OpenFileResponse {
                    repo: repo.to_owned(),
                    path: path.to_owned(),
                    line: line.max(1),
                    column: column.max(1),
                }))
            },
        )
    }

    fn apply_progress(&mut self, progress: IndexProgress) {
        self.status.phase = match progress.phase {
            IndexPhase::Scanning => ServePhase::Scanning,
            IndexPhase::Building => ServePhase::Building,
        };
        self.status.indexing = true;
        self.status.message = progress.message;
        self.status.files_scanned = progress.files_scanned;
        self.status.files_total = progress.files_total;
        self.status.files_indexed = progress.files_indexed;
        self.status.lines_indexed = progress.lines_indexed;
        self.status.skipped_binary = progress.skipped_binary;
        self.status.skipped_large = progress.skipped_large;
    }

    fn apply_finish(&mut self, context: &ContextState, report: IndexBuildReport) -> Result<()> {
        self.engine = SearchEngine::open_existing(context)?;
        self.repos = self
            .engine
            .as_ref()
            .map_or_else(Vec::new, SearchEngine::repo_summaries);
        self.status.phase = ServePhase::Ready;
        self.status.ready = self.engine.is_some();
        self.status.indexing = false;
        self.status.message = if report.rebuilt {
            if report.partial {
                format!(
                    "Search index ready after refreshing {} repos: {} files across {} lines.",
                    report.repos_refreshed, report.files_indexed, report.lines_indexed
                )
            } else {
                format!(
                    "Search index ready after rebuilding {} repos: {} files across {} lines.",
                    report.repos_refreshed, report.files_indexed, report.lines_indexed
                )
            }
        } else {
            format!(
                "Search index is current across {} files.",
                report.files_indexed
            )
        };
        self.status.files_scanned = report.files_scanned;
        self.status.files_total = Some(report.files_indexed);
        self.status.files_indexed = report.files_indexed;
        self.status.lines_indexed = report.lines_indexed;
        self.status.skipped_binary = report.skipped_binary;
        self.status.skipped_large = report.skipped_large;
        self.status.last_report = Some(report);
        self.status.error = None;
        Ok(())
    }

    fn apply_failure(&mut self, message: String) {
        self.status.ready = self.engine.is_some();
        self.status.indexing = false;
        self.status.phase = if self.status.ready {
            ServePhase::Ready
        } else {
            ServePhase::Failed
        };
        self.status.message = if self.status.ready {
            "Index refresh failed. Showing the last loaded index.".to_owned()
        } else {
            "Search index build failed.".to_owned()
        };
        self.status.error = Some(message);
    }
}

enum WorkerEvent {
    Progress(IndexProgress),
    Finished(IndexBuildReport),
    Failed(String),
}

#[derive(Default)]
struct QueryParams {
    column: Option<String>,
    file: Option<String>,
    lang: Option<String>,
    line: Option<String>,
    limit: Option<String>,
    path: Option<String>,
    query: Option<String>,
    repo: Option<String>,
}

impl QueryParams {
    fn parse(raw: &str) -> Self {
        let mut params = Self::default();
        for (key, value) in form_urlencoded::parse(raw.as_bytes()) {
            match key.as_ref() {
                "column" => params.column = Some(value.into_owned()),
                "file" => params.file = Some(value.into_owned()),
                "lang" => params.lang = Some(value.into_owned()),
                "line" => params.line = Some(value.into_owned()),
                "limit" => params.limit = Some(value.into_owned()),
                "path" => params.path = Some(value.into_owned()),
                "query" => params.query = Some(value.into_owned()),
                "repo" => params.repo = Some(value.into_owned()),
                _ => {}
            }
        }
        params
    }

    const fn is_empty(&self) -> bool {
        self.file.is_none()
            && self.lang.is_none()
            && self.path.is_none()
            && self.query.is_none()
            && self.repo.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::{QueryParams, SearchRuntime, display_url, route_request, split_url};
    use crate::context::ContextState;
    use crate::search::service::{RefreshMode, SearchEngine};
    use camino::Utf8PathBuf;
    use nanite_core::{AgentKind, AppPaths, Config};
    use std::collections::HashMap;
    use std::ffi::OsString;
    use std::fs;
    use std::io::Read;
    use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6};
    use std::os::unix::fs::PermissionsExt;
    use tempfile::TempDir;
    use tiny_http::{Method, StatusCode};

    struct Harness {
        _tempdir: TempDir,
        context: ContextState,
    }

    impl Harness {
        fn new() -> Self {
            let tempdir = tempfile::tempdir().unwrap();
            let root = Utf8PathBuf::from_path_buf(tempdir.path().to_path_buf()).unwrap();
            let home_dir = root.join("home");
            let config_dir = root.join("config");
            let data_dir = root.join("data");
            let state_dir = root.join("state");
            let workspace_root = root.join("workspace");
            fs::create_dir_all(&home_dir).unwrap();
            fs::create_dir_all(&config_dir).unwrap();
            fs::create_dir_all(&data_dir).unwrap();
            fs::create_dir_all(&state_dir).unwrap();
            fs::create_dir_all(workspace_root.join("repos")).unwrap();

            let env = HashMap::from([
                ("HOME".to_owned(), home_dir.to_string()),
                ("NANITE_CONFIG_DIR".to_owned(), config_dir.to_string()),
                ("NANITE_DATA_DIR".to_owned(), data_dir.to_string()),
                ("NANITE_STATE_DIR".to_owned(), state_dir.to_string()),
            ]);
            let app_paths = AppPaths::from_env(|key| env.get(key).map(OsString::from)).unwrap();
            let config = Config {
                workspace_root,
                agent: AgentKind::Codex,
            };
            let workspace_paths = config.workspace_paths();

            Self {
                _tempdir: tempdir,
                context: ContextState {
                    app_paths,
                    config,
                    workspace_paths,
                    git_binary: "git".to_owned(),
                    fzf_binary: "fzf".to_owned(),
                    zed_binary: "__nanite_test_missing_zed__".to_owned(),
                },
            }
        }

        fn create_repo(&self, repo_id: &str) -> Utf8PathBuf {
            let path = self.context.workspace_paths.repos_root().join(repo_id);
            fs::create_dir_all(path.join(".git")).unwrap();
            path
        }

        fn install_fake_zed(&mut self) -> Utf8PathBuf {
            let script_path = self.context.workspace_paths.root().join("fake-zed.sh");
            let output_path = self.context.workspace_paths.root().join("zed-args.txt");
            fs::write(
                script_path.as_std_path(),
                "#!/bin/sh\nscript_dir=$(dirname \"$0\")\nprintf '%s\\n' \"$@\" > \"$script_dir/zed-args.txt\"\n",
            )
            .unwrap();
            let mut permissions = fs::metadata(script_path.as_std_path())
                .unwrap()
                .permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(script_path.as_std_path(), permissions).unwrap();
            self.context.zed_binary = script_path.to_string();
            output_path
        }
    }

    #[test]
    fn split_url_handles_missing_query() {
        assert_eq!(split_url("/api/search"), ("/api/search", ""));
    }

    #[test]
    fn split_url_separates_query_string() {
        assert_eq!(
            split_url("/api/search?query=test"),
            ("/api/search", "query=test")
        );
    }

    #[test]
    fn display_url_prefers_loopback_for_unspecified_hosts() {
        assert_eq!(
            display_url(SocketAddr::V4(SocketAddrV4::new(
                Ipv4Addr::UNSPECIFIED,
                4312
            ))),
            "http://127.0.0.1:4312/"
        );
        assert_eq!(
            display_url(SocketAddr::V6(SocketAddrV6::new(
                Ipv6Addr::UNSPECIFIED,
                4312,
                0,
                0
            ))),
            "http://[::1]:4312/"
        );
    }

    #[test]
    fn query_params_parse_known_fields() {
        let params = QueryParams::parse("query=hello&repo=github.com%2Ficepuma%2Fnanite&limit=12");

        assert_eq!(params.query.as_deref(), Some("hello"));
        assert_eq!(params.repo.as_deref(), Some("github.com/icepuma/nanite"));
        assert_eq!(params.limit.as_deref(), Some("12"));
    }

    #[test]
    fn route_request_serves_status_without_an_index() {
        let harness = Harness::new();
        let mut runtime = SearchRuntime::new(&harness.context).unwrap();

        let response =
            route_request(&harness.context, &mut runtime, &Method::Get, "/api/status").unwrap();
        assert_eq!(response.status_code(), StatusCode(200));

        let mut body = String::new();
        response.into_reader().read_to_string(&mut body).unwrap();
        assert!(body.contains("\"ready\":false"));
        assert!(body.contains("\"zed_available\":false"));
    }

    #[test]
    fn route_request_serves_search_json() {
        let harness = Harness::new();
        let repo = harness.create_repo("github.com/icepuma/nanite");
        fs::create_dir_all(repo.join("src")).unwrap();
        fs::write(repo.join("src/lib.rs"), "pub fn workspace_root() {}\n").unwrap();

        SearchEngine::open(&harness.context, RefreshMode::ForceRebuild).unwrap();
        let mut runtime = SearchRuntime::new(&harness.context).unwrap();

        let response = route_request(
            &harness.context,
            &mut runtime,
            &Method::Get,
            "/api/search?query=workspace_root",
        )
        .unwrap();
        assert_eq!(response.status_code(), StatusCode(200));

        let mut body = String::new();
        response.into_reader().read_to_string(&mut body).unwrap();
        assert!(body.contains("\"path\":\"src/lib.rs\""));
    }

    #[test]
    fn route_request_starts_async_reindex() {
        let harness = Harness::new();
        let repo = harness.create_repo("github.com/icepuma/nanite");
        fs::create_dir_all(repo.join("src")).unwrap();
        fs::write(repo.join("src/lib.rs"), "pub fn before_token() {}\n").unwrap();

        SearchEngine::open(&harness.context, RefreshMode::ForceRebuild).unwrap();
        let mut runtime = SearchRuntime::new(&harness.context).unwrap();
        fs::write(repo.join("src/lib.rs"), "pub fn after_token() {}\n").unwrap();

        let response = route_request(
            &harness.context,
            &mut runtime,
            &Method::Post,
            "/api/reindex",
        )
        .unwrap();
        assert_eq!(response.status_code(), StatusCode(202));

        for _ in 0..200 {
            runtime.sync(&harness.context).unwrap();
            if !runtime.status().indexing {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        let search = route_request(
            &harness.context,
            &mut runtime,
            &Method::Get,
            "/api/search?query=after_token",
        )
        .unwrap();
        let mut body = String::new();
        search.into_reader().read_to_string(&mut body).unwrap();
        assert!(body.contains("after_token"));
    }

    #[test]
    fn route_request_opens_files_in_zed() {
        let mut harness = Harness::new();
        let output_path = harness.install_fake_zed();
        let repo = harness.create_repo("github.com/icepuma/nanite");
        fs::create_dir_all(repo.join("src")).unwrap();
        fs::write(repo.join("src/lib.rs"), "pub fn workspace_root() {}\n").unwrap();

        SearchEngine::open(&harness.context, RefreshMode::ForceRebuild).unwrap();
        let mut runtime = SearchRuntime::new(&harness.context).unwrap();

        let response = route_request(
            &harness.context,
            &mut runtime,
            &Method::Post,
            "/api/open?repo=github.com%2Ficepuma%2Fnanite&path=src%2Flib.rs&line=7&column=3",
        )
        .unwrap();
        assert_eq!(response.status_code(), StatusCode(202));

        let mut body = String::new();
        response.into_reader().read_to_string(&mut body).unwrap();
        assert!(body.contains("\"line\":7"));
        assert!(body.contains("\"column\":3"));

        for _ in 0..300 {
            if fs::metadata(output_path.as_std_path())
                .map(|metadata| metadata.len() > 0)
                .unwrap_or(false)
            {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        assert!(output_path.exists());
        let args = fs::read_to_string(output_path.as_std_path()).unwrap();
        assert!(!args.contains("--new"));
        assert!(args.contains(repo.as_str()));
        assert!(args.contains("src/lib.rs:7:3"));
    }

    #[test]
    fn route_request_rejects_open_when_zed_is_unavailable() {
        let harness = Harness::new();
        let repo = harness.create_repo("github.com/icepuma/nanite");
        fs::create_dir_all(repo.join("src")).unwrap();
        fs::write(repo.join("src/lib.rs"), "pub fn workspace_root() {}\n").unwrap();

        SearchEngine::open(&harness.context, RefreshMode::ForceRebuild).unwrap();
        let mut runtime = SearchRuntime::new(&harness.context).unwrap();

        let response = route_request(
            &harness.context,
            &mut runtime,
            &Method::Post,
            "/api/open?repo=github.com%2Ficepuma%2Fnanite&path=src%2Flib.rs",
        )
        .unwrap();
        assert_eq!(response.status_code(), StatusCode(503));

        let mut body = String::new();
        response.into_reader().read_to_string(&mut body).unwrap();
        assert!(body.contains("Zed CLI is not available"));
    }
}
