//! Nova Studio — Native Multi-Segment Turbo Downloader & HLS Stream Engine
//!
//! รองรับการดาวน์โหลดแบบแบ่งหลายเธรดพร้อมกัน (HTTP Range / Segmented Download),
//! ส่งต่อ Cookie/Referer จากเซสชันของ WebView2, ดักจับและรวมไฟล์สตรีม HLS (.m3u8),
//! พร้อมระบบเปลี่ยนชื่อไฟล์อัตโนมัติป้องกันการเขียนทับ

use std::fs::{self, File, OpenOptions};
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

#[cfg(windows)]
use std::os::windows::fs::FileExt;
#[cfg(unix)]
use std::os::unix::fs::FileExt;

use ureq::tls::{RootCerts, TlsConfig, TlsProvider};
use ureq::ResponseExt;
use url::Url;

const CHROME_STEALTH_UA: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36 Edg/141.0.0.0";
const MIN_SEGMENTED_SIZE: u64 = 256 * 1024; // 256 KB ขึ้นไปจึงแบ่งเธรด
const MAX_RETRY_PER_SEGMENT: usize = 3;

#[derive(Debug, Clone)]
pub struct TurboRequest {
    pub id: String,
    pub url: String,
    pub suggested_filename: Option<String>,
    pub referer: Option<String>,
    pub cookie_header: Option<String>,
    pub segments: usize,
    pub prefer_http3: bool,
    pub save_dir: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub enum TurboEvent {
    Started {
        id: String,
        file_name: String,
        url: String,
        path: String,
        total_bytes: u64,
        mode: String,
    },
    Progress {
        id: String,
        downloaded_bytes: u64,
        total_bytes: u64,
        pct: u32,
        speed_bps: u64,
        mode: String,
    },
    Finished {
        id: String,
        success: bool,
        file_name: String,
        path: String,
        message: String,
    },
}

/// โฟลเดอร์ Downloads มาตรฐานของผู้ใช้บนระบบปฏิบัติการ
pub fn default_downloads_dir() -> PathBuf {
    if let Some(home) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) {
        let dl = PathBuf::from(home).join("Downloads");
        if dl.exists() || fs::create_dir_all(&dl).is_ok() {
            return dl;
        }
    }
    PathBuf::from(".")
}

/// ป้องกันการเขียนทับไฟล์เดิม: ถ้ามีไฟล์ซ้ำจะเปลี่ยนเป็น `name (1).ext`, `name (2).ext` ...
pub fn unique_download_path(original: &Path) -> PathBuf {
    if !original.exists() {
        return original.to_path_buf();
    }
    let parent = original.parent().map(|p| p.to_path_buf()).unwrap_or_default();
    let stem = original
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "download".into());
    let ext = original
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();

    for n in 1..10_000 {
        let candidate = parent.join(format!("{stem} ({n}){ext}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    original.to_path_buf()
}

/// ทำความสะอาดชื่อไฟล์ให้ปลอดภัยบน Windows และคงภาษาไทยไว้ครบถ้วน
pub fn sanitize_filename(raw: &str) -> String {
    let trimmed = raw.trim().trim_matches('"').trim_matches('\'');
    let mut cleaned: String = trimmed
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    cleaned = cleaned.trim().trim_matches('.').to_string();
    if cleaned.is_empty() {
        "nova_download".to_string()
    } else if cleaned.chars().count() > 140 {
        cleaned.chars().take(140).collect()
    } else {
        cleaned
    }
}

/// ดึงชื่อไฟล์จาก Header `Content-Disposition` (รองรับทั้ง `filename*=UTF-8''...` และ `filename="..."`)
pub fn parse_content_disposition_filename(header: &str) -> Option<String> {
    let lower = header.to_ascii_lowercase();
    if let Some(pos) = lower.find("filename*=") {
        let tail = header[pos + 10..].split(';').next()?.trim();
        let encoded = tail
            .strip_prefix("UTF-8''")
            .or_else(|| tail.strip_prefix("utf-8''"))
            .unwrap_or(tail);
        let decoded = percent_decode_utf8(encoded);
        let s = sanitize_filename(&decoded);
        if !s.is_empty() && s != "nova_download" {
            return Some(s);
        }
    }
    if let Some(pos) = lower.find("filename=") {
        let tail = header[pos + 9..].split(';').next()?.trim();
        let unquoted = tail.trim_matches('"').trim_matches('\'');
        let decoded = percent_decode_utf8(unquoted);
        let s = sanitize_filename(&decoded);
        if !s.is_empty() && s != "nova_download" {
            return Some(s);
        }
    }
    None
}

fn percent_decode_utf8(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(hex) = std::str::from_utf8(&bytes[i + 1..i + 3]) {
                if let Ok(val) = u8::from_str_radix(hex, 16) {
                    out.push(val);
                    i += 3;
                    continue;
                }
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

pub fn extract_filename_from_url(url_str: &str) -> Option<String> {
    let parsed = Url::parse(url_str).ok()?;
    let seg = parsed.path_segments()?.next_back()?;
    if seg.is_empty() {
        return None;
    }
    let decoded = percent_decode_utf8(seg);
    let sanitized = sanitize_filename(&decoded);
    if sanitized.is_empty() || sanitized == "nova_download" {
        None
    } else {
        Some(sanitized)
    }
}

pub fn parse_content_range_total(header: &str) -> Option<u64> {
    let slash_pos = header.rfind('/')?;
    let total_str = header[slash_pos + 1..].trim();
    if total_str == "*" {
        return None;
    }
    total_str.parse::<u64>().ok()
}

fn build_http_agent() -> ureq::Agent {
    let tls = TlsConfig::builder()
        .provider(TlsProvider::NativeTls)
        .root_certs(RootCerts::PlatformVerifier)
        .build();
    ureq::Agent::config_builder()
        .tls_config(tls)
        .timeout_connect(Some(Duration::from_secs(15)))
        .timeout_recv_response(Some(Duration::from_secs(30)))
        .http_status_as_error(false)
        .build()
        .into()
}

fn attach_headers(
    mut req: ureq::RequestBuilder<ureq::typestate::WithoutBody>,
    referer: &Option<String>,
    cookie_header: &Option<String>,
) -> ureq::RequestBuilder<ureq::typestate::WithoutBody> {
    req = req
        .header("User-Agent", CHROME_STEALTH_UA)
        .header("Accept", "*/*");
    if let Some(r) = referer {
        if !r.trim().is_empty() && !r.starts_with("about:") {
            req = req.header("Referer", r);
        }
    }
    if let Some(c) = cookie_header {
        if !c.trim().is_empty() {
            req = req.header("Cookie", c);
        }
    }
    req
}

#[derive(Debug)]
struct ProbeInfo {
    total_bytes: Option<u64>,
    accept_ranges: bool,
    alt_svc_h3: bool,
    cd_filename: Option<String>,
    content_type: String,
    final_url: String,
}

fn probe_target(
    agent: &ureq::Agent,
    url: &str,
    referer: &Option<String>,
    cookie_header: &Option<String>,
) -> ProbeInfo {
    let mut info = ProbeInfo {
        total_bytes: None,
        accept_ranges: false,
        alt_svc_h3: false,
        cd_filename: None,
        content_type: String::new(),
        final_url: url.to_string(),
    };

    if let Ok(res) = attach_headers(agent.head(url), referer, cookie_header).call() {
        let status = res.status().as_u16();
        info.final_url = res.get_uri().to_string();
        if status < 400 {
            if let Some(alt) = res.headers().get("alt-svc").and_then(|v| v.to_str().ok()) {
                if alt.to_ascii_lowercase().contains("h3=") {
                    info.alt_svc_h3 = true;
                }
            }
            if let Some(ar) = res.headers().get("accept-ranges").and_then(|v| v.to_str().ok()) {
                if ar.eq_ignore_ascii_case("bytes") {
                    info.accept_ranges = true;
                }
            }
            if let Some(cl) = res.headers().get("content-length").and_then(|v| v.to_str().ok()) {
                info.total_bytes = cl.trim().parse::<u64>().ok();
            }
            if let Some(cd) = res.headers().get("content-disposition").and_then(|v| v.to_str().ok()) {
                info.cd_filename = parse_content_disposition_filename(cd);
            }
            if let Some(ct) = res.headers().get("content-type").and_then(|v| v.to_str().ok()) {
                info.content_type = ct.to_ascii_lowercase();
            }
            if info.total_bytes.is_some() {
                return info;
            }
        }
    }

    // Fallback: ทดลอง GET Range: bytes=0-0 สำหรับเซิร์ฟเวอร์ที่บล็อก HEAD
    if let Ok(res) = attach_headers(agent.get(url), referer, cookie_header)
        .header("Range", "bytes=0-0")
        .call()
    {
        let status = res.status().as_u16();
        info.final_url = res.get_uri().to_string();
        if status == 206 {
            info.accept_ranges = true;
            if let Some(cr) = res.headers().get("content-range").and_then(|v| v.to_str().ok()) {
                info.total_bytes = parse_content_range_total(cr);
            }
        } else if status < 400 {
            if let Some(cl) = res.headers().get("content-length").and_then(|v| v.to_str().ok()) {
                info.total_bytes = cl.trim().parse::<u64>().ok();
            }
        }
        if info.cd_filename.is_none() {
            if let Some(cd) = res.headers().get("content-disposition").and_then(|v| v.to_str().ok()) {
                info.cd_filename = parse_content_disposition_filename(cd);
            }
        }
        if info.content_type.is_empty() {
            if let Some(ct) = res.headers().get("content-type").and_then(|v| v.to_str().ok()) {
                info.content_type = ct.to_ascii_lowercase();
            }
        }
    }

    info
}

#[derive(Debug, PartialEq)]
pub enum M3u8Kind {
    Master { best_variant_url: String },
    Media { init_url: Option<String>, segments: Vec<String> },
    EncryptedError(String),
}

/// แปลงเนื้อหาเพลย์ลิสต์ HLS (.m3u8) เป็นรายการ Variant หรือ Segment URL แบบสมบูรณ์
pub fn parse_m3u8(playlist_text: &str, base_url: &str) -> Result<M3u8Kind, String> {
    let base = Url::parse(base_url).map_err(|e| format!("URL ไม่ถูกต้อง: {e}"))?;
    let mut best_bw: u64 = 0;
    let mut best_variant: Option<String> = None;
    let mut pending_bw: Option<u64> = None;
    let mut init_url: Option<String> = None;
    let mut segments = Vec::new();

    for raw_line in playlist_text.lines() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with("#EXT-X-KEY:") {
            let upper = line.to_ascii_uppercase();
            if !upper.contains("METHOD=NONE") {
                return Ok(M3u8Kind::EncryptedError(
                    "สตรีมนี้มีการเข้ารหัส DRM / AES-128 (ไม่รองรับการดาวน์โหลดตรง)".into(),
                ));
            }
        } else if line.starts_with("#EXT-X-MAP:") {
            if let Some(uri_pos) = line.find("URI=\"") {
                let tail = &line[uri_pos + 5..];
                if let Some(end) = tail.find('"') {
                    if let Ok(resolved) = base.join(&tail[..end]) {
                        init_url = Some(resolved.to_string());
                    }
                }
            }
        } else if line.starts_with("#EXT-X-STREAM-INF:") {
            let bw = line
                .split(',')
                .find_map(|part| {
                    let p = part.trim();
                    p.strip_prefix("BANDWIDTH=")
                        .and_then(|v| v.trim().parse::<u64>().ok())
                })
                .unwrap_or(1);
            pending_bw = Some(bw);
        } else if !line.starts_with('#') {
            let resolved = base
                .join(line)
                .map(|u| u.to_string())
                .map_err(|e| format!("ไม่สามารถแปลงลิงก์ segment ได้: {e}"))?;
            if let Some(bw) = pending_bw.take() {
                if bw >= best_bw || best_variant.is_none() {
                    best_bw = bw;
                    best_variant = Some(resolved);
                }
            } else {
                segments.push(resolved);
            }
        }
    }

    if let Some(best) = best_variant {
        return Ok(M3u8Kind::Master { best_variant_url: best });
    }
    if segments.is_empty() {
        return Err("ไม่พบข้อมูลวิดีโอ Segment ในเพลย์ลิสต์ .m3u8".into());
    }
    Ok(M3u8Kind::Media { init_url, segments })
}

fn is_hls_target(url: &str, content_type: &str) -> bool {
    let lower_url = url.to_ascii_lowercase();
    let path_part = lower_url.split('?').next().unwrap_or("");
    path_part.ends_with(".m3u8")
        || lower_url.contains(".m3u8?")
        || content_type.contains("mpegurl")
}

fn is_dash_target(url: &str, content_type: &str) -> bool {
    let lower_url = url.to_ascii_lowercase();
    let path_part = lower_url.split('?').next().unwrap_or("");
    path_part.ends_with(".mpd")
        || lower_url.contains(".mpd?")
        || content_type.contains("dash+xml")
}

#[derive(Debug, PartialEq)]
pub struct DashPlan {
    pub init_url: Option<String>,
    pub segments: Vec<String>,
}

/// แปลง ISO-8601 duration เช่น `PT1H2M30.5S` เป็นจำนวนวินาที
pub fn parse_iso8601_duration(raw: &str) -> Option<f64> {
    let s = raw.trim();
    if !s.starts_with('P') {
        return None;
    }
    let body = s.split('T').nth(1).unwrap_or("");
    let mut secs = 0.0f64;
    let mut num_buf = String::new();
    for ch in body.chars() {
        if ch.is_ascii_digit() || ch == '.' {
            num_buf.push(ch);
        } else {
            let val: f64 = num_buf.parse().ok()?;
            num_buf.clear();
            match ch {
                'H' | 'h' => secs += val * 3600.0,
                'M' | 'm' => secs += val * 60.0,
                'S' | 's' => secs += val,
                _ => {}
            }
        }
    }
    if secs > 0.0 { Some(secs) } else { None }
}

fn format_dash_template(tpl: &str, rep_id: &str, number: u64, time_val: u64) -> String {
    let mut out = tpl
        .replace("$RepresentationID$", rep_id)
        .replace("$Number$", &number.to_string())
        .replace("$Time$", &time_val.to_string());

    // รองรับรูปแบบ `$Number%04d$`
    while let Some(start) = out.find("$Number%0") {
        if let Some(end_rel) = out[start..].find("d$") {
            let width_str = &out[start + 9..start + end_rel];
            if let Ok(width) = width_str.parse::<usize>() {
                let formatted = format!("{number:0width$}");
                out.replace_range(start..start + end_rel + 2, &formatted);
                continue;
            }
        }
        break;
    }
    out
}

fn resolve_base_urls<'a>(node: roxmltree::Node<'a, 'a>, parent_base: &Url) -> Url {
    let mut cur = parent_base.clone();
    for child in node.children().filter(|c| c.has_tag_name("BaseURL")) {
        if let Some(txt) = child.text() {
            let trimmed = txt.trim();
            if !trimmed.is_empty() {
                if let Ok(joined) = cur.join(trimmed) {
                    cur = joined;
                }
            }
        }
    }
    cur
}

/// แปลงไฟล์ XML Manifest MPEG-DASH (.mpd) เพื่อดึง Initialization + Media Segments คุณภาพสูงสุด
pub fn parse_mpd(mpd_xml: &str, mpd_url: &str) -> Result<DashPlan, String> {
    let doc = roxmltree::Document::parse(mpd_xml)
        .map_err(|e| format!("อ่าน XML ของ .mpd ไม่สำเร็จ: {e}"))?;
    let mpd_node = doc.root_element();
    if !mpd_node.has_tag_name("MPD") {
        return Err("ไฟล์ไม่ใช่มาตรฐาน MPEG-DASH MPD".into());
    }

    let root_url = Url::parse(mpd_url).map_err(|e| format!("URL ไม่ถูกต้อง: {e}"))?;
    let mpd_base = resolve_base_urls(mpd_node, &root_url);
    let mpd_dur_secs = mpd_node
        .attribute("mediaPresentationDuration")
        .and_then(parse_iso8601_duration)
        .unwrap_or(0.0);

    let period_node = mpd_node
        .children()
        .find(|c| c.has_tag_name("Period"))
        .ok_or_else(|| "ไม่พบ <Period> ในไฟล์ .mpd".to_string())?;
    let period_base = resolve_base_urls(period_node, &mpd_base);

    // ตรวจหา ContentProtection (DRM)
    if doc.descendants().any(|n| n.has_tag_name("ContentProtection")) {
        return Err("สตรีม .mpd นี้มีการเข้ารหัส DRM (ContentProtection)".into());
    }

    // คัด AdaptationSet วิดีโอเป็นหลัก ถ้าไม่มีจึงใช้เสียง
    let adaptation_sets: Vec<_> = period_node
        .children()
        .filter(|c| c.has_tag_name("AdaptationSet"))
        .collect();
    if adaptation_sets.is_empty() {
        return Err("ไม่พบ AdaptationSet ในไฟล์ .mpd".into());
    }

    let chosen_as = adaptation_sets
        .iter()
        .copied()
        .find(|as_node| {
            as_node.attribute("mimeType").is_some_and(|m| m.starts_with("video/"))
                || as_node.attribute("contentType") == Some("video")
                || as_node
                    .children()
                    .filter(|c| c.has_tag_name("Representation"))
                    .any(|r| r.attribute("mimeType").is_some_and(|m| m.starts_with("video/")))
        })
        .unwrap_or(adaptation_sets[0]);

    let as_base = resolve_base_urls(chosen_as, &period_base);
    let as_template = chosen_as.children().find(|c| c.has_tag_name("SegmentTemplate"));
    let as_seg_list = chosen_as.children().find(|c| c.has_tag_name("SegmentList"));

    let chosen_rep = chosen_as
        .children()
        .filter(|c| c.has_tag_name("Representation"))
        .max_by_key(|r| {
            r.attribute("bandwidth")
                .and_then(|b| b.parse::<u64>().ok())
                .unwrap_or(0)
        })
        .ok_or_else(|| "ไม่พบ <Representation> ในไฟล์ .mpd".to_string())?;

    let rep_base = resolve_base_urls(chosen_rep, &as_base);
    let rep_id = chosen_rep.attribute("id").unwrap_or("0");

    // 1) กรณีใช้ <SegmentTemplate> (นิยมสูงสุดสำหรับ MPEG-DASH)
    let rep_tpl = chosen_rep.children().find(|c| c.has_tag_name("SegmentTemplate")).or(as_template);
    if let Some(tpl) = rep_tpl {
        let init_url = tpl
            .attribute("initialization")
            .or_else(|| as_template.and_then(|t| t.attribute("initialization")))
            .map(|i| format_dash_template(i, rep_id, 0, 0))
            .and_then(|u| rep_base.join(&u).ok())
            .map(|u| u.to_string());

        let media_tpl = tpl
            .attribute("media")
            .or_else(|| as_template.and_then(|t| t.attribute("media")))
            .ok_or_else(|| "ไม่พบแอตทริบิวต์ media ใน SegmentTemplate".to_string())?;

        let start_num: u64 = tpl
            .attribute("startNumber")
            .or_else(|| as_template.and_then(|t| t.attribute("startNumber")))
            .and_then(|s| s.parse().ok())
            .unwrap_or(1);

        let mut segments = Vec::new();
        if let Some(timeline) = tpl
            .children()
            .find(|c| c.has_tag_name("SegmentTimeline"))
            .or_else(|| as_template.and_then(|t| t.children().find(|c| c.has_tag_name("SegmentTimeline"))))
        {
            let mut cur_time: u64 = 0;
            let mut cur_num: u64 = start_num;
            for s_node in timeline.children().filter(|c| c.has_tag_name("S")) {
                if let Some(t) = s_node.attribute("t").and_then(|v| v.parse::<u64>().ok()) {
                    cur_time = t;
                }
                let d: u64 = s_node.attribute("d").and_then(|v| v.parse().ok()).unwrap_or(0);
                let repeat: i64 = s_node.attribute("r").and_then(|v| v.parse().ok()).unwrap_or(0);
                let count = if repeat >= 0 { (repeat + 1) as u64 } else { 1 };
                for _ in 0..count {
                    let resolved_rel = format_dash_template(media_tpl, rep_id, cur_num, cur_time);
                    if let Ok(u) = rep_base.join(&resolved_rel) {
                        segments.push(u.to_string());
                    }
                    cur_time = cur_time.saturating_add(d);
                    cur_num += 1;
                    if segments.len() >= 10_000 {
                        break;
                    }
                }
            }
        } else {
            let timescale: f64 = tpl
                .attribute("timescale")
                .or_else(|| as_template.and_then(|t| t.attribute("timescale")))
                .and_then(|v| v.parse().ok())
                .unwrap_or(1.0);
            let dur_ticks: f64 = tpl
                .attribute("duration")
                .or_else(|| as_template.and_then(|t| t.attribute("duration")))
                .and_then(|v| v.parse().ok())
                .ok_or_else(|| "ไม่พบ duration ใน SegmentTemplate".to_string())?;
            let seg_secs = (dur_ticks / timescale.max(1.0)).max(0.1);
            let total_segs = if mpd_dur_secs > 0.0 {
                (mpd_dur_secs / seg_secs).ceil().max(1.0).min(10_000.0) as u64
            } else {
                1
            };
            for idx in 0..total_segs {
                let num = start_num + idx;
                let resolved_rel = format_dash_template(media_tpl, rep_id, num, idx * (dur_ticks as u64));
                if let Ok(u) = rep_base.join(&resolved_rel) {
                    segments.push(u.to_string());
                }
            }
        }

        if segments.is_empty() {
            return Err("ไม่พบรายการ Segment ใน .mpd".into());
        }
        return Ok(DashPlan { init_url, segments });
    }

    // 2) กรณีใช้ <SegmentList>
    let rep_list = chosen_rep.children().find(|c| c.has_tag_name("SegmentList")).or(as_seg_list);
    if let Some(sl) = rep_list {
        let init_url = sl
            .children()
            .find(|c| c.has_tag_name("Initialization"))
            .and_then(|n| n.attribute("sourceURL"))
            .and_then(|u| rep_base.join(u).ok())
            .map(|u| u.to_string());
        let segments: Vec<String> = sl
            .children()
            .filter(|c| c.has_tag_name("SegmentURL"))
            .filter_map(|n| n.attribute("media"))
            .filter_map(|u| rep_base.join(u).ok())
            .map(|u| u.to_string())
            .collect();
        if segments.is_empty() {
            return Err("SegmentList ไม่มี SegmentURL".into());
        }
        return Ok(DashPlan { init_url, segments });
    }

    // 3) กรณี Single-file Representation ที่มีแค่ <BaseURL>
    if rep_base != as_base || chosen_rep.children().any(|c| c.has_tag_name("BaseURL")) {
        return Ok(DashPlan {
            init_url: None,
            segments: vec![rep_base.to_string()],
        });
    }

    Err("ไม่พบรูปแบบ Segment ที่รองรับในไฟล์ .mpd".into())
}

/// รันงานดาวน์โหลดแบบ HTTP/3 (QUIC), Multi-Segment IDM, HLS (.m3u8), MPEG-DASH (.mpd)
pub fn run_turbo_download<F>(req: TurboRequest, cancel_flag: Arc<AtomicBool>, emit: F)
where
    F: Fn(TurboEvent) + Send + Sync + 'static,
{
    let agent = build_http_agent();
    let probe = probe_target(&agent, &req.url, &req.referer, &req.cookie_header);
    let effective_url = probe.final_url.clone();
    let is_hls = is_hls_target(&effective_url, &probe.content_type);
    let is_dash = !is_hls && is_dash_target(&effective_url, &probe.content_type);

    let save_dir = req.save_dir.clone().unwrap_or_else(default_downloads_dir);
    let _ = fs::create_dir_all(&save_dir);

    let base_name = req
        .suggested_filename
        .as_deref()
        .map(sanitize_filename)
        .or_else(|| probe.cd_filename.clone())
        .or_else(|| extract_filename_from_url(&effective_url))
        .unwrap_or_else(|| "nova_download".to_string());

    let adjusted_name = if is_hls && base_name.to_ascii_lowercase().ends_with(".m3u8") {
        let stem = base_name.trim_end_matches(|c: char| !c.eq(&'.'));
        let trimmed = base_name.strip_suffix(".m3u8")
            .or_else(|| base_name.strip_suffix(".M3U8"))
            .unwrap_or(stem);
        format!("{}.ts", if trimmed.is_empty() { "stream" } else { trimmed })
    } else if is_hls && !base_name.contains('.') {
        format!("{base_name}.ts")
    } else if is_dash && base_name.to_ascii_lowercase().ends_with(".mpd") {
        let trimmed = base_name.strip_suffix(".mpd")
            .or_else(|| base_name.strip_suffix(".MPD"))
            .unwrap_or("dash_stream");
        format!("{}.mp4", if trimmed.is_empty() { "dash_stream" } else { trimmed })
    } else if is_dash && !base_name.contains('.') {
        format!("{base_name}.mp4")
    } else {
        base_name
    };

    let dest_path = unique_download_path(&save_dir.join(&adjusted_name));
    let final_name = dest_path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or(adjusted_name);
    let part_path = PathBuf::from(format!("{}.novapart", dest_path.display()));

    if is_hls {
        execute_hls_download(&agent, &req, &effective_url, &dest_path, &part_path, &final_name, &cancel_flag, &emit);
        return;
    }
    if is_dash {
        execute_dash_download(&agent, &req, &effective_url, &dest_path, &part_path, &final_name, &cancel_flag, &emit);
        return;
    }

    // ทดลองเชื่อมต่อ HTTP/3 (QUIC) ก่อนเมื่อเซิร์ฟเวอร์ประกาศ Alt-Svc: h3 หรือผู้ใช้เปิดใช้งาน HTTP/3
    if (req.prefer_http3 || probe.alt_svc_h3) && effective_url.starts_with("https://") {
        let h3_mode = "HTTP/3 QUIC".to_string();
        emit(TurboEvent::Started {
            id: req.id.clone(),
            file_name: final_name.clone(),
            url: req.url.clone(),
            path: dest_path.display().to_string(),
            total_bytes: probe.total_bytes.unwrap_or(0),
            mode: h3_mode.clone(),
        });

        let id_c = req.id.clone();
        let mode_c = h3_mode.clone();
        let emit_ref = &emit;
        let h3_res = crate::net_h3::download_via_h3(
            &effective_url,
            req.referer.as_deref(),
            req.cookie_header.as_deref(),
            &part_path,
            &cancel_flag,
            move |dl_bytes, total_len, pct, speed| {
                emit_ref(TurboEvent::Progress {
                    id: id_c.clone(),
                    downloaded_bytes: dl_bytes,
                    total_bytes: total_len,
                    pct,
                    speed_bps: speed,
                    mode: mode_c.clone(),
                });
            },
        );
        match h3_res {
            Ok(_) => {
                finalize_part_file(&req.id, &final_name, &dest_path, &part_path, Ok(()), &cancel_flag, &emit);
                return;
            }
            Err(_) if !cancel_flag.load(Ordering::Relaxed) => {
                // ถอยกลับไปใช้ HTTP/1.1 Turbo อัตโนมัติหากเครือข่ายบล็อก UDP/443
                let _ = fs::remove_file(&part_path);
            }
            Err(err) => {
                finalize_part_file(&req.id, &final_name, &dest_path, &part_path, Err(err), &cancel_flag, &emit);
                return;
            }
        }
    }

    let total_bytes = probe.total_bytes.unwrap_or(0);
    let seg_count = req.segments.clamp(1, 16);
    let use_segmented = probe.accept_ranges && total_bytes >= MIN_SEGMENTED_SIZE && seg_count > 1;
    let mode_label = if use_segmented {
        format!("{}x Turbo", seg_count)
    } else {
        "1x Stream".to_string()
    };

    emit(TurboEvent::Started {
        id: req.id.clone(),
        file_name: final_name.clone(),
        url: req.url.clone(),
        path: dest_path.display().to_string(),
        total_bytes,
        mode: mode_label.clone(),
    });

    let res = if use_segmented {
        download_segmented(
            &agent,
            &req,
            &effective_url,
            &part_path,
            total_bytes,
            seg_count,
            &mode_label,
            &cancel_flag,
            &emit,
        )
    } else {
        download_single(
            &agent,
            &req,
            &effective_url,
            &part_path,
            total_bytes,
            &mode_label,
            &cancel_flag,
            &emit,
        )
    };

    finalize_part_file(&req.id, &final_name, &dest_path, &part_path, res, &cancel_flag, &emit);
}

fn finalize_part_file<F>(
    id: &str,
    file_name: &str,
    dest_path: &Path,
    part_path: &Path,
    outcome: Result<(), String>,
    cancel_flag: &AtomicBool,
    emit: &F,
) where
    F: Fn(TurboEvent),
{
    if cancel_flag.load(Ordering::Relaxed) {
        let _ = fs::remove_file(part_path);
        emit(TurboEvent::Finished {
            id: id.to_string(),
            success: false,
            file_name: file_name.to_string(),
            path: dest_path.display().to_string(),
            message: "ยกเลิกการดาวน์โหลดแล้ว".into(),
        });
        return;
    }

    match outcome {
        Ok(()) => {
            if let Err(e) = fs::rename(part_path, dest_path) {
                let _ = fs::remove_file(part_path);
                emit(TurboEvent::Finished {
                    id: id.to_string(),
                    success: false,
                    file_name: file_name.to_string(),
                    path: dest_path.display().to_string(),
                    message: format!("บันทึกไฟล์ปลายทางไม่สำเร็จ: {e}"),
                });
            } else {
                emit(TurboEvent::Finished {
                    id: id.to_string(),
                    success: true,
                    file_name: file_name.to_string(),
                    path: dest_path.display().to_string(),
                    message: "ดาวน์โหลดสำเร็จ".into(),
                });
            }
        }
        Err(err) => {
            let _ = fs::remove_file(part_path);
            emit(TurboEvent::Finished {
                id: id.to_string(),
                success: false,
                file_name: file_name.to_string(),
                path: dest_path.display().to_string(),
                message: err,
            });
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn download_segmented<F>(
    agent: &ureq::Agent,
    req: &TurboRequest,
    url: &str,
    part_path: &Path,
    total_bytes: u64,
    segments: usize,
    mode_label: &str,
    cancel_flag: &Arc<AtomicBool>,
    emit: &F,
) -> Result<(), String>
where
    F: Fn(TurboEvent) + Send + Sync,
{
    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .read(true)
        .truncate(true)
        .open(part_path)
        .map_err(|e| format!("สร้างไฟล์ชั่วคราวไม่สำเร็จ: {e}"))?;

    file.set_len(total_bytes)
        .map_err(|e| format!("จองพื้นที่ไฟล์ไม่สำเร็จ: {e}"))?;

    let chunk_size = total_bytes / (segments as u64);
    let mut ranges = Vec::with_capacity(segments);
    for idx in 0..segments {
        let start = (idx as u64) * chunk_size;
        let end = if idx + 1 == segments {
            total_bytes - 1
        } else {
            ((idx as u64) + 1) * chunk_size - 1
        };
        if start <= end {
            ranges.push((start, end));
        }
    }

    let downloaded = Arc::new(AtomicU64::new(0));
    let done_workers = Arc::new(AtomicU64::new(0));
    let worker_total = ranges.len() as u64;
    let first_error: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));

    thread::scope(|s| {
        for (start, end) in ranges {
            let file_handle = file.try_clone().map_err(|e| format!("clone handle error: {e}"));
            let downloaded_ref = Arc::clone(&downloaded);
            let done_ref = Arc::clone(&done_workers);
            let err_ref = Arc::clone(&first_error);
            let cancel_ref = Arc::clone(cancel_flag);
            let agent_ref = agent;
            let referer_ref = &req.referer;
            let cookie_ref = &req.cookie_header;

            s.spawn(move || {
                let fh = match file_handle {
                    Ok(h) => h,
                    Err(msg) => {
                        let mut g = err_ref.lock().unwrap();
                        if g.is_none() {
                            *g = Some(msg);
                        }
                        done_ref.fetch_add(1, Ordering::Relaxed);
                        return;
                    }
                };

                if let Err(e) = fetch_range_into_file(
                    agent_ref,
                    url,
                    referer_ref,
                    cookie_ref,
                    &fh,
                    start,
                    end,
                    &downloaded_ref,
                    &cancel_ref,
                ) {
                    let mut g = err_ref.lock().unwrap();
                    if g.is_none() {
                        *g = Some(e);
                    }
                }
                done_ref.fetch_add(1, Ordering::Relaxed);
            });
        }

        // รายงานความเร็วและความคืบหน้าทุก ~220ms จนทุกเธรดเสร็จ
        let mut last_bytes = 0u64;
        let mut last_tick = Instant::now();
        while done_workers.load(Ordering::Relaxed) < worker_total {
            thread::sleep(Duration::from_millis(220));
            if cancel_flag.load(Ordering::Relaxed) {
                break;
            }
            let now = Instant::now();
            let dt = now.duration_since(last_tick).as_secs_f64().max(0.05);
            let cur = downloaded.load(Ordering::Relaxed);
            let speed = ((cur.saturating_sub(last_bytes)) as f64 / dt) as u64;
            let pct = ((cur as f64 / total_bytes as f64) * 100.0).min(100.0) as u32;
            last_bytes = cur;
            last_tick = now;

            emit(TurboEvent::Progress {
                id: req.id.clone(),
                downloaded_bytes: cur,
                total_bytes,
                pct,
                speed_bps: speed,
                mode: mode_label.to_string(),
            });
        }
    });

    if let Some(e) = first_error.lock().unwrap().take() {
        return Err(e);
    }
    file.sync_data()
        .map_err(|e| format!("flush ข้อมูลลงดิสก์ไม่สำเร็จ: {e}"))?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn fetch_range_into_file(
    agent: &ureq::Agent,
    url: &str,
    referer: &Option<String>,
    cookie_header: &Option<String>,
    file: &File,
    start: u64,
    end: u64,
    downloaded: &AtomicU64,
    cancel_flag: &AtomicBool,
) -> Result<(), String> {
    let mut offset = start;
    let mut attempts = 0;
    let mut buf = vec![0u8; 64 * 1024];

    while offset <= end {
        if cancel_flag.load(Ordering::Relaxed) {
            return Ok(());
        }

        let range_hdr = format!("bytes={offset}-{end}");
        let res = match attach_headers(agent.get(url), referer, cookie_header)
            .header("Range", &range_hdr)
            .call()
        {
            Ok(r) => r,
            Err(e) => {
                attempts += 1;
                if attempts >= MAX_RETRY_PER_SEGMENT {
                    return Err(format!("เชื่อมต่อช่วง {range_hdr} ล้มเหลว: {e}"));
                }
                thread::sleep(Duration::from_millis(400));
                continue;
            }
        };

        let status = res.status().as_u16();
        if status >= 400 {
            return Err(format!("เซิร์ฟเวอร์ตอบกลับ HTTP {status}"));
        }

        let mut reader = res.into_body().into_with_config().limit(u64::MAX).reader();
        let mut segment_failed = false;

        while offset <= end {
            if cancel_flag.load(Ordering::Relaxed) {
                return Ok(());
            }
            let remaining = (end - offset + 1) as usize;
            let to_read = remaining.min(buf.len());
            match reader.read(&mut buf[..to_read]) {
                Ok(0) => break,
                Ok(n) => {
                    write_all_at(file, &buf[..n], offset)
                        .map_err(|e| format!("เขียนไฟล์ที่ตำแหน่ง {offset} ไม่สำเร็จ: {e}"))?;
                    offset += n as u64;
                    downloaded.fetch_add(n as u64, Ordering::Relaxed);
                }
                Err(_) => {
                    segment_failed = true;
                    break;
                }
            }
        }

        if segment_failed {
            attempts += 1;
            if attempts >= MAX_RETRY_PER_SEGMENT {
                return Err("การเชื่อมต่อขาดหายระหว่างดาวน์โหลด".into());
            }
            thread::sleep(Duration::from_millis(400));
        } else if offset <= end {
            attempts += 1;
            if attempts >= MAX_RETRY_PER_SEGMENT {
                return Err("ข้อมูลจากเซิร์ฟเวอร์ไม่ครบตามช่วงที่ขอ".into());
            }
        }
    }
    Ok(())
}

fn write_all_at(file: &File, mut buf: &[u8], mut offset: u64) -> std::io::Result<()> {
    while !buf.is_empty() {
        let written = file.seek_write(buf, offset)?;
        if written == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::WriteZero,
                "written 0 bytes",
            ));
        }
        buf = &buf[written..];
        offset += written as u64;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn download_single<F>(
    agent: &ureq::Agent,
    req: &TurboRequest,
    url: &str,
    part_path: &Path,
    total_bytes: u64,
    mode_label: &str,
    cancel_flag: &AtomicBool,
    emit: &F,
) -> Result<(), String>
where
    F: Fn(TurboEvent),
{
    let res = attach_headers(agent.get(url), &req.referer, &req.cookie_header)
        .call()
        .map_err(|e| format!("เชื่อมต่อดาวน์โหลดไม่สำเร็จ: {e}"))?;

    let status = res.status().as_u16();
    if status >= 400 {
        return Err(format!("เซิร์ฟเวอร์ตอบกลับ HTTP {status}"));
    }

    let real_total = if total_bytes > 0 {
        total_bytes
    } else {
        res.headers()
            .get("content-length")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.trim().parse::<u64>().ok())
            .unwrap_or(0)
    };

    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(part_path)
        .map_err(|e| format!("สร้างไฟล์ชั่วคราวไม่สำเร็จ: {e}"))?;
    let mut writer = BufWriter::with_capacity(128 * 1024, file);
    let mut reader = res.into_body().into_with_config().limit(u64::MAX).reader();
    let mut buf = vec![0u8; 64 * 1024];

    let mut cur_bytes = 0u64;
    let mut last_report_bytes = 0u64;
    let mut last_tick = Instant::now();

    loop {
        if cancel_flag.load(Ordering::Relaxed) {
            return Ok(());
        }
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                writer
                    .write_all(&buf[..n])
                    .map_err(|e| format!("เขียนไฟล์ไม่สำเร็จ: {e}"))?;
                cur_bytes += n as u64;

                let elapsed = last_tick.elapsed();
                if elapsed >= Duration::from_millis(220) {
                    let speed = ((cur_bytes - last_report_bytes) as f64 / elapsed.as_secs_f64().max(0.05)) as u64;
                    let pct = if real_total > 0 {
                        ((cur_bytes as f64 / real_total as f64) * 100.0).min(100.0) as u32
                    } else {
                        0
                    };
                    last_report_bytes = cur_bytes;
                    last_tick = Instant::now();

                    emit(TurboEvent::Progress {
                        id: req.id.clone(),
                        downloaded_bytes: cur_bytes,
                        total_bytes: real_total,
                        pct,
                        speed_bps: speed,
                        mode: mode_label.to_string(),
                    });
                }
            }
            Err(e) => return Err(format!("อ่านข้อมูลไม่สำเร็จ: {e}")),
        }
    }

    writer.flush().map_err(|e| format!("บันทึกข้อมูลไม่สำเร็จ: {e}"))?;
    Ok(())
}

fn fetch_manifest_text(agent: &ureq::Agent, req: &TurboRequest, url: &str) -> Result<String, String> {
    let mut r = attach_headers(agent.get(url), &req.referer, &req.cookie_header)
        .call()
        .map_err(|e| format!("ดึงเพลย์ลิสต์สตรีมไม่สำเร็จ: {e}"))?;
    if r.status().as_u16() >= 400 {
        return Err(format!("เพลย์ลิสต์ตอบกลับ HTTP {}", r.status().as_u16()));
    }
    r.body_mut()
        .with_config()
        .limit(10 * 1024 * 1024)
        .read_to_string()
        .map_err(|e| format!("อ่านเพลย์ลิสต์ไม่สำเร็จ: {e}"))
}

#[allow(clippy::too_many_arguments)]
fn execute_hls_download<F>(
    agent: &ureq::Agent,
    req: &TurboRequest,
    playlist_url: &str,
    dest_path: &Path,
    part_path: &Path,
    final_name: &str,
    cancel_flag: &Arc<AtomicBool>,
    emit: &F,
) where
    F: Fn(TurboEvent) + Send + Sync,
{
    let mut current_url = playlist_url.to_string();
    let (init_url, segments) = loop {
        match fetch_manifest_text(agent, req, &current_url).and_then(|txt| parse_m3u8(&txt, &current_url)) {
            Ok(M3u8Kind::Master { best_variant_url }) => {
                if best_variant_url == current_url {
                    emit(TurboEvent::Finished {
                        id: req.id.clone(),
                        success: false,
                        file_name: final_name.to_string(),
                        path: dest_path.display().to_string(),
                        message: "Master playlist วนซ้ำไม่พบสตรีมปลายทาง".into(),
                    });
                    return;
                }
                current_url = best_variant_url;
            }
            Ok(M3u8Kind::Media { init_url, segments }) => break (init_url, segments),
            Ok(M3u8Kind::EncryptedError(msg)) | Err(msg) => {
                emit(TurboEvent::Finished {
                    id: req.id.clone(),
                    success: false,
                    file_name: final_name.to_string(),
                    path: dest_path.display().to_string(),
                    message: msg,
                });
                return;
            }
        }
    };

    let mode_label = format!("HLS ({} segs)", segments.len());
    execute_segment_pipeline(agent, req, dest_path, part_path, final_name, init_url, &segments, &mode_label, cancel_flag, emit);
}

#[allow(clippy::too_many_arguments)]
fn execute_dash_download<F>(
    agent: &ureq::Agent,
    req: &TurboRequest,
    mpd_url: &str,
    dest_path: &Path,
    part_path: &Path,
    final_name: &str,
    cancel_flag: &Arc<AtomicBool>,
    emit: &F,
) where
    F: Fn(TurboEvent) + Send + Sync,
{
    let plan = match fetch_manifest_text(agent, req, mpd_url).and_then(|xml| parse_mpd(&xml, mpd_url)) {
        Ok(p) => p,
        Err(msg) => {
            emit(TurboEvent::Finished {
                id: req.id.clone(),
                success: false,
                file_name: final_name.to_string(),
                path: dest_path.display().to_string(),
                message: msg,
            });
            return;
        }
    };

    let mode_label = format!("DASH ({} segs)", plan.segments.len());
    execute_segment_pipeline(
        agent,
        req,
        dest_path,
        part_path,
        final_name,
        plan.init_url,
        &plan.segments,
        &mode_label,
        cancel_flag,
        emit,
    );
}

/// ดาวน์โหลดและรวมชิ้นส่วนสื่อ (Initialization + Segments) แบบโหลดขนานทีละ 4 ชุดเรียงลำดับถูกต้อง (ใช้ร่วมกันทั้ง HLS และ MPEG-DASH)
#[allow(clippy::too_many_arguments)]
fn execute_segment_pipeline<F>(
    agent: &ureq::Agent,
    req: &TurboRequest,
    dest_path: &Path,
    part_path: &Path,
    final_name: &str,
    init_url: Option<String>,
    segments: &[String],
    mode_label: &str,
    cancel_flag: &Arc<AtomicBool>,
    emit: &F,
) where
    F: Fn(TurboEvent) + Send + Sync,
{
    emit(TurboEvent::Started {
        id: req.id.clone(),
        file_name: final_name.to_string(),
        url: req.url.clone(),
        path: dest_path.display().to_string(),
        total_bytes: 0,
        mode: mode_label.to_string(),
    });

    let file = match OpenOptions::new().create(true).write(true).truncate(true).open(part_path) {
        Ok(f) => f,
        Err(e) => {
            emit(TurboEvent::Finished {
                id: req.id.clone(),
                success: false,
                file_name: final_name.to_string(),
                path: dest_path.display().to_string(),
                message: format!("สร้างไฟล์ไม่สำเร็จ: {e}"),
            });
            return;
        }
    };
    let mut writer = BufWriter::with_capacity(256 * 1024, file);

    let fetch_bytes = |seg_url: &str| -> Result<Vec<u8>, String> {
        let mut r = attach_headers(agent.get(seg_url), &req.referer, &req.cookie_header)
            .call()
            .map_err(|e| format!("โหลด segment ไม่สำเร็จ: {e}"))?;
        if r.status().as_u16() >= 400 {
            return Err(format!("Segment HTTP {}", r.status().as_u16()));
        }
        r.body_mut()
            .with_config()
            .limit(128 * 1024 * 1024)
            .read_to_vec()
            .map_err(|e| format!("อ่าน segment ไม่สำเร็จ: {e}"))
    };

    let mut total_downloaded = 0u64;
    if let Some(init) = init_url {
        match fetch_bytes(&init) {
            Ok(bytes) => {
                let _ = writer.write_all(&bytes);
                total_downloaded += bytes.len() as u64;
            }
            Err(e) => {
                let _ = fs::remove_file(part_path);
                emit(TurboEvent::Finished {
                    id: req.id.clone(),
                    success: false,
                    file_name: final_name.to_string(),
                    path: dest_path.display().to_string(),
                    message: e,
                });
                return;
            }
        }
    }

    let total_segs = segments.len();
    let mut completed_segs = 0usize;
    let mut outcome: Result<(), String> = Ok(());
    let start_clock = Instant::now();

    for batch in segments.chunks(4) {
        if cancel_flag.load(Ordering::Relaxed) {
            break;
        }
        let batch_results: Vec<Result<Vec<u8>, String>> = thread::scope(|s| {
            let handles: Vec<_> = batch
                .iter()
                .map(|seg_url| s.spawn(move || fetch_bytes(seg_url)))
                .collect();
            handles
                .into_iter()
                .map(|h| h.join().unwrap_or_else(|_| Err("Stream worker panicked".into())))
                .collect()
        });

        for item in batch_results {
            match item {
                Ok(bytes) => {
                    if let Err(e) = writer.write_all(&bytes) {
                        outcome = Err(format!("เขียน segment ลงดิสก์ล้มเหลว: {e}"));
                        break;
                    }
                    total_downloaded += bytes.len() as u64;
                    completed_segs += 1;
                }
                Err(e) => {
                    outcome = Err(e);
                    break;
                }
            }
        }
        if outcome.is_err() {
            break;
        }

        let elapsed = start_clock.elapsed().as_secs_f64().max(0.1);
        let speed = (total_downloaded as f64 / elapsed) as u64;
        let pct = ((completed_segs as f64 / total_segs as f64) * 100.0).min(100.0) as u32;
        emit(TurboEvent::Progress {
            id: req.id.clone(),
            downloaded_bytes: total_downloaded,
            total_bytes: 0,
            pct,
            speed_bps: speed,
            mode: mode_label.to_string(),
        });
    }

    if outcome.is_ok() {
        outcome = writer.flush().map_err(|e| format!("flush ไฟล์ไม่สำเร็จ: {e}"));
    }
    drop(writer);
    finalize_part_file(&req.id, final_name, dest_path, part_path, outcome, cancel_flag, emit);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    #[test]
    fn unique_path_skips_collisions() {
        let dir = std::env::temp_dir().join(format!("nova_dl_test_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let p0 = dir.join("video.mp4");
        assert_eq!(unique_download_path(&p0), p0);

        fs::write(&p0, b"a").unwrap();
        let p1 = unique_download_path(&p0);
        assert_eq!(p1, dir.join("video (1).mp4"));

        fs::write(&p1, b"b").unwrap();
        let p2 = unique_download_path(&p0);
        assert_eq!(p2, dir.join("video (2).mp4"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn sanitize_and_content_disposition_preserve_thai() {
        assert_eq!(sanitize_filename("รายงาน:สรุป/2026*.pdf"), "รายงาน_สรุป_2026_.pdf");
        let h1 = "attachment; filename*=UTF-8''%E0%B9%84%E0%B8%9F%E0%B8%A5%E0%B9%8C.zip";
        assert_eq!(parse_content_disposition_filename(h1).as_deref(), Some("ไฟล์.zip"));
        let h2 = "attachment; filename=\"setup-v6.exe\"";
        assert_eq!(parse_content_disposition_filename(h2).as_deref(), Some("setup-v6.exe"));
        assert_eq!(parse_content_range_total("bytes 0-0/1048576"), Some(1_048_576));
    }

    #[test]
    fn m3u8_master_and_media_parsing() {
        let master = "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=800000\nlow/index.m3u8\n#EXT-X-STREAM-INF:BANDWIDTH=2500000\nhigh/index.m3u8\n";
        let res = parse_m3u8(master, "https://cdn.example.com/live/master.m3u8").unwrap();
        assert_eq!(
            res,
            M3u8Kind::Master {
                best_variant_url: "https://cdn.example.com/live/high/index.m3u8".into()
            }
        );

        let media = "#EXTM3U\n#EXT-X-MAP:URI=\"init.mp4\"\n#EXTINF:4.0,\nseg0.m4s\n#EXTINF:4.0,\nseg1.m4s\n";
        let mres = parse_m3u8(media, "https://cdn.example.com/live/high/index.m3u8").unwrap();
        assert_eq!(
            mres,
            M3u8Kind::Media {
                init_url: Some("https://cdn.example.com/live/high/init.mp4".into()),
                segments: vec![
                    "https://cdn.example.com/live/high/seg0.m4s".into(),
                    "https://cdn.example.com/live/high/seg1.m4s".into(),
                ]
            }
        );
    }

    #[test]
    fn segmented_download_reconstructs_exact_bytes_over_local_http() {
        // สร้างไฟล์จำลอง 512 KB (> MIN_SEGMENTED_SIZE) เพื่อทดสอบแบ่ง 4 เธรดจริงผ่าน HTTP Range
        let payload: Arc<Vec<u8>> = Arc::new((0..(512 * 1024)).map(|i| (i % 251) as u8).collect());
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server_payload = Arc::clone(&payload);

        let stop = Arc::new(AtomicBool::new(false));
        let stop_srv = Arc::clone(&stop);
        let srv = thread::spawn(move || {
            listener.set_nonblocking(true).unwrap();
            while !stop_srv.load(Ordering::Relaxed) {
                if let Ok((mut sock, _)) = listener.accept() {
                    let data = Arc::clone(&server_payload);
                    thread::spawn(move || {
                        let _ = sock.set_read_timeout(Some(Duration::from_secs(2)));
                        let mut buf = [0u8; 4096];
                        let n = sock.read(&mut buf).unwrap_or(0);
                        let req_str = String::from_utf8_lossy(&buf[..n]);
                        let is_head = req_str.starts_with("HEAD ");
                        let range_hdr = req_str
                            .lines()
                            .find(|l| l.to_ascii_lowercase().starts_with("range:"))
                            .and_then(|l| l.split_once(':'))
                            .map(|(_, v)| v.trim().to_string());

                        if is_head {
                            let hdr = format!(
                                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nAccept-Ranges: bytes\r\nConnection: close\r\n\r\n",
                                data.len()
                            );
                            let _ = sock.write_all(hdr.as_bytes());
                        } else if let Some(rh) = range_hdr {
                            let spec = rh.strip_prefix("bytes=").unwrap();
                            let (s_str, e_str) = spec.split_once('-').unwrap();
                            let start: usize = s_str.parse().unwrap();
                            let end: usize = e_str.parse().unwrap();
                            let slice = &data[start..=end];
                            let hdr = format!(
                                "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {start}-{end}/{}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                                data.len(),
                                slice.len()
                            );
                            let _ = sock.write_all(hdr.as_bytes());
                            let _ = sock.write_all(slice);
                        } else {
                            let hdr = format!(
                                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                                data.len()
                            );
                            let _ = sock.write_all(hdr.as_bytes());
                            let _ = sock.write_all(&data);
                        }
                    });
                } else {
                    thread::sleep(Duration::from_millis(10));
                }
            }
        });

        let out_dir = std::env::temp_dir().join(format!("nova_seg_dl_{}", std::process::id()));
        let _ = fs::remove_dir_all(&out_dir);
        fs::create_dir_all(&out_dir).unwrap();

        let events = Arc::new(Mutex::new(Vec::<TurboEvent>::new()));
        let events_cb = Arc::clone(&events);
        let cancel = Arc::new(AtomicBool::new(false));

        run_turbo_download(
            TurboRequest {
                id: "t1".into(),
                url: format!("http://{addr}/bundle.bin"),
                suggested_filename: Some("bundle.bin".into()),
                referer: None,
                cookie_header: None,
                segments: 4,
                prefer_http3: false,
                save_dir: Some(out_dir.clone()),
            },
            cancel,
            move |ev| {
                events_cb.lock().unwrap().push(ev);
            },
        );

        stop.store(true, Ordering::Relaxed);
        let _ = srv.join();

        let downloaded = fs::read(out_dir.join("bundle.bin")).unwrap();
        assert_eq!(downloaded.len(), payload.len());
        assert_eq!(downloaded, *payload);

        let evs = events.lock().unwrap();
        assert!(evs.iter().any(|e| matches!(e, TurboEvent::Started { mode, .. } if mode == "4x Turbo")));
        assert!(evs.iter().any(|e| matches!(e, TurboEvent::Finished { success: true, .. })));

        let _ = fs::remove_dir_all(&out_dir);
    }

    #[test]
    fn mpd_dash_parses_highest_bandwidth_template_and_timeline() {
        assert_eq!(parse_iso8601_duration("PT1M30.5S"), Some(90.5));
        let mpd = r#"<?xml version="1.0"?>
        <MPD xmlns="urn:mpeg:dash:schema:mpd:2011" mediaPresentationDuration="PT12S">
          <Period>
            <AdaptationSet mimeType="video/mp4">
              <Representation id="360p" bandwidth="500000">
                <SegmentTemplate initialization="$RepresentationID$/init.mp4" media="$RepresentationID$/seg-$Number%04d$.m4s" startNumber="1" duration="4" timescale="1"/>
              </Representation>
              <Representation id="1080p" bandwidth="4500000">
                <SegmentTemplate initialization="$RepresentationID$/init.mp4" media="$RepresentationID$/seg-$Number%04d$.m4s" startNumber="1">
                  <SegmentTimeline>
                    <S t="0" d="4" r="1"/>
                    <S d="4"/>
                  </SegmentTimeline>
                </SegmentTemplate>
              </Representation>
            </AdaptationSet>
          </Period>
        </MPD>"#;
        let plan = parse_mpd(mpd, "https://cdn.example.com/dash/stream.mpd").unwrap();
        assert_eq!(plan.init_url.as_deref(), Some("https://cdn.example.com/dash/1080p/init.mp4"));
        assert_eq!(
            plan.segments,
            vec![
                "https://cdn.example.com/dash/1080p/seg-0001.m4s",
                "https://cdn.example.com/dash/1080p/seg-0002.m4s",
                "https://cdn.example.com/dash/1080p/seg-0003.m4s",
            ]
        );
    }
}
