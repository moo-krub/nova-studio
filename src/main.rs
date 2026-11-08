use mimalloc::MiMalloc;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

use std::{
    collections::HashMap,
    fs,
    io::{Read, Write},
    net::TcpListener,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Condvar, Mutex,
    },
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use serde::{Deserialize, Serialize};
use tao::{
    dpi::{LogicalPosition, LogicalSize, PhysicalPosition},
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder},
    window::{Fullscreen, Icon, WindowBuilder},
};
use wry::{Rect, WebView, WebViewBuilder};

#[cfg(windows)]
use wry::{WebViewBuilderExtWindows, WebViewExtWindows};
#[cfg(windows)]
use std::os::windows::process::CommandExt;

const UI_HEIGHT_NORMAL: f64 = 82.0;
const UI_HEIGHT_ZEN: f64 = 42.0;
const PANEL_WIDTH: f64 = 385.0;
const DATA_FILE: &str = "nova_browser_data.json";

const UI_HTML: &str = include_str!("ui.html");
const SIDE_PANEL_HTML: &str = include_str!("panel.html");
const INJECTED_ENGINE_JS: &str = include_str!("injected.js");
const NEW_TAB_HTML: &str = include_str!("new_tab.html");

// =========================================================================
// 1. โครงสร้างข้อมูล Persistent Storage (Bookmarks, History, Downloads, Session)
// =========================================================================
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct BookmarkItem {
    title: String,
    url: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct HistoryItem {
    title: String,
    url: String,
    timestamp: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct DownloadItem {
    file_name: String,
    url: String,
    path: String,
    status: String, // "downloading" | "completed" | "failed"
    timestamp: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct SavedTab {
    title: String,
    url: String,
    workspace: String,
    pinned: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
struct PersistentData {
    bookmarks: Vec<BookmarkItem>,
    history: Vec<HistoryItem>,
    downloads: Vec<DownloadItem>,
    saved_tabs: Vec<SavedTab>,
    closed_tabs: Vec<SavedTab>,
    active_workspace: String,
    active_tab_index: usize,
    adblock_enabled: bool,
    blocked_count: usize,
    theme: String,
    notes: String,
    #[serde(default)]
    bookmarks_bar: bool,
    #[serde(default = "default_tab_layout")]
    tab_layout: String,
}

fn default_tab_layout() -> String {
    "horizontal".to_string()
}

impl Default for PersistentData {
    fn default() -> Self {
        Self {
            bookmarks: vec![
                BookmarkItem { title: "Google".into(), url: "https://www.google.com".into() },
                BookmarkItem { title: "YouTube".into(), url: "https://www.youtube.com".into() },
                BookmarkItem { title: "GitHub".into(), url: "https://github.com".into() },
                BookmarkItem { title: "ChatGPT".into(), url: "https://chatgpt.com".into() },
                BookmarkItem { title: "Rust".into(), url: "https://www.rust-lang.org".into() },
            ],
            history: Vec::new(),
            downloads: Vec::new(),
            saved_tabs: Vec::new(),
            closed_tabs: Vec::new(),
            active_workspace: "main".to_string(),
            active_tab_index: 0,
            adblock_enabled: true,
            blocked_count: 0,
            theme: "aurora".to_string(),
            notes: "📝 สมุดโน้ตด่วน Nova...\n- ปิดโปรแกรมเปิดใหม่ แท็บเดิมกลับมาครบ!\n- คลิกขวาที่ Bookmark เพื่อลบ".to_string(),
            bookmarks_bar: false,
            tab_layout: "horizontal".to_string(),
        }
    }
}


// =========================================================================
// 0. Asynchronous Debounced Storage Engine (Zero UI Stutter)
// =========================================================================
enum StorageMsg {
    SaveDebounced(PersistentData),
    Flush(PersistentData),
}

static STORAGE_TX: std::sync::OnceLock<std::sync::mpsc::Sender<StorageMsg>> = std::sync::OnceLock::new();

fn queue_save_debounced(data: PersistentData) {
    if let Some(tx) = STORAGE_TX.get() {
        let _ = tx.send(StorageMsg::SaveDebounced(data));
    } else {
        data.save_direct();
    }
}

fn flush_save_sync(data: PersistentData) {
    if let Some(tx) = STORAGE_TX.get() {
        let _ = tx.send(StorageMsg::Flush(data));
    } else {
        data.save_direct();
    }
}

fn init_storage_worker() {
    let (tx, rx) = std::sync::mpsc::channel::<StorageMsg>();
    let _ = STORAGE_TX.set(tx);

    thread::spawn(move || {
        let mut pending: Option<PersistentData> = None;
        let mut last_action = std::time::Instant::now();
        let debounce = Duration::from_millis(350);

        loop {
            let timeout = if pending.is_some() {
                let elapsed = last_action.elapsed();
                if elapsed >= debounce {
                    Duration::from_millis(0)
                } else {
                    debounce - elapsed
                }
            } else {
                Duration::from_secs(3600)
            };

            match rx.recv_timeout(timeout) {
                Ok(StorageMsg::SaveDebounced(data)) => {
                    pending = Some(data);
                    last_action = std::time::Instant::now();
                }
                Ok(StorageMsg::Flush(data)) => {
                    data.save_direct();
                    pending = None;
                    last_action = std::time::Instant::now();
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    if let Some(data) = pending.take() {
                        data.save_direct();
                        last_action = std::time::Instant::now();
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    if let Some(data) = pending.take() {
                        data.save_direct();
                    }
                    break;
                }
            }
        }
    });
}

impl PersistentData {
    fn load() -> Self {
        if let Ok(content) = fs::read_to_string(DATA_FILE) {
            if let Ok(data) = serde_json::from_str::<PersistentData>(&content) {
                return data;
            }
        }
        Self::default()
    }
    fn save_direct(&self) {
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let tmp_path = format!("{}.tmp", DATA_FILE);
            if fs::write(&tmp_path, &json).is_ok() {
                let _ = fs::rename(&tmp_path, DATA_FILE);
            }
        }
    }
    fn save(&self) {
        queue_save_debounced(self.clone());
    }
}

struct SharedAiBridge {
    latest_page_report: String,
    active_url: String,
    active_title: String,
    extract_seq: u64,
}

// =========================================================================
// 2. คำสั่งควบคุมทั้งหมดของเบราว์เซอร์ (v5.1 Complete Edition)
// =========================================================================
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "cmd")]
enum UiCommand {
    RestoreSession,
    NewTab { url: Option<String>, incognito: Option<bool>, workspace: Option<String>, pinned: Option<bool> },
    ReopenClosedTab,
    SwitchTab { id: usize },
    CloseTab { id: usize },
    TogglePinTab { id: usize },
    ToggleTabMute { id: usize },
    HibernateTab { id: usize },
    SwitchWorkspace { workspace: String },
    Navigate { url: String },
    Back,
    Forward,
    Reload,
    ReloadTab { id: usize },
    Zoom { factor: f64 },
    ResetZoom,
    PrintPage,
    ToggleFullscreen,
    ToggleDevTools,
    OpenCommandPalette,
    SetUiModalOpen { open: bool },
    StartInPlaceUpdate { download_url: String, version: String },
    RelaunchAfterUpdate,
    InternalUpdateProgress { pct: u32, step: String, log: String },
    InternalUpdateComplete { success: bool, message: String },
    CycleTilingMode,
    MoveTabById { id: usize, before_id: Option<usize> },
    ToggleBookmarksBar,
    ToggleSidePanel,
    ToggleZenMode,
    ToggleAlwaysOnTop,
    ToggleShadowMode { enable: bool },
    ToggleAdBlock,
    SetAsDefaultBrowser,
    ToggleVerticalTabs,
    SnoozeInactiveTabs,
    // --- เครื่องมือหน้าเว็บ & จัดการไฟล์/ประวัติ/ดาวน์โหลด ---
    ToggleRgxGpuShader,
    TogglePageDarkMode,
    ToggleReaderMode,
    ToggleVoiceRead,
    TranslatePageToThai,
    BoostAudio { gain: f64 },
    ScrapePageData,
    SavePageOffline,
    FindInPage { query: String },
    ClearSiteData,
    TriggerPiP,
    SetVideoSpeed { rate: f64 },
    SetTheme { theme: String },
    SaveNotes { notes: String },
    AddBookmark,
    RemoveBookmark { index: usize },
    ClearHistory,
    DeleteHistoryItem { index: usize },
    OpenDownloadedFile { path: String },
    OpenDownloadsFolder,
    ClearDownloads,
    // --- คำสั่ง AI Agent ---
    EvalJs { script: String },
    AiHighlightElements { show: bool },
    AiClick { target: String },
    AiType { target: String, text: String },
    AiAutopilotGoal { goal: String },
    RequestPageExtract,
    // --- คำสั่งซิงค์ภายใน ---
    InternalTitleChanged { id: usize, title: String },
    InternalUrlChanged { id: usize, url: String, secret: Option<String> },
    InternalPageExtracted { report: String, secret: Option<String> },
    InternalAdBlocked { count: Option<usize>, secret: Option<String> },
    InternalSaveOfflineHtml { title: String, html: String, secret: Option<String> },
    InternalLoadState { id: usize, loading: bool },
    InternalDownloadStarted { url: String, path: String },
    InternalDownloadCompleted { url: String, path: String, success: bool },
}

struct Tab {
    id: usize,
    webview: WebView,
    title: String,
    url: String,
    workspace: String,
    pinned: bool,
    hibernated: bool,
    incognito: bool,
    zoom_level: f64,
    muted: bool,
    loading: bool,
}

// Allowlist IPC ของเชลล์ (ui/panel): รับเฉพาะคำสั่งผู้ใช้ — คำสั่ง Internal* สงวนให้แท็บเนื้อหา
// ซึ่งมีระบบตรวจ secret ของตัวเองแล้ว ป้องกันเว็บที่ถูกปะทะสั่งจัดการไฟล์/สถานะผ่านเชลล์

// =========================================================================
// Windows Desktop Integration: Set as Default Browser
// =========================================================================
fn register_default_browser() -> Result<(), String> {
    let exe_path = std::env::current_exe().map_err(|e| e.to_string())?;
    let exe_str = exe_path.to_string_lossy().to_string();

    let commands = [
        format!(r#"reg add "HKCU\Software\Classes\NovaStudio.HTML" /ve /d "Nova Studio HTML Document" /f"#),
        format!(r#"reg add "HKCU\Software\Classes\NovaStudio.HTML\DefaultIcon" /ve /d "\"{}\",0" /f"#, exe_str),
        format!(r#"reg add "HKCU\Software\Classes\NovaStudio.HTML\shell\open\command" /ve /d "\"\"{}\"\" \"%1\"" /f"#, exe_str),
        format!(r#"reg add "HKCU\Software\Clients\StartMenuInternet\NovaStudio" /ve /d "Nova Studio" /f"#),
        format!(r#"reg add "HKCU\Software\Clients\StartMenuInternet\NovaStudio\DefaultIcon" /ve /d "\"{}\",0" /f"#, exe_str),
        format!(r#"reg add "HKCU\Software\Clients\StartMenuInternet\NovaStudio\shell\open\command" /ve /d "\"\"{}\"\"" /f"#, exe_str),
        format!(r#"reg add "HKCU\Software\Clients\StartMenuInternet\NovaStudio\Capabilities" /v "ApplicationName" /d "Nova Studio" /f"#),
        format!(r#"reg add "HKCU\Software\Clients\StartMenuInternet\NovaStudio\Capabilities" /v "ApplicationDescription" /d "Nova Studio — 2027 Spatial Web Browser" /f"#),
        format!(r#"reg add "HKCU\Software\Clients\StartMenuInternet\NovaStudio\Capabilities\URLAssociations" /v "http" /d "NovaStudio.HTML" /f"#),
        format!(r#"reg add "HKCU\Software\Clients\StartMenuInternet\NovaStudio\Capabilities\URLAssociations" /v "https" /d "NovaStudio.HTML" /f"#),
        format!(r#"reg add "HKCU\Software\RegisteredApplications" /v "NovaStudio" /d "Software\Clients\StartMenuInternet\NovaStudio\Capabilities" /f"#),
    ];

    for cmd in commands {
        #[cfg(windows)]
        let _ = std::process::Command::new("cmd")
            .args(["/C", &cmd])
            .creation_flags(0x08000000)
            .status();
    }

    let _ = std::process::Command::new("explorer")
        .arg("ms-settings:defaultapps")
        .spawn();

    Ok(())
}

fn load_adblock_rules(builtins: &[&str]) -> Vec<String> {
    let mut list: Vec<String> = builtins.iter().map(|s| s.to_string()).collect();
    let rules_file = "adblock_rules.txt";
    if std::path::Path::new(rules_file).exists() {
        if let Ok(content) = std::fs::read_to_string(rules_file) {
            for line in content.lines() {
                let trimmed = line.trim();
                if !trimmed.is_empty() && !trimmed.starts_with('#') && !trimmed.starts_with('!') {
                    let clean = trimmed.trim_start_matches("||").trim_end_matches('^');
                    if !clean.is_empty() {
                        list.push(clean.to_lowercase());
                    }
                }
            }
        }
    } else {
        let default_content = "# =========================================================================\n# Nova Studio — Custom EasyList / Adblock Domain Rules\n# Add custom ad/tracker domains or paste EasyList rules here\n# Lines starting with # or ! are comments\n# =========================================================================\ntracker.example.com\nanalytics.example.com\n";
        let _ = std::fs::write(rules_file, default_content);
    }
    list
}

fn shell_ipc_allowed(body: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v.get("cmd").and_then(|c| c.as_str()).map(str::to_string))
        .is_some_and(|c| !c.starts_with("Internal"))
}

#[cfg(windows)]
fn hook_allow_all_certificates(webview: &wry::WebView) {
    use webview2_com::{
        Microsoft::Web::WebView2::Win32::*,
        ServerCertificateErrorDetectedEventHandler,
    };
    use windows_core::Interface;

    if let Ok(wv14) = webview.webview().cast::<ICoreWebView2_14>() {
        let mut token = 0i64;
        let _ = unsafe {
            wv14.add_ServerCertificateErrorDetected(
                &ServerCertificateErrorDetectedEventHandler::create(Box::new(|_, args| {
                    if let Some(args) = args {
                        let _ = args.SetAction(COREWEBVIEW2_SERVER_CERTIFICATE_ERROR_ACTION_ALWAYS_ALLOW);
                    }
                    Ok(())
                })),
                &mut token,
            )
        };
    }
}

#[cfg(not(windows))]
fn hook_allow_all_certificates(_webview: &wry::WebView) {}

fn main() -> wry::Result<()> {
    if let Ok(current_exe) = std::env::current_exe() {
        if let Some(parent) = current_exe.parent() {
            let old_file = parent.join(format!("{}.old", current_exe.file_name().unwrap_or_default().to_string_lossy()));
            let _ = std::fs::remove_file(old_file);
            let tmp_file = parent.join("nova_update_tmp.exe");
            let _ = std::fs::remove_file(tmp_file);
        }
    }

    let extreme_gpu_flags = concat!(
        "--ignore-gpu-blocklist ",
        "--ignore-certificate-errors ",
        "--test-type ",
        "--enable-gpu-rasterization ",
        "--enable-zero-copy ",
        "--enable-oop-rasterization ",
        "--enable-quic ",
        "--enable-fast-unload ",
        "--enable-parallel-downloading ",
        "--disk-cache-size=1073741824 ",
        "--autoplay-policy=no-user-gesture-required ",
        "--enable-features=CanvasOopRasterization,UseSkiaRenderer,WebAssemblyLazyCompilation,AsyncDns,DnsOverHttps ",
        "--dns-over-https-mode=secure ",
        "--doh-templates=https://1.1.1.1/dns-query https://dns.google/dns-query ",
        "--disable-blink-features=AutomationControlled"
    );
    unsafe {
        std::env::set_var("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS", extreme_gpu_flags);
    }

    let args: Vec<String> = std::env::args().collect();
    let mut start_in_shadow = false;
    let mut initial_cli_url: Option<String> = None;
    let mut agent_port: u16 = 9222;
    let mut api_token: Option<String> = std::env::var("NOVA_TOKEN").ok();

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--shadow" | "--dark" => start_in_shadow = true,
            "--url" => { if i + 1 < args.len() { initial_cli_url = Some(args[i + 1].clone()); i += 1; } }
            "--port" => { if i + 1 < args.len() { agent_port = args[i + 1].parse().unwrap_or(9222); i += 1; } }
            "--token" => { if i + 1 < args.len() { api_token = Some(args[i + 1].clone()); i += 1; } }
            _ => {}
        }
        i += 1;
    }

    let ipc_secret = format!(
        "nova_sec_{}_{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos()
    );

    init_storage_worker();
    let mut storage = PersistentData::load();
    let blocked_counter = Arc::new(AtomicUsize::new(storage.blocked_count));
    let shared_adblock = Arc::new(AtomicBool::new(storage.adblock_enabled));

    let ai_bridge = Arc::new((
        Mutex::new(SharedAiBridge {
            latest_page_report: "ยังไม่มีข้อมูลหน้าเว็บ".to_string(),
            active_url: String::new(),
            active_title: String::new(),
            extract_seq: 0,
        }),
        Condvar::new(),
    ));

    let event_loop = EventLoopBuilder::<UiCommand>::with_user_event().build();
    let proxy = event_loop.create_proxy();

    let proxy_server = proxy.clone();
    let bridge_server = Arc::clone(&ai_bridge);
    let server_token = api_token.clone();
    thread::spawn(move || {
        start_ai_agent_server(agent_port, proxy_server, bridge_server, server_token);
    });

    let icon_bytes = include_bytes!("../icon_32.rgba");
    let window_icon = Icon::from_rgba(icon_bytes.to_vec(), 32, 32).ok();

    let mut win_builder = WindowBuilder::new()
        .with_title("Nova Studio — Spatial Browser 2027 🦀✨")
        .with_window_icon(window_icon)
        .with_maximized(true)
        .with_inner_size(LogicalSize::new(1460.0, 900.0));

    if start_in_shadow {
        win_builder = win_builder.with_position(PhysicalPosition::new(-3000, -3000));
    }

    let window = win_builder.build(&event_loop).unwrap();
    let win_size = window.inner_size().to_logical::<f64>(window.scale_factor());

    let proxy_ui = proxy.clone();
    let ui_webview = WebViewBuilder::new()
        .with_additional_browser_args(extreme_gpu_flags)
        .with_html(UI_HTML)
        .with_bounds(Rect {
            position: LogicalPosition::new(0.0, 0.0).into(),
            size: LogicalSize::new(win_size.width, UI_HEIGHT_NORMAL).into(),
        })
        .with_ipc_handler(move |req| {
            if shell_ipc_allowed(req.body())
                && let Ok(cmd) = serde_json::from_str::<UiCommand>(req.body())
            {
                let _ = proxy_ui.send_event(cmd);
            }
        })
        .build_as_child(&window)?;
    hook_allow_all_certificates(&ui_webview);

    let proxy_panel = proxy.clone();
    let panel_webview = WebViewBuilder::new()
        .with_additional_browser_args(extreme_gpu_flags)
        .with_html(SIDE_PANEL_HTML)
        .with_bounds(Rect {
            position: LogicalPosition::new(0.0, 0.0).into(),
            size: LogicalSize::new(0.0, 0.0).into(),
        })
        .with_ipc_handler(move |req| {
            if shell_ipc_allowed(req.body())
                && let Ok(cmd) = serde_json::from_str::<UiCommand>(req.body())
            {
                let _ = proxy_panel.send_event(cmd);
            }
        })
        .build_as_child(&window)?;
    hook_allow_all_certificates(&panel_webview);

    let mut tabs: HashMap<usize, Tab> = HashMap::new();
    let mut tab_order: Vec<usize> = Vec::new();
    let mut active_tab_id: usize = 0;
    let mut next_tab_id: usize = 1;
    let mut current_workspace = if storage.active_workspace.is_empty() {
        "main".to_string()
    } else {
        storage.active_workspace.clone()
    };

    let mut tiling_mode: u8 = 1;
    let mut side_panel_open = false;
    let mut zen_mode = false;
    let mut always_on_top = false;
    let mut is_fullscreen = false;
    let mut ui_modal_open = false;

    // กู้คืนแท็บเดิมทั้งหมดตอนเปิดโปรแกรม
    if !storage.saved_tabs.is_empty() {
        let _ = proxy.send_event(UiCommand::RestoreSession);
        if let Some(cli_u) = initial_cli_url {
            let _ = proxy.send_event(UiCommand::NewTab {
                url: Some(cli_u),
                incognito: Some(false),
                workspace: None,
                pinned: Some(false),
            });
        }
    } else {
        let _ = proxy.send_event(UiCommand::NewTab {
            url: initial_cli_url,
            incognito: Some(false),
            workspace: None,
            pinned: Some(false),
        });
    }

    let ad_domains_static = [
        "doubleclick.net", "googlesyndication.com", "googleadservices.com",
        "adservice.google.com", "adnxs.com", "taboola.com", "outbrain.com",
        "popads.net", "adsterra.com", "mc.yandex.ru", "scorecardresearch.com",
        "rubiconproject.com", "criteo.com", "pubmatic.com", "amazon-adsystem.com",
    ];
    let ad_domains = Arc::new(load_adblock_rules(&ad_domains_static));
    // UA ยุคปัจจุบัน (Chrome/141 + Edg/141) — เว็บเสิร์ฟโค้ดรุ่นใหม่ ไม่ตกเป็นเบราว์เซอร์เก่า
    let stealth_ua = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36 Edg/141.0.0.0";

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;

        match event {
            Event::WindowEvent { event: WindowEvent::Resized(_), .. } => {
                let win_size = window.inner_size().to_logical::<f64>(window.scale_factor());
                sync_all_views(
                    &ui_webview, &panel_webview, &tabs, &tab_order, active_tab_id,
                    &current_workspace, win_size.width, win_size.height,
                    tiling_mode, side_panel_open, zen_mode, ui_modal_open, &mut storage, always_on_top, &ai_bridge, &blocked_counter,
                );
            }
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => {
                save_session_snapshot(&mut storage, &tabs, &tab_order, &current_workspace, active_tab_id, &blocked_counter, true);
                flush_save_sync(storage.clone());
                storage.save_direct();
                *control_flow = ControlFlow::Exit;
            }
            Event::UserEvent(cmd) => {
                let win_size = window.inner_size().to_logical::<f64>(window.scale_factor());
                let ui_h = if zen_mode { UI_HEIGHT_ZEN } else { UI_HEIGHT_NORMAL };

                match cmd {
                    UiCommand::RestoreSession => {
                        let to_restore = storage.saved_tabs.clone();
                        println!("🔄 [Session Restore] กำลังกู้คืนแท็บเดิมจำนวน {} แท็บ...", to_restore.len());
                        for st in to_restore {
                            let url_opt = if st.url.is_empty() { None } else { Some(st.url) };
                            let ws_opt = if st.workspace.is_empty() { Some("main".to_string()) } else { Some(st.workspace) };
                            let _ = proxy.send_event(UiCommand::NewTab {
                                url: url_opt,
                                incognito: Some(false),
                                workspace: ws_opt,
                                pinned: Some(st.pinned),
                            });
                        }
                    }

                    UiCommand::NewTab { url, incognito, workspace, pinned } => {
                        let id = next_tab_id;
                        next_tab_id += 1;
                        let is_incognito = incognito.unwrap_or(false);
                        let is_pinned = pinned.unwrap_or(false);
                        let tab_ws = workspace.unwrap_or_else(|| current_workspace.clone());

                        let proxy_title = proxy.clone();
                        let proxy_ipc = proxy.clone();
                        let proxy_ad = proxy.clone();
                        let proxy_dl_start = proxy.clone();
                        let proxy_dl_done = proxy.clone();
                        let proxy_nw = proxy.clone();
                        let proxy_load = proxy.clone();
                        let adblock_flag = Arc::clone(&shared_adblock);
                        let adblock_active = shared_adblock.load(Ordering::Relaxed);
                        let counter_ref = Arc::clone(&blocked_counter);
                        let ad_domains_ref = Arc::clone(&ad_domains);
                        let tab_secret = ipc_secret.clone();

                        let init_script = format!(
                            "window.__NOVA_TAB_ID = {};\nwindow.__NOVA_ADBLOCK = {};\n{}",
                            id,
                            adblock_active,
                            INJECTED_ENGINE_JS.replace("__NOVA_SECRET_PLACEHOLDER__", &tab_secret)
                        );

                        let expected_secret = ipc_secret.clone();
                        let mut builder = WebViewBuilder::new()
                            .with_user_agent(stealth_ua)
                            .with_incognito(is_incognito)
                            .with_devtools(true)
                            .with_additional_browser_args(extreme_gpu_flags)
                            .with_initialization_script(&init_script)
                            .with_document_title_changed_handler(move |new_title| {
                                let _ = proxy_title.send_event(UiCommand::InternalTitleChanged { id, title: new_title });
                            })
                            .with_navigation_handler(move |nav_url| {
                                if adblock_flag.load(Ordering::Relaxed) {
                                    for ad in ad_domains_ref.iter() {
                                        let ad_str: &str = ad;
                                        if nav_url.contains(ad_str) {
                                            counter_ref.fetch_add(1, Ordering::Relaxed);
                                            let _ = proxy_ad.send_event(UiCommand::InternalAdBlocked { count: Some(1), secret: None });
                                            return false;
                                        }
                                    }
                                }
                                true
                            })
                            // target=_blank / window.open → เปิดเป็นแท็บใหม่ใน Nova
                            // (ไม่ปล่อยหน้าต่างกำพร้านอกการควบคุม — ส่งต่อให้ proxy เปิดแท็บแล้วปฏิเสธ popup)
                            .with_new_window_req_handler(move |nw_url: String, _features: wry::NewWindowFeatures| {
                                let lower = nw_url.to_lowercase();
                                let is_auth_or_popup = lower.contains("accounts.google.com")
                                    || lower.contains("oauth")
                                    || lower.contains("signin")
                                    || lower.contains("login")
                                    || lower.contains("auth")
                                    || lower.contains("gsi/transform")
                                    || nw_url.is_empty()
                                    || nw_url == "about:blank";
                                if is_auth_or_popup {
                                    return wry::NewWindowResponse::Allow;
                                }
                                let _ = proxy_nw.send_event(UiCommand::NewTab {
                                    url: Some(nw_url),
                                    incognito: Some(false),
                                    workspace: None,
                                    pinned: Some(false),
                                });
                                wry::NewWindowResponse::Deny
                            })
                            // สถานะโหลดของแท็บ (spinner) — เริ่ม/จบโหลดจากเอนจินจริง
                            .with_on_page_load_handler(move |evt: wry::PageLoadEvent, _url: String| {
                                let _ = proxy_load.send_event(UiCommand::InternalLoadState {
                                    id,
                                    loading: matches!(evt, wry::PageLoadEvent::Started),
                                });
                            })
                            // 📥 ระบบดักจับการดาวน์โหลดไฟล์ระดับ Native ของเบราว์เซอร์
                            .with_download_started_handler(move |dl_url: String, default_path: &mut PathBuf| {
                                let path_str = default_path.display().to_string();
                                println!("📥 [Download Started] กำลังดาวน์โหลด: {} -> {}", dl_url, path_str);
                                let _ = proxy_dl_start.send_event(UiCommand::InternalDownloadStarted {
                                    url: dl_url,
                                    path: path_str,
                                });
                                true
                            })
                            .with_download_completed_handler(move |dl_url: String, file_path: Option<PathBuf>, success: bool| {
                                let path_str = file_path.map(|p| p.display().to_string()).unwrap_or_default();
                                println!("✅ [Download Completed] สถานะ: {} -> {}", success, path_str);
                                let _ = proxy_dl_done.send_event(UiCommand::InternalDownloadCompleted {
                                    url: dl_url,
                                    path: path_str,
                                    success,
                                });
                            })
                            .with_ipc_handler(move |req| {
                                if let Ok(internal_cmd) = serde_json::from_str::<UiCommand>(req.body()) {
                                    let allowed = match &internal_cmd {
                                        UiCommand::InternalUrlChanged { secret, .. }
                                        | UiCommand::InternalPageExtracted { secret, .. }
                                        | UiCommand::InternalAdBlocked { secret, .. }
                                        | UiCommand::InternalSaveOfflineHtml { secret, .. } => {
                                            secret.as_deref() == Some(expected_secret.as_str())
                                        }
                                        UiCommand::NewTab { .. }
                                        | UiCommand::Navigate { .. }
                                        | UiCommand::PrintPage
                                        | UiCommand::ToggleFullscreen
                                        | UiCommand::ToggleSidePanel
                                        | UiCommand::ToggleReaderMode
                                        | UiCommand::ToggleVerticalTabs
                                        | UiCommand::SaveNotes { .. }
                                        | UiCommand::AddBookmark
                                        | UiCommand::RemoveBookmark { .. }
                                        | UiCommand::SetTheme { .. }
                                        | UiCommand::Reload
                                        | UiCommand::Zoom { .. }
                                        | UiCommand::ResetZoom
                                        | UiCommand::OpenCommandPalette
                                        | UiCommand::SetUiModalOpen { .. }
                                        | UiCommand::StartInPlaceUpdate { .. }
                                        | UiCommand::RelaunchAfterUpdate
                                        | UiCommand::CycleTilingMode
                                        | UiCommand::ToggleZenMode
                                        | UiCommand::SnoozeInactiveTabs
                                        | UiCommand::SetAsDefaultBrowser
                                        | UiCommand::OpenDownloadsFolder
                                        | UiCommand::ClearHistory
                                        | UiCommand::ClearDownloads => true,
                                        _ => false,
                                    };
                                    if allowed {
                                        let _ = proxy_ipc.send_event(internal_cmd);
                                    }
                                }
                            })
                            .with_bounds(Rect {
                                position: LogicalPosition::new(0.0, ui_h).into(),
                                size: LogicalSize::new(win_size.width, (win_size.height - ui_h).max(0.0)).into(),
                            });

                        let initial_url_str;
                        if let Some(u) = url {
                            builder = builder.with_url(&u);
                            initial_url_str = u;
                        } else {
                            builder = builder.with_html(NEW_TAB_HTML);
                            initial_url_str = String::new();
                        }

                        if let Ok(webview) = builder.build_as_child(&window) {
                            hook_allow_all_certificates(&webview);
                            let default_title = if is_incognito { "🕵️ แท็บไร้ตัวตน" } else { "แท็บใหม่" };
                            tabs.insert(
                                id,
                                Tab {
                                    id,
                                    webview,
                                    title: default_title.to_string(),
                                    url: initial_url_str,
                                    workspace: tab_ws.clone(),
                                    pinned: is_pinned,
                                    hibernated: false,
                                    incognito: is_incognito,
                                    zoom_level: 1.0,
                                    muted: false,
                                    loading: false,
                                },
                            );
                            tab_order.push(id);
                            if tab_ws == current_workspace {
                                active_tab_id = id;
                            }
                            sync_all_views(
                                &ui_webview, &panel_webview, &tabs, &tab_order, active_tab_id,
                                &current_workspace, win_size.width, win_size.height,
                                tiling_mode, side_panel_open, zen_mode, ui_modal_open, &mut storage, always_on_top, &ai_bridge, &blocked_counter,
                            );
                        }
                    }

                    // --- 📥 จัดการสถานะดาวน์โหลดไฟล์ ---
                    UiCommand::InternalDownloadStarted { url, path } => {
                        let file_name = PathBuf::from(&path)
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_else(|| "file_download".to_string());
                        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
                        storage.downloads.insert(0, DownloadItem {
                            file_name,
                            url,
                            path,
                            status: "downloading".to_string(),
                            timestamp: now,
                        });
                        side_panel_open = true; // เปิดแถบด้านขวาโชว์ความคืบหน้าดาวน์โหลดอัตโนมัติ
                        sync_all_views(
                            &ui_webview, &panel_webview, &tabs, &tab_order, active_tab_id,
                            &current_workspace, win_size.width, win_size.height,
                            tiling_mode, side_panel_open, zen_mode, ui_modal_open, &mut storage, always_on_top, &ai_bridge, &blocked_counter,
                        );
                    }
                    UiCommand::InternalDownloadCompleted { url, path, success } => {
                        if let Some(item) = storage.downloads.iter_mut().find(|d| d.status == "downloading" && (d.url == url || d.path == path)) {
                            item.status = if success { "completed".to_string() } else { "failed".to_string() };
                            if !path.is_empty() {
                                item.path = path.clone();
                            }
                        }
                        sync_all_views(
                            &ui_webview, &panel_webview, &tabs, &tab_order, active_tab_id,
                            &current_workspace, win_size.width, win_size.height,
                            tiling_mode, side_panel_open, zen_mode, ui_modal_open, &mut storage, always_on_top, &ai_bridge, &blocked_counter,
                        );
                        let fname = std::path::Path::new(&path)
                            .file_name()
                            .map(|s| s.to_string_lossy().to_string())
                            .unwrap_or_else(|| url.clone());
                        let toast = if success { format!("✅ ดาวน์โหลดเสร็จ: {}", fname) } else { format!("❌ ดาวน์โหลดล้มเหลว: {}", fname) };
                        let safe = toast.replace('\\', "\\\\").replace('"', "\\\"");
                        let _ = ui_webview.evaluate_script(&format!("window.showToast && window.showToast(\"{}\");", safe));
                    }
                    UiCommand::MoveTabById { id, before_id } => {
                        // ลากแท็บ: ย้าย id ไปไว้หน้า before_id (None = ท้ายสุด) ตามลำดับ tab_order
                        if let Some(from) = tab_order.iter().position(|&t| t == id) {
                            let moved = tab_order[from];
                            tab_order.remove(from);
                            let to = before_id
                                .and_then(|bid| tab_order.iter().position(|&t| t == bid))
                                .unwrap_or(tab_order.len());
                            tab_order.insert(to, moved);
                            sync_all_views(
                                &ui_webview, &panel_webview, &tabs, &tab_order, active_tab_id,
                                &current_workspace, win_size.width, win_size.height,
                                tiling_mode, side_panel_open, zen_mode, ui_modal_open, &mut storage, always_on_top, &ai_bridge, &blocked_counter,
                            );
                        }
                    }
                    UiCommand::ToggleBookmarksBar => {
                        storage.bookmarks_bar = !storage.bookmarks_bar;
                        storage.save();
                        sync_all_views(
                            &ui_webview, &panel_webview, &tabs, &tab_order, active_tab_id,
                            &current_workspace, win_size.width, win_size.height,
                            tiling_mode, side_panel_open, zen_mode, ui_modal_open, &mut storage, always_on_top, &ai_bridge, &blocked_counter,
                        );
                    }
                    UiCommand::InternalLoadState { id, loading } => {
                        if let Some(t) = tabs.get_mut(&id) {
                            t.loading = loading;
                        }
                        sync_all_views(
                            &ui_webview, &panel_webview, &tabs, &tab_order, active_tab_id,
                            &current_workspace, win_size.width, win_size.height,
                            tiling_mode, side_panel_open, zen_mode, ui_modal_open, &mut storage, always_on_top, &ai_bridge, &blocked_counter,
                        );
                    }
                    UiCommand::OpenDownloadedFile { path } => {
                        if !path.is_empty() {
                            #[cfg(target_os = "windows")]
                            let _ = std::process::Command::new("explorer").arg(format!("/select,{}", path)).spawn();
                            #[cfg(target_os = "macos")]
                            let _ = std::process::Command::new("open").arg("-R").arg(&path).spawn();
                            #[cfg(target_os = "linux")]
                            let _ = std::process::Command::new("xdg-open").arg(&path).spawn();
                        }
                    }
                    UiCommand::OpenDownloadsFolder => {
                        if let Some(first) = storage.downloads.first() {
                            if let Some(parent) = PathBuf::from(&first.path).parent() {
                                #[cfg(target_os = "windows")]
                                let _ = std::process::Command::new("explorer").arg(parent).spawn();
                                #[cfg(target_os = "macos")]
                                let _ = std::process::Command::new("open").arg(parent).spawn();
                                #[cfg(target_os = "linux")]
                                let _ = std::process::Command::new("xdg-open").arg(parent).spawn();
                                return;
                            }
                        }
                        #[cfg(target_os = "windows")]
                        let _ = std::process::Command::new("explorer").arg(".").spawn();
                    }
                    UiCommand::ClearDownloads => {
                        storage.downloads.clear();
                        sync_all_views(
                            &ui_webview, &panel_webview, &tabs, &tab_order, active_tab_id,
                            &current_workspace, win_size.width, win_size.height,
                            tiling_mode, side_panel_open, zen_mode, ui_modal_open, &mut storage, always_on_top, &ai_bridge, &blocked_counter,
                        );
                    }

                    UiCommand::ReopenClosedTab => {
                        if let Some(last_closed) = storage.closed_tabs.pop() {
                            let url_opt = if last_closed.url.is_empty() { None } else { Some(last_closed.url) };
                            current_workspace = if last_closed.workspace.is_empty() { "main".to_string() } else { last_closed.workspace };
                            let _ = proxy.send_event(UiCommand::NewTab {
                                url: url_opt,
                                incognito: Some(false),
                                workspace: Some(current_workspace.clone()),
                                pinned: Some(last_closed.pinned),
                            });
                        }
                    }

                                        UiCommand::ToggleTabMute { id } => {
                        if let Some(tab) = tabs.get_mut(&id) {
                            tab.muted = !tab.muted;
                            let script = format!("window.__novaSetMute && window.__novaSetMute({});", tab.muted);
                            let _ = tab.webview.evaluate_script(&script);
                        }
                        sync_all_views(
                            &ui_webview, &panel_webview, &tabs, &tab_order, active_tab_id,
                            &current_workspace, win_size.width, win_size.height,
                            tiling_mode, side_panel_open, zen_mode, ui_modal_open, &mut storage, always_on_top, &ai_bridge, &blocked_counter,
                        );
                    }
                    UiCommand::TogglePinTab { id } => {
                        if let Some(tab) = tabs.get_mut(&id) {
                            tab.pinned = !tab.pinned;
                        }
                        sync_all_views(
                            &ui_webview, &panel_webview, &tabs, &tab_order, active_tab_id,
                            &current_workspace, win_size.width, win_size.height,
                            tiling_mode, side_panel_open, zen_mode, ui_modal_open, &mut storage, always_on_top, &ai_bridge, &blocked_counter,
                        );
                    }

                    UiCommand::SwitchWorkspace { workspace } => {
                        current_workspace = workspace;
                        if let Some(&first_id) = tab_order.iter().rev().find(|&&id| {
                            tabs.get(&id).map(|t| t.workspace == current_workspace).unwrap_or(false)
                        }) {
                            active_tab_id = first_id;
                            sync_all_views(
                                &ui_webview, &panel_webview, &tabs, &tab_order, active_tab_id,
                                &current_workspace, win_size.width, win_size.height,
                                tiling_mode, side_panel_open, zen_mode, ui_modal_open, &mut storage, always_on_top, &ai_bridge, &blocked_counter,
                            );
                        } else {
                            let _ = proxy.send_event(UiCommand::NewTab {
                                url: None,
                                incognito: Some(false),
                                workspace: Some(current_workspace.clone()),
                                pinned: Some(false),
                            });
                        }
                    }

                    UiCommand::SwitchTab { id } => {
                        if let Some(tab) = tabs.get_mut(&id) {
                            if tab.hibernated {
                                tab.hibernated = false;
                                tab.title = tab.title.replace("❄️ ", "");
                                if !tab.url.is_empty() {
                                    let _ = tab.webview.load_url(&tab.url);
                                } else {
                                    let _ = tab.webview.load_html(NEW_TAB_HTML);
                                }
                            }
                            active_tab_id = id;
                            current_workspace = tab.workspace.clone();
                            sync_all_views(
                                &ui_webview, &panel_webview, &tabs, &tab_order, active_tab_id,
                                &current_workspace, win_size.width, win_size.height,
                                tiling_mode, side_panel_open, zen_mode, ui_modal_open, &mut storage, always_on_top, &ai_bridge, &blocked_counter,
                            );
                        }
                    }

                    UiCommand::HibernateTab { id } => {
                        if let Some(tab) = tabs.get_mut(&id) {
                            if !tab.hibernated {
                                tab.hibernated = true;
                                if !tab.title.starts_with("❄️ ") {
                                    tab.title = format!("❄️ {}", tab.title);
                                }
                                let _ = tab.webview.load_html("<html><body style='background:#09090b;color:#71717a;display:flex;align-items:center;justify-content:center;height:100vh;font-family:sans-serif;'><h2>❄️ แช่แข็งแท็บคืนค่า RAM & VRAM แล้ว</h2></body></html>");
                            }
                        }
                        sync_all_views(
                            &ui_webview, &panel_webview, &tabs, &tab_order, active_tab_id,
                            &current_workspace, win_size.width, win_size.height,
                            tiling_mode, side_panel_open, zen_mode, ui_modal_open, &mut storage, always_on_top, &ai_bridge, &blocked_counter,
                        );
                    }

                    UiCommand::CloseTab { id } => {
                        if let Some(t) = tabs.get(&id) {
                            if t.pinned { return; }
                            if !t.incognito && (!t.url.is_empty() || t.title != "แท็บใหม่") {
                                storage.closed_tabs.push(SavedTab {
                                    title: t.title.clone(),
                                    url: t.url.clone(),
                                    workspace: t.workspace.clone(),
                                    pinned: false,
                                });
                                if storage.closed_tabs.len() > 25 {
                                    storage.closed_tabs.remove(0);
                                }
                            }
                        }

                        let ws_count = tab_order.iter().filter(|&&tid| {
                            tabs.get(&tid).map(|t| t.workspace == current_workspace).unwrap_or(false)
                        }).count();

                        if ws_count <= 1 {
                            if let Some(tab) = tabs.get_mut(&id) {
                                let _ = tab.webview.load_html(NEW_TAB_HTML);
                                tab.title = "แท็บใหม่".to_string();
                                tab.url = String::new();
                                tab.hibernated = false;
                            }
                        } else if tabs.remove(&id).is_some() {
                            tab_order.retain(|&x| x != id);
                            if active_tab_id == id {
                                if let Some(&next_id) = tab_order.iter().rev().find(|&&tid| {
                                    tabs.get(&tid).map(|t| t.workspace == current_workspace).unwrap_or(false)
                                }) {
                                    active_tab_id = next_id;
                                }
                            }
                        }
                        sync_all_views(
                            &ui_webview, &panel_webview, &tabs, &tab_order, active_tab_id,
                            &current_workspace, win_size.width, win_size.height,
                            tiling_mode, side_panel_open, zen_mode, ui_modal_open, &mut storage, always_on_top, &ai_bridge, &blocked_counter,
                        );
                    }

                    UiCommand::Navigate { mut url } => {
                        let trimmed = url.trim();
                        if !trimmed.starts_with("http://") && !trimmed.starts_with("https://") {
                            // localhost / loopback เป็น URL (http เท่านั้น) ไม่ใช่คำค้นหา
                            let looks_local = trimmed == "localhost"
                                || trimmed.starts_with("localhost:")
                                || trimmed.starts_with("127.0.0.1")
                                || trimmed.starts_with("[::1]");
                            if looks_local || (trimmed.contains('.') && !trimmed.contains(' ')) {
                                let scheme = if looks_local { "http" } else { "https" };
                                url = format!("{}://{}", scheme, trimmed);
                            } else {
                                url = format!("https://www.google.com/search?q={}", trimmed.replace(' ', "+"));
                            }
                        }
                        if let Some(tab) = tabs.get_mut(&active_tab_id) {
                            tab.hibernated = false;
                            tab.url = url.clone();
                            let _ = tab.webview.load_url(&url);
                            sync_all_views(
                                &ui_webview, &panel_webview, &tabs, &tab_order, active_tab_id,
                                &current_workspace, win_size.width, win_size.height,
                                tiling_mode, side_panel_open, zen_mode, ui_modal_open, &mut storage, always_on_top, &ai_bridge, &blocked_counter,
                            );
                        }
                    }

                    UiCommand::Back => { if let Some(t) = tabs.get(&active_tab_id) { let _ = t.webview.evaluate_script("history.back()"); } }
                    UiCommand::Forward => { if let Some(t) = tabs.get(&active_tab_id) { let _ = t.webview.evaluate_script("history.forward()"); } }
                    UiCommand::Reload => { if let Some(t) = tabs.get(&active_tab_id) { let _ = t.webview.evaluate_script("location.reload()"); } }
                    UiCommand::ReloadTab { id } => { if let Some(t) = tabs.get(&id) { let _ = t.webview.evaluate_script("location.reload()"); } }
                    UiCommand::Zoom { factor } => {
                        if let Some(t) = tabs.get_mut(&active_tab_id) {
                            t.zoom_level = (t.zoom_level + factor).clamp(0.5, 2.5);
                            let _ = t.webview.zoom(t.zoom_level);
                        }
                    }
                    UiCommand::ResetZoom => {
                        if let Some(t) = tabs.get_mut(&active_tab_id) {
                            t.zoom_level = 1.0;
                            let _ = t.webview.zoom(1.0);
                        }
                    }
                    UiCommand::PrintPage => {
                        if let Some(t) = tabs.get(&active_tab_id) {
                            let _ = t.webview.evaluate_script("window.print();");
                        }
                    }
                    UiCommand::ToggleFullscreen => {
                        is_fullscreen = !is_fullscreen;
                        if is_fullscreen {
                            window.set_fullscreen(Some(Fullscreen::Borderless(None)));
                        } else {
                            window.set_fullscreen(None);
                        }
                    }
                    UiCommand::ToggleDevTools => {
                        if let Some(t) = tabs.get(&active_tab_id) {
                            t.webview.open_devtools();
                        }
                    }
                    UiCommand::OpenCommandPalette => {
                        let _ = ui_webview.evaluate_script("if (typeof openCommandPalette === 'function') openCommandPalette();");
                    }
                    UiCommand::SetUiModalOpen { open } => {
                        ui_modal_open = open;
                        let target_h = if ui_modal_open {
                            win_size.height
                        } else if zen_mode {
                            UI_HEIGHT_ZEN
                        } else {
                            UI_HEIGHT_NORMAL
                        };
                        let _ = ui_webview.set_bounds(Rect {
                            position: LogicalPosition::new(0.0, 0.0).into(),
                            size: LogicalSize::new(win_size.width, target_h).into(),
                        });
                    }

                    UiCommand::StartInPlaceUpdate { download_url, version } => {
                        let proxy_upd = proxy.clone();
                        std::thread::spawn(move || {
                            let _ = proxy_upd.send_event(UiCommand::InternalUpdateProgress {
                                pct: 10,
                                step: "เตรียมดาวน์โหลดเวอร์ชันใหม่".into(),
                                log: format!("กำลังเชื่อมต่อเซิร์ฟเวอร์อัปเดต ({version})..."),
                            });

                            let current_exe = match std::env::current_exe() {
                                Ok(p) => p,
                                Err(e) => {
                                    let _ = proxy_upd.send_event(UiCommand::InternalUpdateComplete {
                                        success: false,
                                        message: format!("ไม่สามารถระบุตำแหน่งไฟล์ปัจจุบันได้: {e}"),
                                    });
                                    return;
                                }
                            };

                            let parent_dir = current_exe.parent().unwrap_or(std::path::Path::new("."));
                            let temp_download = parent_dir.join("nova_update_tmp.exe");
                            let old_backup = parent_dir.join(format!("{}.old", current_exe.file_name().unwrap_or_default().to_string_lossy()));

                            let _ = std::fs::remove_file(&temp_download);
                            let _ = std::fs::remove_file(&old_backup);

                            let _ = proxy_upd.send_event(UiCommand::InternalUpdateProgress {
                                pct: 30,
                                step: "กำลังดาวน์โหลดไฟล์อัปเดต".into(),
                                log: "กำลังรับส่งข้อมูลเวอร์ชันใหม่ด้วยความเร็วสูง...".into(),
                            });

                            let download_res = std::process::Command::new("curl.exe")
                                .args(&["-L", "--fail", "-sS", "-o", temp_download.to_string_lossy().as_ref(), &download_url])
                                .status();

                            let mut ok = download_res.map(|s| s.success()).unwrap_or(false);
                            if !ok || !temp_download.exists() || temp_download.metadata().map(|m| m.len()).unwrap_or(0) < 500_000 {
                                let _ = proxy_upd.send_event(UiCommand::InternalUpdateProgress {
                                    pct: 50,
                                    step: "กำลังดาวน์โหลดด้วยช่องทางสำรอง".into(),
                                    log: "สลับไปใช้ PowerShell WebClient...".into(),
                                });

                                let ps_cmd = format!(
                                    "[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12; (New-Object Net.WebClient).DownloadFile('{}', '{}')",
                                    download_url, temp_download.to_string_lossy()
                                );
                                let ps_status = std::process::Command::new("powershell.exe")
                                    .args(&["-NoProfile", "-NonInteractive", "-Command", &ps_cmd])
                                    .status();
                                ok = ps_status.map(|s| s.success()).unwrap_or(false);
                            }

                            if !ok || !temp_download.exists() || temp_download.metadata().map(|m| m.len()).unwrap_or(0) < 500_000 {
                                let _ = proxy_upd.send_event(UiCommand::InternalUpdateComplete {
                                    success: false,
                                    message: "การดาวน์โหลดไม่สมบูรณ์ กรุณาตรวจสอบการเชื่อมต่ออินเทอร์เน็ต".into(),
                                });
                                return;
                            }

                            let _ = proxy_upd.send_event(UiCommand::InternalUpdateProgress {
                                pct: 85,
                                step: "กำลังสลับแทนที่ไฟล์เวอร์ชันใหม่".into(),
                                log: "ตรวจสอบความสมบูรณ์สำเร็จ เตรียมแทนที่ไฟล์".into(),
                            });

                            if let Err(e) = std::fs::rename(&current_exe, &old_backup) {
                                let _ = proxy_upd.send_event(UiCommand::InternalUpdateComplete {
                                    success: false,
                                    message: format!("ไม่สามารถสลับชื่อไฟล์เดิมได้: {e}"),
                                });
                                return;
                            }

                            if let Err(e) = std::fs::rename(&temp_download, &current_exe) {
                                let _ = std::fs::rename(&old_backup, &current_exe);
                                let _ = proxy_upd.send_event(UiCommand::InternalUpdateComplete {
                                    success: false,
                                    message: format!("ไม่สามารถวางไฟล์ใหม่แทนที่ได้: {e}"),
                                });
                                return;
                            }

                            let _ = proxy_upd.send_event(UiCommand::InternalUpdateProgress {
                                pct: 100,
                                step: "อัปเดตเรียบร้อยแล้ว".into(),
                                log: format!("Nova Studio ได้รับการอัปเดตเป็นเวอร์ชัน {version} เรียบร้อยแล้ว!"),
                            });

                            let _ = proxy_upd.send_event(UiCommand::InternalUpdateComplete {
                                success: true,
                                message: format!("อัปเดตเป็น {version} สำเร็จเรียบร้อย!"),
                            });
                        });
                    }

                    UiCommand::RelaunchAfterUpdate => {
                        save_session_snapshot(&mut storage, &tabs, &tab_order, &current_workspace, active_tab_id, &blocked_counter, true);
                        flush_save_sync(storage.clone());
                        storage.save_direct();

                        if let Ok(current_exe) = std::env::current_exe() {
                            let _ = std::process::Command::new(current_exe).spawn();
                        }
                        std::process::exit(0);
                    }

                    UiCommand::InternalUpdateProgress { pct, step, log } => {
                        let script = format!(
                            "window.__novaOnUpdateProgress && window.__novaOnUpdateProgress({}, {}, {});",
                            pct,
                            serde_json::to_string(&step).unwrap_or_default(),
                            serde_json::to_string(&log).unwrap_or_default()
                        );
                        let _ = ui_webview.evaluate_script(&script);
                        if let Some(tab) = tabs.get(&active_tab_id) {
                            let _ = tab.webview.evaluate_script(&script);
                        }
                    }

                    UiCommand::InternalUpdateComplete { success, message } => {
                        let script = format!(
                            "window.__novaOnUpdateComplete && window.__novaOnUpdateComplete({}, {});",
                            success,
                            serde_json::to_string(&message).unwrap_or_default()
                        );
                        let _ = ui_webview.evaluate_script(&script);
                        if let Some(tab) = tabs.get(&active_tab_id) {
                            let _ = tab.webview.evaluate_script(&script);
                        }
                    }

                    UiCommand::CycleTilingMode => {
                        tiling_mode = match tiling_mode { 1 => 2, 2 => 3, _ => 1 };
                        let ws_tabs = tab_order.iter().filter(|&&tid| {
                            tabs.get(&tid).map(|t| t.workspace == current_workspace).unwrap_or(false)
                        }).count();
                        if tiling_mode as usize > ws_tabs {
                            let _ = proxy.send_event(UiCommand::NewTab {
                                url: Some("https://www.google.com".into()),
                                incognito: Some(false),
                                workspace: Some(current_workspace.clone()),
                                pinned: Some(false),
                            });
                        }
                        sync_all_views(
                            &ui_webview, &panel_webview, &tabs, &tab_order, active_tab_id,
                            &current_workspace, win_size.width, win_size.height,
                            tiling_mode, side_panel_open, zen_mode, ui_modal_open, &mut storage, always_on_top, &ai_bridge, &blocked_counter,
                        );
                    }

                    UiCommand::ToggleSidePanel => {
                        side_panel_open = !side_panel_open;
                        sync_all_views(
                            &ui_webview, &panel_webview, &tabs, &tab_order, active_tab_id,
                            &current_workspace, win_size.width, win_size.height,
                            tiling_mode, side_panel_open, zen_mode, ui_modal_open, &mut storage, always_on_top, &ai_bridge, &blocked_counter,
                        );
                        if side_panel_open {
                            let _ = proxy.send_event(UiCommand::RequestPageExtract);
                        }
                    }
                    UiCommand::ToggleZenMode => {
                        zen_mode = !zen_mode;
                        sync_all_views(
                            &ui_webview, &panel_webview, &tabs, &tab_order, active_tab_id,
                            &current_workspace, win_size.width, win_size.height,
                            tiling_mode, side_panel_open, zen_mode, ui_modal_open, &mut storage, always_on_top, &ai_bridge, &blocked_counter,
                        );
                    }
                    UiCommand::ToggleAlwaysOnTop => {
                        always_on_top = !always_on_top;
                        window.set_always_on_top(always_on_top);
                        sync_all_views(
                            &ui_webview, &panel_webview, &tabs, &tab_order, active_tab_id,
                            &current_workspace, win_size.width, win_size.height,
                            tiling_mode, side_panel_open, zen_mode, ui_modal_open, &mut storage, always_on_top, &ai_bridge, &blocked_counter,
                        );
                    }
                    UiCommand::ToggleShadowMode { enable } => {
                        if enable {
                            window.set_outer_position(PhysicalPosition::new(-3000, -3000));
                        } else {
                            window.set_outer_position(PhysicalPosition::new(80, 60));
                            window.set_focus();
                        }
                    }

                    UiCommand::FindInPage { query } => {
                        if let Some(t) = tabs.get(&active_tab_id) {
                            let safe_q = serde_json::to_string(&query).unwrap_or_else(|_| "\"\"".to_string());
                            let _ = t.webview.evaluate_script(&format!("window.find({});", safe_q));
                        }
                    }
                    UiCommand::SavePageOffline => {
                        if let Some(t) = tabs.get(&active_tab_id) {
                            let _ = t.webview.evaluate_script("window.__novaSavePageOffline && window.__novaSavePageOffline();");
                        }
                    }
                    UiCommand::InternalSaveOfflineHtml { title, html, .. } => {
                        let _ = fs::create_dir_all("saved_pages");
                        let safe_name: String = title
                            .chars()
                            .map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' { c } else { '_' })
                            .take(40)
                            .collect();
                        let filename = format!("saved_pages/{}.html", if safe_name.trim().is_empty() { "page" } else { safe_name.trim() });
                        if let Ok(abs_path) = fs::canonicalize("saved_pages") {
                            let full_p = abs_path.join(format!("{}.html", if safe_name.trim().is_empty() { "page" } else { safe_name.trim() }));
                            let _ = fs::write(&full_p, &html);
                            let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
                            storage.downloads.insert(0, DownloadItem {
                                file_name: format!("{}.html", safe_name.trim()),
                                url: "Offline HTML Snapshot".to_string(),
                                path: full_p.display().to_string(),
                                status: "completed".to_string(),
                                timestamp: now,
                            });
                        } else {
                            let _ = fs::write(&filename, &html);
                        }
                        side_panel_open = true;
                        sync_all_views(
                            &ui_webview, &panel_webview, &tabs, &tab_order, active_tab_id,
                            &current_workspace, win_size.width, win_size.height,
                            tiling_mode, side_panel_open, zen_mode, ui_modal_open, &mut storage, always_on_top, &ai_bridge, &blocked_counter,
                        );
                    }

                    UiCommand::TranslatePageToThai => {
                        if let Some(t) = tabs.get(&active_tab_id) {
                            let _ = t.webview.evaluate_script("window.__novaTranslateToThai && window.__novaTranslateToThai();");
                        }
                    }
                    UiCommand::BoostAudio { gain } => {
                        if let Some(t) = tabs.get(&active_tab_id) {
                            let _ = t.webview.evaluate_script(&format!("window.__novaBoostAudio && window.__novaBoostAudio({});", gain));
                        }
                    }
                    UiCommand::ScrapePageData => {
                        if !side_panel_open {
                            side_panel_open = true;
                            sync_all_views(
                                &ui_webview, &panel_webview, &tabs, &tab_order, active_tab_id,
                                &current_workspace, win_size.width, win_size.height,
                                tiling_mode, side_panel_open, zen_mode, ui_modal_open, &mut storage, always_on_top, &ai_bridge, &blocked_counter,
                            );
                        }
                        if let Some(t) = tabs.get(&active_tab_id) {
                            let _ = t.webview.evaluate_script("window.__novaScrapeData && window.__novaScrapeData();");
                        }
                    }
                    UiCommand::ClearSiteData => {
                        if let Some(t) = tabs.get(&active_tab_id) {
                            let _ = t.webview.evaluate_script("window.__novaClearSiteData && window.__novaClearSiteData();");
                        }
                    }
                    UiCommand::ToggleRgxGpuShader => {
                        if let Some(t) = tabs.get(&active_tab_id) {
                            let _ = t.webview.evaluate_script("window.__novaToggleRgxShader && window.__novaToggleRgxShader();");
                        }
                    }
                    UiCommand::TogglePageDarkMode => {
                        if let Some(t) = tabs.get(&active_tab_id) {
                            let _ = t.webview.evaluate_script("window.__novaToggleDarkMode && window.__novaToggleDarkMode();");
                        }
                    }
                    UiCommand::ToggleReaderMode => {
                        if let Some(t) = tabs.get(&active_tab_id) {
                            let _ = t.webview.evaluate_script("window.__novaToggleReaderMode && window.__novaToggleReaderMode();");
                        }
                    }
                    UiCommand::ToggleVoiceRead => {
                        if let Some(t) = tabs.get(&active_tab_id) {
                            let _ = t.webview.evaluate_script("window.__novaToggleVoiceRead && window.__novaToggleVoiceRead();");
                        }
                    }
                    UiCommand::TriggerPiP => {
                        if let Some(t) = tabs.get(&active_tab_id) {
                            let _ = t.webview.evaluate_script("window.__novaTogglePiP && window.__novaTogglePiP();");
                        }
                    }
                    UiCommand::SetVideoSpeed { rate } => {
                        if let Some(t) = tabs.get(&active_tab_id) {
                            let _ = t.webview.evaluate_script(&format!("window.__novaBoostVideo && window.__novaBoostVideo({});", rate));
                        }
                    }
                    UiCommand::SetTheme { theme } => {
                        storage.theme = theme;
                        sync_all_views(
                            &ui_webview, &panel_webview, &tabs, &tab_order, active_tab_id,
                            &current_workspace, win_size.width, win_size.height,
                            tiling_mode, side_panel_open, zen_mode, ui_modal_open, &mut storage, always_on_top, &ai_bridge, &blocked_counter,
                        );
                    }
                    UiCommand::SaveNotes { notes } => {
                        storage.notes = notes;
                        storage.save();
                    }
                    UiCommand::ToggleAdBlock => {
                        let next_val = !shared_adblock.load(Ordering::Relaxed);
                        shared_adblock.store(next_val, Ordering::Relaxed);
                        storage.adblock_enabled = next_val;
                        for tab in tabs.values() {
                            let _ = tab.webview.evaluate_script(&format!("window.__NOVA_ADBLOCK = {};", next_val));
                        }
                        sync_all_views(
                            &ui_webview, &panel_webview, &tabs, &tab_order, active_tab_id,
                            &current_workspace, win_size.width, win_size.height,
                            tiling_mode, side_panel_open, zen_mode, ui_modal_open, &mut storage, always_on_top, &ai_bridge, &blocked_counter,
                        );
                    }

                    UiCommand::SetAsDefaultBrowser => {
                        let _ = register_default_browser();
                        let _ = ui_webview.evaluate_script("window.showToast && window.showToast('🎉 ลงทะเบียนใน Windows แล้ว! กำลังเปิดหน้าต่าง Default Apps...');");
                    }

                    UiCommand::ToggleVerticalTabs => {
                        storage.tab_layout = if storage.tab_layout == "vertical" {
                            "horizontal".to_string()
                        } else {
                            "vertical".to_string()
                        };
                        if storage.tab_layout == "vertical" {
                            side_panel_open = true;
                        }
                        storage.save();
                        sync_all_views(
                            &ui_webview, &panel_webview, &tabs, &tab_order, active_tab_id,
                            &current_workspace, win_size.width, win_size.height,
                            tiling_mode, side_panel_open, zen_mode, ui_modal_open, &mut storage, always_on_top, &ai_bridge, &blocked_counter,
                        );
                        let msg = format!("📑 สลับเลย์เอาต์แท็บเป็น: {}", if storage.tab_layout == "vertical" { "แนวตั้ง (Vertical)" } else { "แนวนอน (Horizontal)" });
                        let _ = ui_webview.evaluate_script(&format!("window.showToast && window.showToast(\"{}\");", msg));
                    }

                    UiCommand::SnoozeInactiveTabs => {
                        let mut count = 0;
                        for (&id, tab) in tabs.iter_mut() {
                            if id != active_tab_id && !tab.pinned && !tab.muted && !tab.hibernated {
                                tab.hibernated = true;
                                let _ = tab.webview.set_bounds(Rect {
                                    position: LogicalPosition::new(0.0, 0.0).into(),
                                    size: LogicalSize::new(0.0, 0.0).into(),
                                });
                                count += 1;
                            }
                        }
                        sync_all_views(
                            &ui_webview, &panel_webview, &tabs, &tab_order, active_tab_id,
                            &current_workspace, win_size.width, win_size.height,
                            tiling_mode, side_panel_open, zen_mode, ui_modal_open, &mut storage, always_on_top, &ai_bridge, &blocked_counter,
                        );
                        let msg = format!("❄️ แช่แข็ง {} แท็บเบื้องหลังเพื่อประหยัดแรมเรียบร้อยแล้ว!", count);
                        let _ = ui_webview.evaluate_script(&format!("window.showToast && window.showToast(\"{}\");", msg));
                    }

                    UiCommand::AddBookmark => {
                        if let Some(t) = tabs.get(&active_tab_id) {
                            if !t.url.is_empty() {
                                let short_title: String = t.title.chars().take(18).collect();
                                storage.bookmarks.push(BookmarkItem {
                                    title: short_title,
                                    url: t.url.clone(),
                                });
                                sync_all_views(
                                    &ui_webview, &panel_webview, &tabs, &tab_order, active_tab_id,
                                    &current_workspace, win_size.width, win_size.height,
                                    tiling_mode, side_panel_open, zen_mode, ui_modal_open, &mut storage, always_on_top, &ai_bridge, &blocked_counter,
                                );
                            }
                        }
                    }
                    UiCommand::RemoveBookmark { index } => {
                        if index < storage.bookmarks.len() {
                            storage.bookmarks.remove(index);
                            sync_all_views(
                                &ui_webview, &panel_webview, &tabs, &tab_order, active_tab_id,
                                &current_workspace, win_size.width, win_size.height,
                                tiling_mode, side_panel_open, zen_mode, ui_modal_open, &mut storage, always_on_top, &ai_bridge, &blocked_counter,
                            );
                        }
                    }
                    UiCommand::ClearHistory => {
                        storage.history.clear();
                        sync_all_views(
                            &ui_webview, &panel_webview, &tabs, &tab_order, active_tab_id,
                            &current_workspace, win_size.width, win_size.height,
                            tiling_mode, side_panel_open, zen_mode, ui_modal_open, &mut storage, always_on_top, &ai_bridge, &blocked_counter,
                        );
                    }
                    UiCommand::DeleteHistoryItem { index } => {
                        if index < storage.history.len() {
                            storage.history.remove(index);
                            sync_all_views(
                                &ui_webview, &panel_webview, &tabs, &tab_order, active_tab_id,
                                &current_workspace, win_size.width, win_size.height,
                                tiling_mode, side_panel_open, zen_mode, ui_modal_open, &mut storage, always_on_top, &ai_bridge, &blocked_counter,
                            );
                        }
                    }

                    UiCommand::EvalJs { script } => {
                        if let Some(t) = tabs.get(&active_tab_id) {
                            let _ = t.webview.evaluate_script(&script);
                        }
                    }
                    UiCommand::AiHighlightElements { show } => {
                        if let Some(t) = tabs.get(&active_tab_id) {
                            let _ = t.webview.evaluate_script(&format!("window.__novaHighlightElements && window.__novaHighlightElements({});", show));
                        }
                    }
                    UiCommand::AiClick { target } => {
                        if let Some(t) = tabs.get(&active_tab_id) {
                            let safe_target = serde_json::to_string(&target).unwrap_or_else(|_| "\"\"".to_string());
                            let _ = t.webview.evaluate_script(&format!("window.__novaClick && window.__novaClick({});", safe_target));
                        }
                    }
                    UiCommand::AiType { target, text } => {
                        if let Some(t) = tabs.get(&active_tab_id) {
                            let safe_target = serde_json::to_string(&target).unwrap_or_else(|_| "\"\"".to_string());
                            let safe_text = serde_json::to_string(&text).unwrap_or_else(|_| "\"\"".to_string());
                            let _ = t.webview.evaluate_script(&format!("window.__novaType && window.__novaType({}, {});", safe_target, safe_text));
                        }
                    }
                    UiCommand::AiAutopilotGoal { goal } => {
                        if let Some(t) = tabs.get(&active_tab_id) {
                            let safe_goal = serde_json::to_string(&goal).unwrap_or_else(|_| "\"\"".to_string());
                            let _ = t.webview.evaluate_script(&format!("window.__novaRunAutopilot && window.__novaRunAutopilot({});", safe_goal));
                        }
                    }
                    UiCommand::RequestPageExtract => {
                        if let Some(t) = tabs.get(&active_tab_id) {
                            let _ = t.webview.evaluate_script("window.__novaExtractDOM && window.__novaExtractDOM();");
                        }
                    }

                    UiCommand::InternalTitleChanged { id, title } => {
                        if let Some(tab) = tabs.get_mut(&id) {
                            if !tab.hibernated {
                                tab.title = if title.trim().is_empty() { "แท็บใหม่".to_string() } else { title.clone() };
                            }
                            if let Some(first_hist) = storage.history.first_mut() {
                                if first_hist.url == tab.url && !title.trim().is_empty() {
                                    first_hist.title = title;
                                }
                            }
                            sync_all_views(
                                &ui_webview, &panel_webview, &tabs, &tab_order, active_tab_id,
                                &current_workspace, win_size.width, win_size.height,
                                tiling_mode, side_panel_open, zen_mode, ui_modal_open, &mut storage, always_on_top, &ai_bridge, &blocked_counter,
                            );
                        }
                    }
                    UiCommand::InternalUrlChanged { id, url, .. } => {
                        if let Some(tab) = tabs.get_mut(&id) {
                            tab.url = url.clone();
                            if !tab.incognito && !url.is_empty() && url != "about:blank" {
                                let is_dup = storage.history.first().map(|h| h.url == url).unwrap_or(false);
                                if !is_dup {
                                    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
                                    storage.history.insert(0, HistoryItem {
                                        title: tab.title.clone(),
                                        url,
                                        timestamp: now,
                                    });
                                    if storage.history.len() > 500 {
                                        storage.history.truncate(500);
                                    }
                                }
                            }
                            sync_all_views(
                                &ui_webview, &panel_webview, &tabs, &tab_order, active_tab_id,
                                &current_workspace, win_size.width, win_size.height,
                                tiling_mode, side_panel_open, zen_mode, ui_modal_open, &mut storage, always_on_top, &ai_bridge, &blocked_counter,
                            );
                        }
                    }
                    UiCommand::InternalPageExtracted { report, .. } => {
                        let (lock, cvar) = &*ai_bridge;
                        if let Ok(mut guard) = lock.lock() {
                            guard.latest_page_report = report.clone();
                            guard.extract_seq += 1;
                            cvar.notify_all();
                        }
                        if side_panel_open {
                            let safe_report = serde_json::to_string(&report).unwrap_or_else(|_| "\"\"".to_string());
                            let _ = panel_webview.evaluate_script(&format!("window.updateAiReport && window.updateAiReport({});", safe_report));
                        }
                    }
                    UiCommand::InternalAdBlocked { count, .. } => {
                        let add = count.unwrap_or(1);
                        let total = blocked_counter.fetch_add(add, Ordering::Relaxed) + add;
                        storage.blocked_count = total;
                        let _ = ui_webview.evaluate_script(&format!("window.updateBlockedCount && window.updateBlockedCount({});", total));
                    }
                }
            }
            _ => {}
        }
    });
}

fn save_session_snapshot(
    storage: &mut PersistentData,
    tabs: &HashMap<usize, Tab>,
    tab_order: &[usize],
    current_workspace: &str,
    active_id: usize,
    blocked_counter: &Arc<AtomicUsize>,
    persist_to_disk: bool,
) {
    let mut saved = Vec::new();
    let mut active_idx = 0;

    for &id in tab_order {
        if let Some(t) = tabs.get(&id) {
            if !t.incognito {
                if id == active_id {
                    active_idx = saved.len();
                }
                saved.push(SavedTab {
                    title: t.title.clone(),
                    url: t.url.clone(),
                    workspace: t.workspace.clone(),
                    pinned: t.pinned,
                });
            }
        }
    }

    storage.saved_tabs = saved;
    storage.active_workspace = current_workspace.to_string();
    storage.active_tab_index = active_idx;
    storage.blocked_count = blocked_counter.load(Ordering::Relaxed);
    if persist_to_disk {
        storage.save();
    }
}

#[allow(clippy::too_many_arguments)]
fn sync_all_views(
    ui_webview: &WebView,
    panel_webview: &WebView,
    tabs: &HashMap<usize, Tab>,
    tab_order: &[usize],
    active_id: usize,
    current_workspace: &str,
    win_width: f64,
    win_height: f64,
    tiling_mode: u8,
    panel_open: bool,
    zen_mode: bool,
    ui_modal_open: bool,
    storage: &mut PersistentData,
    always_on_top: bool,
    ai_bridge: &Arc<(Mutex<SharedAiBridge>, Condvar)>,
    blocked_counter: &Arc<AtomicUsize>,
) {
    save_session_snapshot(storage, tabs, tab_order, current_workspace, active_id, blocked_counter, false);

    let ui_h = if ui_modal_open { win_height } else if zen_mode { UI_HEIGHT_ZEN } else { UI_HEIGHT_NORMAL };
    let normal_ui_h = if zen_mode { UI_HEIGHT_ZEN } else { UI_HEIGHT_NORMAL };
    let content_height = (win_height - normal_ui_h).max(0.0);

    let _ = ui_webview.set_bounds(Rect {
        position: LogicalPosition::new(0.0, 0.0).into(),
        size: LogicalSize::new(win_width, ui_h).into(),
    });

    let avail_width = if panel_open {
        let pw = PANEL_WIDTH.min(win_width * 0.45);
        let _ = panel_webview.set_bounds(Rect {
            position: LogicalPosition::new(win_width - pw, normal_ui_h).into(),
            size: LogicalSize::new(pw, content_height).into(),
        });
        (win_width - pw).max(0.0)
    } else {
        let _ = panel_webview.set_bounds(Rect {
            position: LogicalPosition::new(0.0, 0.0).into(),
            size: LogicalSize::new(0.0, 0.0).into(),
        });
        win_width
    };

    let other_ws_tabs: Vec<usize> = tab_order
        .iter()
        .rev()
        .copied()
        .filter(|&id| id != active_id && tabs.get(&id).map(|t| t.workspace == current_workspace).unwrap_or(false))
        .collect();

    let second_id = if tiling_mode >= 2 { other_ws_tabs.first().copied() } else { None };
    let third_id = if tiling_mode >= 3 { other_ws_tabs.get(1).copied() } else { None };

    let mut tab_list = Vec::new();
    let mut active_url = String::new();
    let mut active_title = String::new();

    for &id in tab_order {
        if let Some(tab) = tabs.get(&id) {
            if id == active_id {
                let w = if second_id.is_some() { avail_width * 0.5 } else { avail_width };
                let _ = tab.webview.set_bounds(Rect {
                    position: LogicalPosition::new(0.0, normal_ui_h).into(),
                    size: LogicalSize::new(w, content_height).into(),
                });
                active_url = tab.url.clone();
                active_title = tab.title.clone();
            } else if Some(id) == second_id {
                let half_w = avail_width * 0.5;
                let h = if third_id.is_some() { content_height * 0.5 } else { content_height };
                let _ = tab.webview.set_bounds(Rect {
                    position: LogicalPosition::new(half_w, normal_ui_h).into(),
                    size: LogicalSize::new(half_w, h).into(),
                });
            } else if Some(id) == third_id {
                let half_w = avail_width * 0.5;
                let half_h = content_height * 0.5;
                let _ = tab.webview.set_bounds(Rect {
                    position: LogicalPosition::new(half_w, normal_ui_h + half_h).into(),
                    size: LogicalSize::new(half_w, half_h).into(),
                });
            } else {
                let _ = tab.webview.set_bounds(Rect {
                    position: LogicalPosition::new(0.0, 0.0).into(),
                    size: LogicalSize::new(0.0, 0.0).into(),
                });
            }

            if tab.workspace == current_workspace {
                tab_list.push(serde_json::json!({
                    "id": tab.id,
                    "title": tab.title,
                    "url": tab.url,
                    "active": id == active_id,
                    "split": Some(id) == second_id || Some(id) == third_id,
                    "pinned": tab.pinned,
                    "hibernated": tab.hibernated,
                    "incognito": tab.incognito,
                    "muted": tab.muted,
                    "loading": tab.loading,
                }));
            }
        }
    }

    if let Ok(mut guard) = ai_bridge.0.lock() {
        guard.active_url = active_url.clone();
        guard.active_title = active_title;
    }

    let active_zoom = tabs.get(&active_id).map(|t| t.zoom_level).unwrap_or(1.0);
    let state = serde_json::json!({
        "tabs": tab_list,
        "workspace": current_workspace,
        "activeUrl": active_url,
        "bookmarks": storage.bookmarks,
        "bookmarksBar": storage.bookmarks_bar,
        "zoomLevel": active_zoom,
        "adblock": storage.adblock_enabled,
        "blockedCount": storage.blocked_count,
        "historyCount": storage.history.len(),
        "downloadCount": storage.downloads.len(),
        "closedCount": storage.closed_tabs.len(),
        "tilingMode": tiling_mode,
        "panelOpen": panel_open,
        "zenMode": zen_mode,
        "alwaysOnTop": always_on_top,
        "theme": storage.theme,
        "tabLayout": storage.tab_layout,
        "ramSavedMb": tabs.values().filter(|t| t.hibernated).count() * 250,
    });

    let _ = ui_webview.evaluate_script(&format!("window.renderBrowserState && window.renderBrowserState({});", state));

    if panel_open {
        let hib_count = tabs.values().filter(|t| t.hibernated).count();
        let panel_data = serde_json::json!({
            "history": storage.history,
            "downloads": storage.downloads,
            "notes": storage.notes,
            "bookmarks": storage.bookmarks,
            "tabs": tab_list,
            "tabLayout": storage.tab_layout,
            "workspace": current_workspace,
            "ramSavedMb": hib_count * 250,
        });
        let _ = panel_webview.evaluate_script(&format!("window.renderPanelData && window.renderPanelData({});", panel_data));
    }
}

fn request_live_dom_snapshot(
    proxy: &tao::event_loop::EventLoopProxy<UiCommand>,
    ai_bridge: &Arc<(Mutex<SharedAiBridge>, Condvar)>,
) -> String {
    let (lock, cvar) = &**ai_bridge;
    let start_seq = lock.lock().map(|g| g.extract_seq).unwrap_or(0);
    let _ = proxy.send_event(UiCommand::RequestPageExtract);

    if let Ok(guard) = lock.lock() {
        let res = cvar.wait_timeout_while(guard, Duration::from_millis(1500), |g| g.extract_seq == start_seq);
        if let Ok((g, _)) = res {
            return g.latest_page_report.clone();
        }
    }
    "ไม่สามารถดึงข้อมูลหน้าเว็บได้ในขณะนี้".to_string()
}

fn start_ai_agent_server(
    port: u16,
    proxy: tao::event_loop::EventLoopProxy<UiCommand>,
    ai_bridge: Arc<(Mutex<SharedAiBridge>, Condvar)>,
    required_token: Option<String>,
) {
    let addr = format!("127.0.0.1:{}", port);
    if let Ok(listener) = TcpListener::bind(&addr) {
        println!("🤖 [Nova Complete AI & MCP Server] พร้อมรับคำสั่งที่ http://{} (CSRF Protected)", addr);
        for stream in listener.incoming().flatten() {
            let mut stream = stream;
            let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
            let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
            let mut buffer = [0; 16384];
            if let Ok(size) = stream.read(&mut buffer) {
                let req_str = String::from_utf8_lossy(&buffer[..size]);
                let mut parts = req_str.splitn(2, "\r\n\r\n");
                let headers_raw = parts.next().unwrap_or("");
                let body = parts.next().unwrap_or("").trim_matches(char::from(0)).trim();

                let mut has_browser_origin = false;
                let mut provided_token: Option<String> = None;

                for line in headers_raw.lines() {
                    let lower = line.to_lowercase();
                    if lower.starts_with("origin:") || lower.starts_with("referer:") {
                        has_browser_origin = true;
                    }
                    if lower.starts_with("sec-fetch-site:") && !lower.contains("none") {
                        has_browser_origin = true;
                    }
                    if lower.starts_with("x-nova-token:") {
                        provided_token = Some(line[13..].trim().to_string());
                    } else if lower.starts_with("authorization:") {
                        let raw_auth = line[14..].trim();
                        if let Some(bearer) = raw_auth.strip_prefix("Bearer ") {
                            provided_token = Some(bearer.trim().to_string());
                        }
                    }
                }

                if has_browser_origin {
                    let _ = stream.write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Type: application/json\r\nContent-Length: 24\r\nConnection: close\r\n\r\n{\"error\":\"CSRF Blocked\"}");
                    continue;
                }

                if let Some(ref req_tok) = required_token {
                    if provided_token.as_deref() != Some(req_tok.as_str()) {
                        let _ = stream.write_all(b"HTTP/1.1 401 Unauthorized\r\nContent-Type: application/json\r\nContent-Length: 24\r\nConnection: close\r\n\r\n{\"error\":\"Unauthorized\"}");
                        continue;
                    }
                }

                if let Ok(json_val) = serde_json::from_str::<serde_json::Value>(body) {
                    if json_val.get("jsonrpc").is_some() {
                        let method = json_val["method"].as_str().unwrap_or("");
                        let id = json_val.get("id").cloned().unwrap_or(serde_json::Value::Null);

                        let mcp_response = match method {
                            "tools/list" => serde_json::json!({
                                "jsonrpc": "2.0", "id": id,
                                "result": {
                                    "tools": [
                                        { "name": "browser_navigate", "description": "เปิดเว็บไซต์ตาม URL", "inputSchema": { "type": "object", "properties": { "url": {"type": "string"} } } },
                                        { "name": "browser_read_page", "description": "อ่านข้อความบนหน้าเว็บและดึงรายการปุ่มพร้อมหมายเลข [#ID]", "inputSchema": { "type": "object", "properties": {} } },
                                        { "name": "browser_autopilot", "description": "สั่งบอทค้นหาหรือคลิกปุ่มตามเป้าหมายอัตโนมัติ", "inputSchema": { "type": "object", "properties": { "goal": {"type": "string"} } } },
                                        { "name": "browser_click", "description": "คลิกปุ่มด้วยหมายเลข ID หรือ CSS Selector", "inputSchema": { "type": "object", "properties": { "target": {"type": "string"} } } },
                                        { "name": "browser_type", "description": "พิมพ์ข้อความลงในช่อง Input ด้วยหมายเลข ID หรือ CSS Selector", "inputSchema": { "type": "object", "properties": { "target": {"type": "string"}, "text": {"type": "string"} } } },
                                        { "name": "browser_shadow_mode", "description": "ส่งเบราว์เซอร์เข้ามิติมืด (-3000,-3000) หรือดึงกลับ", "inputSchema": { "type": "object", "properties": { "enable": {"type": "boolean"} } } }
                                    ]
                                }
                            }),
                            "tools/call" => {
                                let tool_name = json_val["params"]["name"].as_str().unwrap_or("");
                                let args = &json_val["params"]["arguments"];
                                let mut reply_text = "✅ สั่งงาน Nova Complete สำเร็จ!".to_string();

                                let mut is_known_tool = true;
                                match tool_name {
                                    "browser_navigate" => { if let Some(u) = args["url"].as_str() { let _ = proxy.send_event(UiCommand::Navigate { url: u.to_string() }); } }
                                    "browser_read_page" => { reply_text = request_live_dom_snapshot(&proxy, &ai_bridge); }
                                    "browser_autopilot" => { if let Some(g) = args["goal"].as_str() { let _ = proxy.send_event(UiCommand::AiAutopilotGoal { goal: g.to_string() }); } }
                                    "browser_click" => { if let Some(t) = args["target"].as_str() { let _ = proxy.send_event(UiCommand::AiClick { target: t.to_string() }); } }
                                    "browser_type" => { if let (Some(t), Some(txt)) = (args["target"].as_str(), args["text"].as_str()) { let _ = proxy.send_event(UiCommand::AiType { target: t.to_string(), text: txt.to_string() }); } }
                                    "browser_shadow_mode" => { if let Some(en) = args["enable"].as_bool() { let _ = proxy.send_event(UiCommand::ToggleShadowMode { enable: en }); } }
                                    _ => { is_known_tool = false; }
                                }
                                if is_known_tool {
                                    serde_json::json!({ "jsonrpc": "2.0", "id": id, "result": { "content": [{ "type": "text", "text": reply_text }] } })
                                } else {
                                    serde_json::json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32601, "message": format!("Unknown tool: {}", tool_name) } })
                                }
                            }
                            _ => serde_json::json!({ "jsonrpc": "2.0", "id": id, "result": {} }),
                        };

                        let res_str = mcp_response.to_string();
                        let http_res = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", res_str.len(), res_str);
                        let _ = stream.write_all(http_res.as_bytes());
                        continue;
                    }

                    if json_val["cmd"].as_str() == Some("RequestPageExtract") {
                        let snapshot = request_live_dom_snapshot(&proxy, &ai_bridge);
                        let res_json = serde_json::json!({ "status": "ok", "page_report": snapshot }).to_string();
                        let http_res = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", res_json.len(), res_json);
                        let _ = stream.write_all(http_res.as_bytes());
                        continue;
                    }
                }

                if let Ok(cmd) = serde_json::from_str::<UiCommand>(body) {
                    let _ = proxy.send_event(cmd);
                    let res = r#"{"status":"ok","message":"Command executed in Nova Complete v5.1"}"#;
                    let http_res = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", res.len(), res);
                    let _ = stream.write_all(http_res.as_bytes());
                    continue;
                }

                let _ = stream.write_all(b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
            }
        }
    }
}
