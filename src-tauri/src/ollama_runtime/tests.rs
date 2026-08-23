use super::*;
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex,
    },
    thread::{self, JoinHandle},
};

#[derive(Debug, Clone)]
struct RecordedRequest {
    method: String,
    path: String,
    body: Vec<u8>,
}

struct FakeResponse {
    status: u16,
    reason: &'static str,
    headers: Vec<(&'static str, String)>,
    body: Vec<u8>,
    delay: Duration,
}

impl FakeResponse {
    fn json(payload: Value) -> Self {
        let body = serde_json::to_vec(&payload).expect("fake response JSON should serialize");
        Self {
            status: 200,
            reason: "OK",
            headers: vec![
                ("Content-Type", "application/json".to_string()),
                ("Content-Length", body.len().to_string()),
            ],
            body,
            delay: Duration::ZERO,
        }
    }

    fn error(status: u16, message: &str) -> Self {
        let mut response = Self::json(json!({ "error": message }));
        response.status = status;
        response.reason = "Test Error";
        response
    }

    fn delayed(mut self, delay: Duration) -> Self {
        self.delay = delay;
        self
    }
}

struct FakeServer {
    address: SocketAddr,
    endpoint: String,
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl FakeServer {
    fn start(handler: impl Fn(&RecordedRequest) -> FakeResponse + Send + Sync + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("fake server should bind");
        listener
            .set_nonblocking(true)
            .expect("fake server should be nonblocking");
        let address = listener
            .local_addr()
            .expect("fake server should have an address");
        let endpoint = format!("http://{address}/");
        let requests = Arc::new(Mutex::new(Vec::new()));
        let worker_requests = requests.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = stop.clone();
        let handler = Arc::new(handler);
        let worker = thread::spawn(move || {
            while !worker_stop.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream
                            .set_read_timeout(Some(Duration::from_secs(1)))
                            .expect("fake server should set a read timeout");
                        let Some(request) = read_request(&stream) else {
                            continue;
                        };
                        worker_requests
                            .lock()
                            .expect("fake request log should lock")
                            .push(request.clone());
                        let response = handler(&request);
                        if !response.delay.is_zero() {
                            thread::sleep(response.delay);
                        }
                        let _ = write_response(&mut stream, response);
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2));
                    }
                    Err(error) => panic!("fake server accept failed: {error}"),
                }
            }
        });
        Self {
            address,
            endpoint,
            requests,
            stop,
            worker: Some(worker),
        }
    }

    fn request_paths(&self) -> Vec<String> {
        self.requests
            .lock()
            .expect("fake request log should lock")
            .iter()
            .map(|request| request.path.clone())
            .collect()
    }
}

impl Drop for FakeServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(self.address);
        if let Some(worker) = self.worker.take() {
            worker.join().expect("fake server should stop cleanly");
        }
    }
}

fn read_request(stream: &TcpStream) -> Option<RecordedRequest> {
    let mut reader = BufReader::new(stream.try_clone().ok()?);
    let mut request_line = String::new();
    reader.read_line(&mut request_line).ok()?;
    let mut request_parts = request_line.split_whitespace();
    let method = request_parts.next()?.to_string();
    let path = request_parts.next()?.to_string();
    let mut content_length = 0_usize;
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).ok()?;
        if line == "\r\n" {
            break;
        }
        if line.is_empty() {
            return None;
        }
        if let Some(value) = line
            .split_once(':')
            .filter(|(name, _)| name.eq_ignore_ascii_case("content-length"))
            .map(|(_, value)| value.trim())
        {
            content_length = value.parse().ok()?;
        }
    }
    let mut body = vec![0_u8; content_length];
    reader.read_exact(&mut body).ok()?;
    Some(RecordedRequest { method, path, body })
}

fn write_response(stream: &mut TcpStream, response: FakeResponse) -> std::io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 {} {}\r\nConnection: close\r\n",
        response.status, response.reason
    )?;
    for (name, value) in response.headers {
        write!(stream, "{name}: {value}\r\n")?;
    }
    stream.write_all(b"\r\n")?;
    stream.write_all(&response.body)?;
    stream.flush()
}

fn chunked_body(bytes: &[u8], chunk_size: usize) -> Vec<u8> {
    let mut body = Vec::new();
    for chunk in bytes.chunks(chunk_size) {
        write!(&mut body, "{:x}\r\n", chunk.len()).expect("chunk header should write");
        body.extend_from_slice(chunk);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(b"0\r\nX-Test-Trailer: complete\r\n\r\n");
    body
}

fn uncancelled() -> ProviderCancellation {
    ProviderCancellation::new(Arc::new(AtomicBool::new(false)))
}

#[test]
fn task_0008_discovers_capabilities_and_context_from_show_not_tags() {
    let server = FakeServer::start(|request| match request.path.as_str() {
        "/api/version" => FakeResponse::json(json!({ "version": "0.12.3" })),
        "/api/tags" => FakeResponse::json(json!({
            "models": [
                {
                    "name": "qwen2.5-coder:7b",
                    "details": { "context_length": 1 },
                    "capabilities": []
                },
                { "name": "vision-only:latest" }
            ]
        })),
        "/api/show" => {
            assert_eq!(request.method, "POST");
            let body: Value =
                serde_json::from_slice(&request.body).expect("show body should be JSON");
            assert_eq!(body["verbose"], false);
            match body["model"].as_str() {
                Some("qwen2.5-coder:7b") => FakeResponse::json(json!({
                    "capabilities": ["completion", "tools", "TOOLS"],
                    "model_info": {
                        "general.architecture": "qwen2",
                        "qwen2.context_length": 32768
                    }
                })),
                Some("vision-only:latest") => FakeResponse::json(json!({
                    "capabilities": ["completion", "vision"],
                    "model_info": {
                        "general.architecture": "vision",
                        "vision.context_length": 8192,
                        "vision.audio.context_length": 1024
                    }
                })),
                model => panic!("unexpected show model: {model:?}"),
            }
        }
        path => panic!("unexpected fake Ollama path: {path}"),
    });
    let session = OllamaSession::for_test_endpoint(&server.endpoint)
        .expect("test Ollama session should be created");

    let status = session.inspect_catalog();

    assert!(status.connected);
    assert!(status.catalog_ready);
    assert_eq!(status.version.as_deref(), Some("0.12.3"));
    assert_eq!(status.models.len(), 2);
    assert_eq!(
        status.models[0],
        OllamaModel {
            name: "qwen2.5-coder:7b".to_string(),
            capabilities: vec!["completion".to_string(), "tools".to_string()],
            context_length: Some(32_768),
            availability: ProviderAvailability::Ready,
            message: "Model capabilities and context metadata are ready.".to_string(),
        }
    );
    assert!(!status.models[1].supports_tools());
    assert_eq!(status.models[1].context_length, Some(8_192));
    let paths = server.request_paths();
    assert_eq!(
        paths
            .iter()
            .filter(|path| path.as_str() == "/api/show")
            .count(),
        2
    );
}

#[test]
fn task_0008_catalog_keeps_per_model_show_failures_truthful() {
    let server = FakeServer::start(|request| match request.path.as_str() {
        "/api/version" => FakeResponse::json(json!({ "version": "test" })),
        "/api/tags" => FakeResponse::json(json!({
            "models": [{ "name": "ready" }, { "name": "broken" }]
        })),
        "/api/show" => {
            let body: Value =
                serde_json::from_slice(&request.body).expect("show body should be JSON");
            if body["model"] == "ready" {
                FakeResponse::json(json!({
                    "capabilities": ["tools"],
                    "model_info": {}
                }))
            } else {
                FakeResponse::error(500, "metadata unavailable")
            }
        }
        path => panic!("unexpected fake Ollama path: {path}"),
    });
    let session = OllamaSession::for_test_endpoint(&server.endpoint)
        .expect("test Ollama session should be created");

    let status = session.inspect_catalog();

    assert!(status.catalog_ready);
    assert_eq!(status.models[0].availability, ProviderAvailability::Ready);
    assert_eq!(
        status.models[1].availability,
        ProviderAvailability::Unavailable
    );
    assert!(status.models[1].message.contains("HTTP 500"));
    assert!(status.message.contains("metadata is unavailable for 1"));
}

#[test]
fn task_0008_transport_decodes_chunked_gzip_after_arbitrary_boundaries() {
    const GZIP_JSON: &[u8] = &[
        31, 139, 8, 0, 0, 0, 0, 0, 0, 3, 171, 86, 42, 41, 74, 204, 43, 46, 200, 47, 42, 81, 178,
        82, 74, 206, 207, 45, 40, 74, 45, 46, 78, 77, 81, 170, 5, 0, 8, 180, 63, 183, 26, 0, 0, 0,
    ];
    let server = FakeServer::start(|_| FakeResponse {
        status: 200,
        reason: "OK",
        headers: vec![
            ("Content-Type", "application/json".to_string()),
            ("Content-Encoding", "gzip".to_string()),
            ("Transfer-Encoding", "chunked".to_string()),
        ],
        body: chunked_body(GZIP_JSON, 7),
        delay: Duration::ZERO,
    });
    let session = OllamaSession::for_test_endpoint(&server.endpoint)
        .expect("test Ollama session should be created");

    let response = session
        .runtime
        .block_on(session.client.request_json(
            Method::GET,
            "/api/version",
            None,
            RequestControl::inspection(),
        ))
        .expect("chunked gzip JSON should decode");

    assert_eq!(response, json!({ "transport": "compressed" }));
}

#[test]
fn task_0008_transport_rejects_declared_and_streamed_response_overflow() {
    let declared = FakeServer::start(|_| FakeResponse {
        status: 200,
        reason: "OK",
        headers: vec![
            ("Content-Type", "application/json".to_string()),
            (
                "Content-Length",
                (MAX_OLLAMA_RESPONSE_BYTES + 1).to_string(),
            ),
        ],
        body: Vec::new(),
        delay: Duration::ZERO,
    });
    let session = OllamaSession::for_test_endpoint(&declared.endpoint)
        .expect("test Ollama session should be created");
    let error = session
        .runtime
        .block_on(session.client.request_json(
            Method::GET,
            "/api/version",
            None,
            RequestControl::inspection(),
        ))
        .expect_err("declared overflow should fail");
    assert_eq!(error.kind, OllamaErrorKind::OutputLimit);
    drop(declared);

    let streamed = FakeServer::start(|_| FakeResponse {
        status: 200,
        reason: "OK",
        headers: vec![
            ("Content-Type", "application/json".to_string()),
            ("Transfer-Encoding", "chunked".to_string()),
        ],
        body: chunked_body(&vec![b'x'; MAX_OLLAMA_RESPONSE_BYTES + 1], 16 * 1024),
        delay: Duration::ZERO,
    });
    let session = OllamaSession::for_test_endpoint(&streamed.endpoint)
        .expect("test Ollama session should be created");
    let error = session
        .runtime
        .block_on(session.client.request_json(
            Method::GET,
            "/api/version",
            None,
            RequestControl::inspection(),
        ))
        .expect_err("streamed overflow should fail");
    assert_eq!(error.kind, OllamaErrorKind::OutputLimit);
}

#[test]
fn task_0008_transport_classifies_malformed_error_and_disconnected_cases() {
    let malformed = FakeServer::start(|_| FakeResponse {
        status: 200,
        reason: "OK",
        headers: vec![
            ("Content-Type", "application/json".to_string()),
            ("Content-Length", "8".to_string()),
        ],
        body: b"not-json".to_vec(),
        delay: Duration::ZERO,
    });
    let session = OllamaSession::for_test_endpoint(&malformed.endpoint)
        .expect("test Ollama session should be created");
    let error = session
        .runtime
        .block_on(session.client.request_json(
            Method::GET,
            "/api/version",
            None,
            RequestControl::inspection(),
        ))
        .expect_err("malformed JSON should fail");
    assert_eq!(error.kind, OllamaErrorKind::Protocol);
    drop(malformed);

    let listener = TcpListener::bind("127.0.0.1:0").expect("unused address should bind");
    let endpoint = format!(
        "http://{}/",
        listener
            .local_addr()
            .expect("unused address should be available")
    );
    drop(listener);
    let session =
        OllamaSession::for_test_endpoint(&endpoint).expect("test Ollama session should be created");
    let error = session
        .runtime
        .block_on(session.client.request_json(
            Method::GET,
            "/api/version",
            None,
            RequestControl::inspection(),
        ))
        .expect_err("disconnected endpoint should fail");
    assert_eq!(error.kind, OllamaErrorKind::Unavailable);
}

#[test]
fn task_0008_transport_uses_task_deadline() {
    let server = FakeServer::start(|_| {
        FakeResponse::json(json!({ "version": "too late" })).delayed(Duration::from_millis(250))
    });
    let session = OllamaSession::for_test_endpoint(&server.endpoint)
        .expect("test Ollama session should be created");
    let started = Instant::now();

    let error = session
        .runtime
        .block_on(session.client.request_json(
            Method::GET,
            "/api/version",
            None,
            RequestControl::run(uncancelled(), Instant::now() + Duration::from_millis(60)),
        ))
        .expect_err("slow request should time out");

    assert_eq!(error.kind, OllamaErrorKind::TimedOut);
    assert!(started.elapsed() < Duration::from_millis(200));
}

#[test]
fn task_0008_cancellation_drops_the_active_socket_without_an_orphan_worker() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("cancellation server should bind");
    let address = listener
        .local_addr()
        .expect("cancellation server should have an address");
    let endpoint = format!("http://{address}/");
    let (request_received_sender, request_received_receiver) = mpsc::channel();
    let (socket_closed_sender, socket_closed_receiver) = mpsc::channel();
    let server = thread::spawn(move || {
        let (stream, _) = listener
            .accept()
            .expect("cancellation server should accept");
        stream
            .set_read_timeout(Some(Duration::from_millis(50)))
            .expect("cancellation server should set read timeout");
        let request = read_request(&stream).expect("cancellation request should parse");
        assert_eq!(request.path, "/api/chat");
        request_received_sender
            .send(())
            .expect("request receipt should be reported");
        let mut probe = [0_u8; 1];
        let mut stream = stream;
        let closed = loop {
            match stream.read(&mut probe) {
                Ok(0) => break true,
                Ok(_) => continue,
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                    ) => {}
                Err(_) => break true,
            }
        };
        socket_closed_sender
            .send(closed)
            .expect("socket closure should be reported");
    });
    let session =
        OllamaSession::for_test_endpoint(&endpoint).expect("test Ollama session should be created");
    let flag = Arc::new(AtomicBool::new(false));
    let cancellation = ProviderCancellation::new(flag.clone());
    let setter = thread::spawn(move || {
        request_received_receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("request should reach fake server");
        flag.store(true, Ordering::SeqCst);
    });

    let error = session
        .runtime
        .block_on(session.client.request_json(
            Method::POST,
            "/api/chat",
            Some(json!({ "model": "test" })),
            RequestControl::run(cancellation, Instant::now() + Duration::from_secs(2)),
        ))
        .expect_err("cancelled request should fail");

    assert_eq!(error.kind, OllamaErrorKind::Cancelled);
    assert!(
        socket_closed_receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("server should observe socket state"),
        "dropping the request future must close the active socket"
    );
    setter.join().expect("cancellation setter should finish");
    server.join().expect("cancellation server should finish");
}

#[test]
fn task_0008_endpoint_and_redirect_boundaries_fail_closed() {
    for endpoint in [
        "https://127.0.0.1:11434/",
        "http://localhost:11434/",
        "http://192.0.2.1:11434/",
        "http://user@127.0.0.1:11434/",
    ] {
        assert!(
            OllamaSession::for_test_endpoint(endpoint).is_err(),
            "endpoint should be rejected: {endpoint}"
        );
    }

    let server = FakeServer::start(|_| FakeResponse {
        status: 302,
        reason: "Found",
        headers: vec![
            ("Content-Type", "application/json".to_string()),
            ("Content-Length", "28".to_string()),
            ("Location", "http://127.0.0.1:9/api/version".to_string()),
        ],
        body: br#"{"error":"redirect refused"}"#.to_vec(),
        delay: Duration::ZERO,
    });
    let session = OllamaSession::for_test_endpoint(&server.endpoint)
        .expect("test Ollama session should be created");
    let error = session
        .runtime
        .block_on(session.client.request_json(
            Method::GET,
            "/api/version",
            None,
            RequestControl::inspection(),
        ))
        .expect_err("redirect should not be followed");
    assert_eq!(error.kind, OllamaErrorKind::Protocol);
    assert!(error.message.contains("HTTP 302"));
}
