//! Process isolation keeps proxy environment changes out of concurrent tests.
use rustrss_core::fetch::{CacheHeaders, FetchResult, Fetcher};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn proxy_child() {
    let Ok(url) = std::env::var("RUSTRSS_PROXY_TEST_URL") else {
        return;
    };
    let result = Fetcher::new("proxy-fixture")
        .unwrap()
        .fetch(&url, CacheHeaders::default())
        .await;
    match std::env::var("RUSTRSS_PROXY_TEST_EXPECT").unwrap().as_str() {
        "ok" => assert!(matches!(result, FetchResult::Fetched { .. }), "{result:?}"),
        "407" => assert!(
            matches!(result, FetchResult::Failed { ref code, .. } if code == "http_407"),
            "{result:?}"
        ),
        _ => panic!("unknown fixture expectation"),
    }
}

async fn run_child(url: String, proxy: String, bypass: &str, expected: &str) {
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command.args(["--exact", "proxy_child", "--nocapture"]);
    for key in [
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "NO_PROXY",
        "http_proxy",
        "https_proxy",
        "all_proxy",
        "no_proxy",
    ] {
        command.env_remove(key);
    }
    command
        .env("HTTP_PROXY", proxy)
        .env("NO_PROXY", bypass)
        .env("RUSTRSS_PROXY_TEST_URL", url)
        .env("RUSTRSS_PROXY_TEST_EXPECT", expected);
    let output = tokio::task::spawn_blocking(move || command.output().unwrap())
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[tokio::test]
async fn environment_proxy_and_no_proxy_are_honored() {
    let proxy = MockServer::start().await;
    Mock::given(wiremock::matchers::method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_string("via proxy"))
        .expect(1)
        .mount(&proxy)
        .await;
    run_child(
        "http://proxy-fixture.invalid/feed".into(),
        proxy.uri(),
        "",
        "ok",
    )
    .await;
    let origin = MockServer::start().await;
    Mock::given(wiremock::matchers::method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_string("direct"))
        .expect(1)
        .mount(&origin)
        .await;
    run_child(origin.uri(), proxy.uri(), "127.0.0.1", "ok").await;
}

#[tokio::test]
async fn proxy_authentication_failure_is_reported() {
    let proxy = MockServer::start().await;
    Mock::given(wiremock::matchers::method("GET"))
        .respond_with(ResponseTemplate::new(407))
        .expect(1)
        .mount(&proxy)
        .await;
    run_child(
        "http://proxy-fixture.invalid/feed".into(),
        proxy.uri(),
        "",
        "407",
    )
    .await;
}

#[tokio::test]
async fn custom_proxy_switch_and_direct_bypass_take_effect() {
    use rustrss_core::network::{ProxyConfig, ProxyMode};
    let proxy = MockServer::start().await;
    Mock::given(wiremock::matchers::method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_string("proxy"))
        .expect(1)
        .mount(&proxy)
        .await;
    let origin = MockServer::start().await;
    Mock::given(wiremock::matchers::method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_string("origin"))
        .expect(2)
        .mount(&origin)
        .await;
    let base = Fetcher::new("proxy-fixture").unwrap();
    let mut config = ProxyConfig {
        mode: ProxyMode::Custom,
        url: proxy.uri(),
        no_proxy: String::new(),
    };
    let proxied = base.configured(&config).unwrap();
    match proxied.fetch(&origin.uri(), CacheHeaders::default()).await {
        FetchResult::Fetched { body, .. } => assert_eq!(body, b"proxy"),
        other => panic!("{other:?}"),
    }
    config.no_proxy = "127.0.0.1".into();
    let bypassed = base.configured(&config).unwrap();
    match bypassed.fetch(&origin.uri(), CacheHeaders::default()).await {
        FetchResult::Fetched { body, .. } => assert_eq!(body, b"origin"),
        other => panic!("{other:?}"),
    }
    config.mode = ProxyMode::Direct;
    match base
        .configured(&config)
        .unwrap()
        .fetch(&origin.uri(), CacheHeaders::default())
        .await
    {
        FetchResult::Fetched { body, .. } => assert_eq!(body, b"origin"),
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn https_uses_connect_and_reports_tunnel_rejection() {
    use std::io::{Read, Write};
    use std::time::{Duration, Instant};
    use rustrss_core::network::{ProxyConfig, ProxyMode};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    listener.set_nonblocking(true).unwrap();
    let server = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock && Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
                Err(e) => panic!("proxy fixture accept: {e}"),
            }
        };
        stream.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        let mut request = [0; 4096];
        let size = stream.read(&mut request).unwrap();
        let request = String::from_utf8_lossy(&request[..size]).into_owned();
        stream.write_all(b"HTTP/1.1 407 Proxy Authentication Required\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
        request
    });
    let config = ProxyConfig { mode: ProxyMode::Custom, url: format!("http://{address}"), no_proxy: String::new() };
    let result = Fetcher::new("proxy-fixture").unwrap().configured(&config).unwrap()
        .fetch("https://proxy-fixture.invalid/feed", CacheHeaders::default()).await;
    let request = server.join().unwrap();
    assert!(request.starts_with("CONNECT proxy-fixture.invalid:443 HTTP/1.1\r\n"), "{request}");
    assert!(matches!(result, FetchResult::Failed { .. }), "{result:?}");
}

#[tokio::test]
async fn unavailable_proxy_fails_without_direct_fallback() {
    use rustrss_core::network::{ProxyConfig, ProxyMode};
    let origin = MockServer::start().await;
    Mock::given(wiremock::matchers::method("GET"))
        .respond_with(ResponseTemplate::new(200)).expect(0).mount(&origin).await;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let config = ProxyConfig { mode: ProxyMode::Custom, url: format!("http://{address}"), no_proxy: String::new() };
    let result = Fetcher::new("proxy-fixture").unwrap().configured(&config).unwrap()
        .fetch(&origin.uri(), CacheHeaders::default()).await;
    assert!(matches!(result, FetchResult::Failed { ref code, .. } if code == "connection_error"), "{result:?}");
}
