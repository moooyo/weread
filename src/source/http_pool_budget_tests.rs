//! Real bounded TCP keepalive fixtures for the production transport factory.

use super::*;
use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
    thread::{self, JoinHandle},
    time::Instant,
};

struct ActiveSocket(Arc<AtomicUsize>);
impl Drop for ActiveSocket {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

struct KeepaliveServer {
    address: SocketAddr,
    stopped: Arc<AtomicBool>,
    released: Arc<AtomicBool>,
    accepted: Arc<AtomicUsize>,
    active: Arc<AtomicUsize>,
    worker: Option<JoinHandle<std::result::Result<(), String>>>,
}

impl KeepaliveServer {
    fn new(hold_responses: bool) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let stopped = Arc::new(AtomicBool::new(false));
        let released = Arc::new(AtomicBool::new(!hold_responses));
        let accepted = Arc::new(AtomicUsize::new(0));
        let active = Arc::new(AtomicUsize::new(0));
        let stop = stopped.clone();
        let release = released.clone();
        let count = accepted.clone();
        let live = active.clone();
        let worker = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(8);
            let mut workers = Vec::new();
            let mut failure = None;
            while !stop.load(Ordering::Acquire)
                && Instant::now() < deadline
                && count.load(Ordering::Acquire) < 64
            {
                match listener.accept() {
                    Ok((stream, _)) => {
                        count.fetch_add(1, Ordering::AcqRel);
                        live.fetch_add(1, Ordering::AcqRel);
                        let guard = ActiveSocket(live.clone());
                        let stop = stop.clone();
                        let release = release.clone();
                        workers.push(thread::spawn(move || {
                            let _guard = guard;
                            serve(stream, &stop, &release, deadline)
                        }));
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2));
                    }
                    Err(error) => {
                        failure = Some(error.to_string());
                        break;
                    }
                }
            }
            stop.store(true, Ordering::Release);
            for worker in workers {
                match worker.join() {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => {
                        if failure.is_none() {
                            failure = Some(error);
                        }
                    }
                    Err(_) => {
                        if failure.is_none() {
                            failure = Some("TCP fixture worker panicked".into());
                        }
                    }
                }
            }
            failure.map_or(Ok(()), Err)
        });
        Self {
            address,
            stopped,
            released,
            accepted,
            active,
            worker: Some(worker),
        }
    }

    fn url(&self) -> String {
        format!("http://{}/image", self.address)
    }
    fn wait(&self, predicate: impl Fn() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while !predicate() {
            assert!(
                Instant::now() < deadline,
                "TCP fixture lifecycle condition was not reached"
            );
            thread::sleep(Duration::from_millis(2));
        }
    }
}

impl Drop for KeepaliveServer {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Release);
        self.released.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let result = worker.join();
            if !thread::panicking() {
                result
                    .expect("TCP fixture worker panicked")
                    .expect("TCP fixture failed");
            }
        }
    }
}

fn serve(
    mut stream: TcpStream,
    stopped: &AtomicBool,
    released: &AtomicBool,
    deadline: Instant,
) -> std::result::Result<(), String> {
    stream
        .set_read_timeout(Some(Duration::from_millis(50)))
        .map_err(|error| error.to_string())?;
    stream
        .set_write_timeout(Some(Duration::from_millis(100)))
        .map_err(|error| error.to_string())?;
    while !stopped.load(Ordering::Acquire) && Instant::now() < deadline {
        let mut request = Vec::new();
        let mut fragment = [0u8; 512];
        loop {
            if stopped.load(Ordering::Acquire) || Instant::now() >= deadline {
                return Ok(());
            }
            match stream.read(&mut fragment) {
                Ok(0) => return Ok(()),
                Ok(count) => {
                    if request.len() + count > 4096 {
                        return Err("TCP fixture request exceeded its budget".into());
                    }
                    request.extend_from_slice(&fragment[..count]);
                    if request.windows(4).any(|window| window == b"\r\n\r\n") {
                        break;
                    }
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) => {}
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::BrokenPipe
                    ) =>
                {
                    return Ok(());
                }
                Err(error) => return Err(error.to_string()),
            }
        }
        while !released.load(Ordering::Acquire) {
            if stopped.load(Ordering::Acquire) || Instant::now() >= deadline {
                return Ok(());
            }
            thread::sleep(Duration::from_millis(2));
        }
        if let Err(error) = stream.write_all(
            b"HTTP/1.1 200 Fixture\r\nContent-Length: 1\r\nConnection: keep-alive\r\n\r\nX",
        ) {
            if matches!(
                error.kind(),
                std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::BrokenPipe
            ) {
                return Ok(());
            }
            return Err(error.to_string());
        }
    }
    Ok(())
}

fn fixture_client() -> WeReadClient {
    let mut client = WeReadClient::new(AuthSession::default()).unwrap();
    // Only loopback/proxy/time settings differ from the same production factory.
    client.http = http_client_builder(FIRST_PARTY_IDLE_PER_HOST)
        .no_proxy()
        .connect_timeout(Duration::from_millis(300))
        .timeout(Duration::from_secs(4))
        .build()
        .unwrap();
    client.resource_http = http_client_builder(0)
        .no_proxy()
        .connect_timeout(Duration::from_millis(300))
        .timeout(Duration::from_secs(4))
        .build()
        .unwrap();
    client
}

#[test]
fn destination_policy_keeps_reviewed_reuse_without_restoring_cookie_permission() {
    let client = WeReadClient::new(AuthSession {
        user_vid: "synthetic-pool-account".into(),
        cookies: BTreeMap::from([("wr_skey".into(), "synthetic-pool-cookie".into())]),
        ..Default::default()
    })
    .unwrap();
    for raw in [
        "https://weread.qq.com/image",
        "https://res.weread.qq.com/image",
    ] {
        let url = reqwest::Url::parse(raw).unwrap();
        assert!(std::ptr::eq(client.resource_transport(&url), &client.http));
        let request = client
            .resource_request(&url, "https://weread.qq.com/", false)
            .unwrap()
            .build()
            .unwrap();
        assert!(!request.headers().contains_key("Cookie"));
    }
    for raw in [
        "https://cdn.example.test/image",
        "https://weread.qq.com:8443/image",
        "https://res.weread.qq.com:8443/image",
        "https://weread.qq.com.evil/image",
    ] {
        let url = reqwest::Url::parse(raw).unwrap();
        assert!(std::ptr::eq(
            client.resource_transport(&url),
            &client.resource_http
        ));
        let request = client
            .resource_request(&url, "https://weread.qq.com/", true)
            .unwrap()
            .build()
            .unwrap();
        assert!(!request.headers().contains_key("Cookie"));
    }
}

#[test]
fn first_party_factory_reuses_complete_keepalive_responses() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let server = KeepaliveServer::new(false);
    let client = fixture_client();
    runtime.block_on(async {
        for _ in 0..3 {
            let response = client.http.get(server.url()).send().await.unwrap();
            assert_eq!(response_bytes(response, 1).await.unwrap(), b"X");
        }
    });
    server.wait(|| server.accepted.load(Ordering::Acquire) == 1);
    assert_eq!(server.active.load(Ordering::Acquire), 1);
    drop(client);
    server.wait(|| server.active.load(Ordering::Acquire) == 0);
}

#[test]
fn arbitrary_authorities_do_not_retain_idle_sockets_after_complete_responses() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let servers = [
        KeepaliveServer::new(false),
        KeepaliveServer::new(false),
        KeepaliveServer::new(false),
    ];
    let client = fixture_client();
    for server in &servers {
        let url = reqwest::Url::parse(&server.url()).unwrap();
        for _ in 0..2 {
            runtime.block_on(async {
                let response = client
                    .resource_transport(&url)
                    .get(url.clone())
                    .send()
                    .await
                    .unwrap();
                assert_eq!(response_bytes(response, 1).await.unwrap(), b"X");
            });
            server.wait(|| server.active.load(Ordering::Acquire) == 0);
        }
        assert_eq!(server.accepted.load(Ordering::Acquire), 2);
    }
    assert!(
        servers
            .iter()
            .all(|server| server.active.load(Ordering::Acquire) == 0)
    );
}

#[test]
fn first_party_pool_retires_idle_connections_above_its_explicit_host_cap() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let server = KeepaliveServer::new(true);
    let client = fixture_client();
    let mut work = Vec::new();
    for _ in 0..FIRST_PARTY_IDLE_PER_HOST + 2 {
        let http = client.http.clone();
        let url = server.url();
        work.push(runtime.spawn(async move {
            let response = http.get(url).send().await.unwrap();
            response_bytes(response, 1).await.unwrap()
        }));
    }
    server.wait(|| server.accepted.load(Ordering::Acquire) == FIRST_PARTY_IDLE_PER_HOST + 2);
    server.released.store(true, Ordering::Release);
    runtime.block_on(async {
        for work in work {
            assert_eq!(work.await.unwrap(), b"X");
        }
    });
    server.wait(|| server.active.load(Ordering::Acquire) <= FIRST_PARTY_IDLE_PER_HOST);
    assert_eq!(
        server.active.load(Ordering::Acquire),
        FIRST_PARTY_IDLE_PER_HOST
    );
    drop(client);
    server.wait(|| server.active.load(Ordering::Acquire) == 0);
}
