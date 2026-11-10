//! Nova Studio — Native HTTP/3 (QUIC) Client & Shared Async Runtime
//!
//! เชื่อมต่อและดาวน์โหลดไฟล์ผ่านโปรโตคอล HTTP/3 บน QUIC (quinn + h3 + h3-quinn)
//! ใช้ร่วมกับ Windows Platform Certificate Verifier และแชร์ Tokio Runtime เดียวกันทั้งแอป

use std::fs::OpenOptions;
use std::io::{BufWriter, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, ToSocketAddrs};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use bytes::Buf;
use http::Request;
use rustls::ClientConfig;
use rustls_platform_verifier::ConfigVerifierExt;
use url::Url;

static SHARED_TOKIO: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

/// Tokio Multi-Thread Runtime กลาง (ใช้ร่วมกันทั้ง HTTP/3 และ BitTorrent Engine)
pub fn shared_runtime() -> &'static tokio::runtime::Runtime {
    SHARED_TOKIO.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .thread_name("nova-async-io")
            .build()
            .expect("Failed to initialize Nova Tokio runtime")
    })
}

pub fn build_quinn_client_config() -> Result<quinn::ClientConfig, String> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let mut tls_cfg = ClientConfig::with_platform_verifier()
        .map_err(|e| format!("ตั้งค่า TLS สำหรับ QUIC ไม่สำเร็จ: {e}"))?;
    tls_cfg.alpn_protocols = vec![b"h3".to_vec()];
    let quic_crypto = quinn::crypto::rustls::QuicClientConfig::try_from(tls_cfg)
        .map_err(|e| format!("สร้าง QuicClientConfig ไม่สำเร็จ: {e}"))?;
    Ok(quinn::ClientConfig::new(Arc::new(quic_crypto)))
}

#[derive(Debug)]
pub struct H3DownloadResult {
    pub total_bytes: u64,
    pub content_disposition: Option<String>,
}

/// ดาวน์โหลดไฟล์ผ่าน HTTP/3 (QUIC) โดยตรง พร้อมรายงานความคืบหน้าและรองรับการยกเลิก
pub fn download_via_h3<F>(
    url_str: &str,
    referer: Option<&str>,
    cookie_header: Option<&str>,
    part_path: &Path,
    cancel_flag: &Arc<AtomicBool>,
    emit_progress: F,
) -> Result<H3DownloadResult, String>
where
    F: Fn(u64, u64, u32, u64) + Send + Sync,
{
    let parsed = Url::parse(url_str).map_err(|e| format!("URL ไม่ถูกต้อง: {e}"))?;
    if parsed.scheme() != "https" {
        return Err("HTTP/3 รองรับเฉพาะ HTTPS".into());
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| "ไม่พบชื่อโฮสต์ใน URL".to_string())?
        .to_string();
    let port = parsed.port_or_known_default().unwrap_or(443);

    let remote_addr: SocketAddr = (host.as_str(), port)
        .to_socket_addrs()
        .map_err(|e| format!("ค้นหา IP ของ {host} ไม่สำเร็จ: {e}"))?
        .next()
        .ok_or_else(|| format!("ไม่พบ IP ปลายทางสำหรับ {host}"))?;

    let bind_addr = match remote_addr.ip() {
        IpAddr::V4(_) => SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0),
        IpAddr::V6(_) => SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), 0),
    };

    let client_cfg = build_quinn_client_config()?;
    let uri_owned = url_str.to_string();
    let referer_owned = referer.map(str::to_string);
    let cookie_owned = cookie_header.map(str::to_string);
    let part_owned = part_path.to_path_buf();
    let cancel_ref = Arc::clone(cancel_flag);

    shared_runtime().block_on(async move {
        let mut endpoint = quinn::Endpoint::client(bind_addr)
            .map_err(|e| format!("เปิด UDP Endpoint สำหรับ QUIC ไม่สำเร็จ: {e}"))?;
        endpoint.set_default_client_config(client_cfg);

        let connecting = endpoint
            .connect(remote_addr, &host)
            .map_err(|e| format!("เริ่ม QUIC connect ไม่สำเร็จ: {e}"))?;
        let conn = tokio::time::timeout(Duration::from_secs(5), connecting)
            .await
            .map_err(|_| "QUIC Handshake หมดเวลา (เซิร์ฟเวอร์อาจปิด UDP/443)".to_string())?
            .map_err(|e| format!("QUIC Handshake ล้มเหลว: {e}"))?;

        let (mut driver, mut send_req) = h3::client::new(h3_quinn::Connection::new(conn))
            .await
            .map_err(|e| format!("สร้าง HTTP/3 session ไม่สำเร็จ: {e}"))?;

        let driver_handle = tokio::spawn(async move {
            let _ = std::future::poll_fn(|cx| driver.poll_close(cx)).await;
        });

        let mut req_builder = Request::get(&uri_owned)
            .header(
                "user-agent",
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36",
            )
            .header("accept", "*/*");
        if let Some(ref r) = referer_owned {
            if !r.trim().is_empty() && !r.starts_with("about:") {
                req_builder = req_builder.header("referer", r.as_str());
            }
        }
        if let Some(ref c) = cookie_owned {
            if !c.trim().is_empty() {
                req_builder = req_builder.header("cookie", c.as_str());
            }
        }

        let req = req_builder
            .body(())
            .map_err(|e| format!("สร้าง HTTP/3 Request ไม่สำเร็จ: {e}"))?;
        let mut stream = send_req
            .send_request(req)
            .await
            .map_err(|e| format!("ส่ง HTTP/3 Request ไม่สำเร็จ: {e}"))?;
        stream
            .finish()
            .await
            .map_err(|e| format!("ปิดส่วนส่ง Request ไม่สำเร็จ: {e}"))?;

        let resp = stream
            .recv_response()
            .await
            .map_err(|e| format!("รับ HTTP/3 Response ไม่สำเร็จ: {e}"))?;

        if resp.status().as_u16() >= 400 {
            driver_handle.abort();
            return Err(format!("เซิร์ฟเวอร์ HTTP/3 ตอบกลับ HTTP {}", resp.status().as_u16()));
        }

        let content_len = resp
            .headers()
            .get("content-length")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.trim().parse::<u64>().ok())
            .unwrap_or(0);
        let content_disposition = resp
            .headers()
            .get("content-disposition")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);

        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&part_owned)
            .map_err(|e| format!("สร้างไฟล์ชั่วคราวไม่สำเร็จ: {e}"))?;
        let mut writer = BufWriter::with_capacity(128 * 1024, file);

        let mut downloaded = 0u64;
        let mut last_bytes = 0u64;
        let mut last_tick = Instant::now();

        while let Some(mut chunk) = stream
            .recv_data()
            .await
            .map_err(|e| format!("อ่านข้อมูล HTTP/3 ขัดข้อง: {e}"))?
        {
            if cancel_ref.load(Ordering::Relaxed) {
                break;
            }
            while chunk.has_remaining() {
                let bytes = chunk.chunk();
                let len = bytes.len();
                writer
                    .write_all(bytes)
                    .map_err(|e| format!("เขียนไฟล์ไม่สำเร็จ: {e}"))?;
                downloaded += len as u64;
                chunk.advance(len);
            }

            let elapsed = last_tick.elapsed();
            if elapsed >= Duration::from_millis(220) {
                let speed = ((downloaded - last_bytes) as f64 / elapsed.as_secs_f64().max(0.05)) as u64;
                let pct = if content_len > 0 {
                    ((downloaded as f64 / content_len as f64) * 100.0).min(100.0) as u32
                } else {
                    0
                };
                last_bytes = downloaded;
                last_tick = Instant::now();
                emit_progress(downloaded, content_len, pct, speed);
            }
        }

        writer.flush().map_err(|e| format!("flush ไฟล์ไม่สำเร็จ: {e}"))?;
        endpoint.wait_idle().await;
        driver_handle.abort();

        Ok(H3DownloadResult {
            total_bytes: downloaded,
            content_disposition,
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn h3_client_config_initializes_cleanly() {
        let cfg = build_quinn_client_config();
        assert!(cfg.is_ok(), "QUIC/HTTP3 TLS config should initialize cleanly: {:?}", cfg.err());
    }
}
