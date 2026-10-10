#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::net::{SocketAddr, TcpListener};

use trogon_atlas_server::telemetry::install_prometheus_exporter;

fn free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    port
}

#[tokio::test]
async fn exporter_serves_metrics_endpoint() {
    let addr: SocketAddr = format!("127.0.0.1:{}", free_port()).parse().unwrap();
    // loopback address: allow_external flag is irrelevant here
    install_prometheus_exporter(addr, false).expect("install exporter");

    metrics::counter!("rpc_requests_total", "service" => "x", "method" => "Y", "code" => "ok")
        .increment(1);

    // Give the exporter's HTTP server a moment to come up.
    for _ in 0..20 {
        if std::net::TcpStream::connect(addr).is_ok() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }

    let body = tokio::task::spawn_blocking(move || -> std::io::Result<String> {
        use std::io::{Read, Write};
        let mut stream = std::net::TcpStream::connect(addr)?;
        stream.write_all(b"GET /metrics HTTP/1.0\r\nHost: localhost\r\n\r\n")?;
        let mut buf = String::new();
        stream.read_to_string(&mut buf)?;
        Ok(buf)
    })
    .await
    .unwrap()
    .expect("scrape /metrics");

    assert!(
        body.contains("rpc_requests_total"),
        "expected rpc_requests_total in scrape output, got:\n{body}"
    );
}
