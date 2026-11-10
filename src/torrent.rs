//! Nova Studio — Native BitTorrent & Magnet P2P Engine (librqbit)
//!
//! รองรับลิงก์ `magnet:?xt=...` และไฟล์ `.torrent` โดยใช้ Tokio Runtime ร่วมกับทั้งแอป
//! พร้อมการดึงชื่อทอร์เรนต์ ความเร็วแบบเรียลไทม์ และการยกเลิกงาน

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use librqbit::api::TorrentIdOrHash;
use librqbit::{AddTorrent, AddTorrentOptions, AddTorrentResponse, Session};
use tokio::sync::OnceCell;
use url::Url;

use crate::downloader::{default_downloads_dir, sanitize_filename, TurboEvent};
use crate::net_h3::shared_runtime;

static TORRENT_SESSION: OnceCell<Arc<Session>> = OnceCell::const_new();

pub fn is_torrent_or_magnet(raw_url: &str) -> bool {
    let lower = raw_url.trim().to_ascii_lowercase();
    if lower.starts_with("magnet:") {
        return true;
    }
    let path_part = lower.split('?').next().unwrap_or("");
    path_part.ends_with(".torrent") || lower.contains(".torrent?")
}

/// ดึงชื่อ `dn=` จาก Magnet URI มาแสดงเบื้องต้นก่อน metadata ดาวน์โหลดเสร็จ
pub fn extract_magnet_display_name(raw_url: &str) -> Option<String> {
    let parsed = Url::parse(raw_url).ok()?;
    if parsed.scheme() != "magnet" {
        return None;
    }
    for (k, v) in parsed.query_pairs() {
        if k == "dn" && !v.trim().is_empty() {
            return Some(sanitize_filename(&v));
        }
    }
    None
}

async fn get_or_init_session(save_dir: PathBuf) -> Result<Arc<Session>, String> {
    TORRENT_SESSION
        .get_or_try_init(|| async move {
            std::fs::create_dir_all(&save_dir)
                .map_err(|e| format!("สร้างโฟลเดอร์ Torrent ไม่สำเร็จ: {e}"))?;
            Session::new(save_dir)
                .await
                .map_err(|e| format!("เริ่มต้น BitTorrent Session ไม่สำเร็จ: {e}"))
        })
        .await
        .cloned()
}

pub fn run_torrent_download<F>(
    dl_id: String,
    url: String,
    save_dir_opt: Option<PathBuf>,
    cancel_flag: Arc<AtomicBool>,
    emit: F,
) where
    F: Fn(TurboEvent) + Send + Sync + 'static,
{
    let base_dir = save_dir_opt.unwrap_or_else(|| default_downloads_dir().join("NovaTorrents"));
    let fallback_name = extract_magnet_display_name(&url)
        .unwrap_or_else(|| "BitTorrent Download".to_string());

    emit(TurboEvent::Started {
        id: dl_id.clone(),
        file_name: fallback_name.clone(),
        url: url.clone(),
        path: base_dir.display().to_string(),
        total_bytes: 0,
        mode: "BitTorrent P2P".to_string(),
    });

    shared_runtime().block_on(async move {
        let session = match get_or_init_session(base_dir.clone()).await {
            Ok(s) => s,
            Err(err) => {
                emit(TurboEvent::Finished {
                    id: dl_id,
                    success: false,
                    file_name: fallback_name,
                    path: base_dir.display().to_string(),
                    message: err,
                });
                return;
            }
        };

        let opts = AddTorrentOptions {
            overwrite: true,
            ..Default::default()
        };

        let added = session
            .add_torrent(AddTorrent::from_url(&url), Some(opts))
            .await;

        let handle = match added {
            Ok(AddTorrentResponse::Added(_, h)) | Ok(AddTorrentResponse::AlreadyManaged(_, h)) => h,
            Ok(AddTorrentResponse::ListOnly(_)) => {
                emit(TurboEvent::Finished {
                    id: dl_id,
                    success: false,
                    file_name: fallback_name,
                    path: base_dir.display().to_string(),
                    message: "โหมดแสดงรายชื่ออย่างเดียวไม่รองรับการโหลดอัตโนมัติ".into(),
                });
                return;
            }
            Err(e) => {
                emit(TurboEvent::Finished {
                    id: dl_id,
                    success: false,
                    file_name: fallback_name,
                    path: base_dir.display().to_string(),
                    message: format!("เพิ่ม Torrent ไม่สำเร็จ: {e}"),
                });
                return;
            }
        };

        let mut last_name = handle.name().unwrap_or_else(|| fallback_name.clone());
        let torrent_path = base_dir.join(&last_name);
        let tid = handle.id();

        loop {
            tokio::time::sleep(Duration::from_millis(450)).await;

            if cancel_flag.load(Ordering::Relaxed) {
                let _ = session.delete(TorrentIdOrHash::Id(tid), false).await;
                emit(TurboEvent::Finished {
                    id: dl_id,
                    success: false,
                    file_name: last_name,
                    path: torrent_path.display().to_string(),
                    message: "ยกเลิก Torrent แล้ว".into(),
                });
                return;
            }

            let stats = handle.stats();
            if let Some(real_name) = handle.name() {
                if real_name != last_name {
                    last_name = real_name;
                    emit(TurboEvent::Started {
                        id: dl_id.clone(),
                        file_name: last_name.clone(),
                        url: url.clone(),
                        path: base_dir.join(&last_name).display().to_string(),
                        total_bytes: stats.total_bytes,
                        mode: "BitTorrent P2P".to_string(),
                    });
                }
            }

            if let Some(err) = stats.error {
                emit(TurboEvent::Finished {
                    id: dl_id,
                    success: false,
                    file_name: last_name.clone(),
                    path: base_dir.join(&last_name).display().to_string(),
                    message: format!("Torrent ผิดพลาด: {err}"),
                });
                return;
            }

            let pct = if stats.total_bytes > 0 {
                ((stats.progress_bytes as f64 / stats.total_bytes as f64) * 100.0).min(100.0) as u32
            } else {
                0
            };
            let speed_bps = stats
                .live
                .as_ref()
                .map(|l| l.download_speed.as_bytes())
                .unwrap_or(0);
            let peers_note = stats
                .live
                .as_ref()
                .map(|l| format!("P2P ({} peers)", l.snapshot.peer_stats.live))
                .unwrap_or_else(|| "P2P (DHT...)".to_string());

            emit(TurboEvent::Progress {
                id: dl_id.clone(),
                downloaded_bytes: stats.progress_bytes,
                total_bytes: stats.total_bytes,
                pct,
                speed_bps,
                mode: peers_note,
            });

            if stats.finished {
                emit(TurboEvent::Finished {
                    id: dl_id,
                    success: true,
                    file_name: last_name.clone(),
                    path: base_dir.join(&last_name).display().to_string(),
                    message: "ดาวน์โหลด Torrent สำเร็จ!".into(),
                });
                return;
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn magnet_and_torrent_detection_works() {
        assert!(is_torrent_or_magnet("magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567&dn=Ubuntu+24.04"));
        assert!(is_torrent_or_magnet("https://releases.ubuntu.com/24.04/ubuntu-24.04.torrent?token=abc"));
        assert!(!is_torrent_or_magnet("https://example.com/video.mp4"));

        let name = extract_magnet_display_name(
            "magnet:?xt=urn:btih:1234&dn=Ubuntu%2024.04%20LTS.iso&tr=udp://tracker.opentrackr.org:1337"
        );
        assert_eq!(name.as_deref(), Some("Ubuntu 24.04 LTS.iso"));
    }
}
