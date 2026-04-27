use super::CmdResult;
use crate::{
    cmd::StringifyErr as _,
    config::{Config, kcp_connect_host, kcp_listen_host},
    core::KcptunManager,
};
use anyhow::{Context as _, Result, anyhow, bail};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use std::time::{Duration, Instant};
use tokio::{
    io::{AsyncReadExt as _, AsyncWriteExt as _},
    net::TcpStream,
};
use tokio_rustls::{
    TlsConnector,
    rustls::{ClientConfig, RootCertStore, pki_types::ServerName},
};

#[tauri::command]
pub async fn start_kcptun_client() -> CmdResult {
    KcptunManager::global().start().await.stringify_err()
}

#[tauri::command]
pub async fn stop_kcptun_client() -> CmdResult {
    KcptunManager::global().stop().await;
    Ok(())
}

#[tauri::command]
pub async fn restart_kcptun_client() -> CmdResult {
    KcptunManager::global().restart().await.stringify_err()
}

#[tauri::command]
pub async fn get_kcptun_status() -> CmdResult<bool> {
    Ok(KcptunManager::global().is_running().await)
}

#[tauri::command]
pub async fn test_kcptun_upstream_proxy() -> CmdResult<String> {
    test_upstream_proxy().await.stringify_err()
}

async fn test_upstream_proxy() -> Result<String> {
    const TEST_HOST: &str = "ipv4.icanhazip.com";
    const TEST_PORT: u16 = 443;
    const MAX_HEADER_SIZE: usize = 16 * 1024;
    const MAX_RESPONSE_SIZE: usize = 64 * 1024;

    let verge = Config::verge().await;
    let verge_arc = verge.latest_arc();
    let config = verge_arc
        .kcp_proxy
        .clone()
        .ok_or_else(|| anyhow!("KCP Tunnel config is missing"))?;
    let proxy_host = verge_arc.proxy_host.clone();
    drop(verge_arc);
    drop(verge);

    let allow_lan = Config::clash()
        .await
        .latest_arc()
        .0
        .get("allow-lan")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);

    let local_port = match config.local_port.unwrap_or(1087) {
        0 => 1087,
        port => port,
    };
    let listen_host = kcp_listen_host(allow_lan, proxy_host.as_deref());
    let connect_host = kcp_connect_host(&listen_host);
    let proxy_addr = format_endpoint(&connect_host, local_port);
    let target = format!("{TEST_HOST}:{TEST_PORT}");
    let mut logs = Vec::<String>::new();
    let started = Instant::now();

    logs.push(format!("* Test URL: https://{TEST_HOST}/"));
    logs.push(format!("* Proxy: http://{proxy_addr}"));
    logs.push(format!("* CONNECT target: {target}"));

    let mut stream = tokio::time::timeout(Duration::from_secs(10), TcpStream::connect(&proxy_addr))
        .await
        .context("timed out connecting to local kcptun HTTP proxy")?
        .with_context(|| format!("failed to connect to local kcptun HTTP proxy {proxy_addr}"))?;
    logs.push(format!("* Connected to {proxy_addr}"));

    let mut connect_req = format!(
        "CONNECT {target} HTTP/1.1\r\nHost: {target}\r\nUser-Agent: Clash-Verge-KcpTunnel-Test/1.0\r\nProxy-Connection: Keep-Alive\r\n"
    );

    if let Some(username) = config.proxy_username.as_deref().filter(|value| !value.is_empty()) {
        let password = config.proxy_password.as_deref().unwrap_or_default();
        let token = STANDARD.encode(format!("{username}:{password}"));
        connect_req.push_str(&format!("Proxy-Authorization: Basic {token}\r\n"));
        logs.push("* Proxy auth: Basic <redacted>".into());
    }
    connect_req.push_str("\r\n");

    logs.push(format!("> CONNECT {target} HTTP/1.1"));
    logs.push(format!("> Host: {target}"));
    tokio::time::timeout(Duration::from_secs(10), stream.write_all(connect_req.as_bytes()))
        .await
        .context("timed out sending CONNECT request")?
        .context("failed to send CONNECT request")?;

    let connect_header = read_http_header(&mut stream, MAX_HEADER_SIZE).await?;
    for line in connect_header.lines() {
        logs.push(format!("< {line}"));
    }

    let status_line = connect_header.lines().next().unwrap_or_default();
    if status_line.split_whitespace().nth(1) != Some("200") {
        logs.push(format!("* CONNECT failed after {} ms", started.elapsed().as_millis()));
        return Ok(logs.join("\n"));
    }
    logs.push("* CONNECT tunnel established".into());

    let mut roots = RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let tls_config = ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    let connector = TlsConnector::from(std::sync::Arc::new(tls_config));
    let server_name = ServerName::try_from(TEST_HOST.to_string())
        .map_err(|err| anyhow!("invalid TLS server name {TEST_HOST}: {err}"))?;

    logs.push(format!("* TLS handshake with {TEST_HOST}"));
    let mut tls_stream = tokio::time::timeout(Duration::from_secs(10), connector.connect(server_name, stream))
        .await
        .context("timed out during TLS handshake")?
        .context("TLS handshake failed")?;
    logs.push("* TLS handshake completed".into());

    let get_req = format!(
        "GET / HTTP/1.1\r\nHost: {TEST_HOST}\r\nUser-Agent: Clash-Verge-KcpTunnel-Test/1.0\r\nAccept: */*\r\nConnection: close\r\n\r\n"
    );
    logs.push("> GET / HTTP/1.1".into());
    logs.push(format!("> Host: {TEST_HOST}"));
    tokio::time::timeout(Duration::from_secs(10), tls_stream.write_all(get_req.as_bytes()))
        .await
        .context("timed out sending HTTPS request")?
        .context("failed to send HTTPS request")?;

    let response = tokio::time::timeout(
        Duration::from_secs(20),
        read_to_end_limited(&mut tls_stream, MAX_RESPONSE_SIZE),
    )
    .await
    .context("timed out reading HTTPS response")?
    .context("failed to read HTTPS response")?;

    let response = String::from_utf8_lossy(&response);
    let (headers, body) = response
        .split_once("\r\n\r\n")
        .or_else(|| response.split_once("\n\n"))
        .unwrap_or((&response, ""));

    for line in headers.lines().take(32) {
        logs.push(format!("< {line}"));
    }

    let body = body.trim();
    if !body.is_empty() {
        logs.push(String::new());
        logs.push(body.into());
    }
    logs.push(format!("* Done in {} ms", started.elapsed().as_millis()));

    Ok(logs.join("\n"))
}

fn format_endpoint(host: &str, port: u16) -> String {
    if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

async fn read_to_end_limited<T>(stream: &mut T, max_size: usize) -> Result<Vec<u8>>
where
    T: tokio::io::AsyncRead + Unpin,
{
    let mut data = Vec::new();
    let mut buf = [0_u8; 4096];

    loop {
        let read = stream.read(&mut buf).await?;
        if read == 0 {
            return Ok(data);
        }

        data.extend_from_slice(&buf[..read]);
        if data.len() > max_size {
            bail!("HTTPS response exceeded {max_size} bytes");
        }
    }
}

async fn read_http_header(stream: &mut TcpStream, max_size: usize) -> Result<String> {
    let mut data = Vec::new();
    let mut buf = [0_u8; 1024];

    loop {
        let read = tokio::time::timeout(Duration::from_secs(10), stream.read(&mut buf))
            .await
            .context("timed out reading CONNECT response")?
            .context("failed to read CONNECT response")?;

        if read == 0 {
            bail!("connection closed before CONNECT response completed");
        }

        data.extend_from_slice(&buf[..read]);
        if data.windows(4).any(|item| item == b"\r\n\r\n") || data.windows(2).any(|item| item == b"\n\n") {
            return Ok(String::from_utf8_lossy(&data).into_owned());
        }

        if data.len() > max_size {
            bail!("CONNECT response header exceeded {max_size} bytes");
        }
    }
}
