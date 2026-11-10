// src/installer.rs
// Nova Studio — Modern 2027 Spatial Setup Installer (Arc/Chrome Style)
#![windows_subsystem = "windows"]

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::thread;
use std::time::Duration;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

use serde::{Deserialize, Serialize};
use tao::dpi::{LogicalPosition, LogicalSize};
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::window::WindowBuilder;
use wry::{Rect, WebViewBuilder};

const INSTALLER_HTML: &str = include_str!("installer.html");
const NOVA_BIN: &[u8] = include_bytes!("../target/release/my_rust_browser.exe");
const ADBLOCK_RULES: &str = include_str!("../adblock_rules.txt");
const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

#[allow(non_snake_case)]
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "cmd")]
enum InstallerIpc {
    Ready,
    BrowseFolder { current: Option<String> },
    StartInstall {
        installDir: String,
        desktopShortcut: bool,
        startMenuShortcut: bool,
        setDefaultBrowser: bool,
        launchAfter: bool,
    },
    LaunchApp { targetExe: String },
    CloseInstaller,
    // Internal thread signals
    InternalProgress { pct: u32, step: String, log: String },
    InternalComplete { target_exe: String, auto_launched: bool },
    InternalFolderSelected { path: String },
}

#[allow(non_snake_case)]
#[derive(Serialize)]
struct DriveInfo {
    letter: char,
    totalGb: f64,
    freeGb: f64,
}

unsafe extern "system" {
    fn GetDiskFreeSpaceExW(
        directory_name: *const u16,
        free_bytes_available: *mut u64,
        total_number_of_bytes: *mut u64,
        total_number_of_free_bytes: *mut u64,
    ) -> i32;
}

fn scan_drives() -> Vec<DriveInfo> {
    let mut list = Vec::new();
    for c in b'C'..=b'Z' {
        let drive_str = format!("{}:\\\0", c as char);
        let wide: Vec<u16> = drive_str.encode_utf16().collect();
        let mut free: u64 = 0;
        let mut total: u64 = 0;
        let res = unsafe {
            GetDiskFreeSpaceExW(wide.as_ptr(), &mut free, &mut total, std::ptr::null_mut())
        };
        if res != 0 && total > 0 {
            list.push(DriveInfo {
                letter: c as char,
                totalGb: (total as f64 / 1_000_000_000.0 * 10.0).round() / 10.0,
                freeGb: (free as f64 / 1_000_000_000.0 * 10.0).round() / 10.0,
            });
        }
    }
    list
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let event_loop = EventLoopBuilder::<InstallerIpc>::with_user_event().build();
    let proxy = event_loop.create_proxy();

    let window = WindowBuilder::new()
        .with_title("Nova Studio — ตัวติดตั้งระบบ (Setup Installer)")
        .with_inner_size(LogicalSize::new(560.0, 560.0))
        .with_resizable(false)
        .build(&event_loop)?;

    let proxy_ipc = proxy.clone();
    let webview = WebViewBuilder::new()
        .with_html(INSTALLER_HTML)
        .with_bounds(Rect {
            position: LogicalPosition::new(0.0, 0.0).into(),
            size: LogicalSize::new(560.0, 560.0).into(),
        })
        .with_ipc_handler(move |req| {
            if let Ok(cmd) = serde_json::from_str::<InstallerIpc>(req.body()) {
                let _ = proxy_ipc.send_event(cmd);
            }
        })
        .build_as_child(&window)?;

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;

        match event {
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => {
                *control_flow = ControlFlow::Exit;
            }
            Event::UserEvent(cmd) => {
                match cmd {
                    InstallerIpc::Ready => {
                        let drives = scan_drives();
                        let local_app_data = match std::env::var("LOCALAPPDATA") {
                            Ok(v) => PathBuf::from(v),
                            Err(_) => {
                                let user_profile = std::env::var("USERPROFILE").unwrap_or_else(|_| "C:\\".to_string());
                                PathBuf::from(user_profile).join("AppData").join("Local")
                            }
                        };
                        let default_path = local_app_data.join("Programs").join("NovaStudio").to_string_lossy().to_string();

                        let init_data = serde_json::json!({
                            "defaultPath": default_path,
                            "drives": drives,
                            "version": APP_VERSION,
                        });
                        let _ = webview.evaluate_script(&format!(
                            "window.initInstallerData && window.initInstallerData({});",
                            init_data
                        ));
                    }

                    InstallerIpc::BrowseFolder { current } => {
                        let proxy_browse = proxy.clone();
                        thread::spawn(move || {
                            let initial = current.unwrap_or_default().replace('\\', "\\\\");
                            let ps_cmd = format!(
                                r#"Add-Type -AssemblyName System.Windows.Forms
$f = New-Object System.Windows.Forms.FolderBrowserDialog
$f.Description = 'เลือกโฟลเดอร์สำหรับติดตั้ง Nova Studio'
$f.SelectedPath = '{}'
$f.ShowNewFolderButton = $true
if ($f.ShowDialog() -eq [System.Windows.Forms.DialogResult]::OK) {{
    Write-Output $f.SelectedPath
}}"#,
                                initial
                            );
                            #[cfg(windows)]
                            let out = Command::new("powershell")
                                .args(["-NoProfile", "-NonInteractive", "-Command", &ps_cmd])
                                .creation_flags(0x08000000)
                                .output();

                            #[cfg(windows)]
                            if let Ok(res) = out {
                                let raw = String::from_utf8_lossy(&res.stdout).trim().to_string();
                                if !raw.is_empty() {
                                    let _ = proxy_browse.send_event(InstallerIpc::InternalFolderSelected { path: raw });
                                }
                            }
                        });
                    }

                    InstallerIpc::InternalFolderSelected { path } => {
                        let json_path = serde_json::to_string(&path).unwrap_or_else(|_| "\"\"".to_string());
                        let _ = webview.evaluate_script(&format!("window.onFolderSelected && window.onFolderSelected({});", json_path));
                    }

                    InstallerIpc::StartInstall {
                        installDir,
                        desktopShortcut,
                        startMenuShortcut,
                        setDefaultBrowser,
                        launchAfter,
                    } => {
                        let proxy_worker = proxy.clone();
                        thread::spawn(move || {
                            let target_dir = PathBuf::from(&installDir);

                            // Step 1: Create directories
                            let _ = proxy_worker.send_event(InstallerIpc::InternalProgress {
                                pct: 15,
                                step: "กำลังสร้างโฟลเดอร์ปลายทาง...".to_string(),
                                log: format!("📁 สร้างโฟลเดอร์: {}", target_dir.display()),
                            });
                            thread::sleep(Duration::from_millis(300));

                            if let Err(e) = fs::create_dir_all(&target_dir) {
                                let _ = proxy_worker.send_event(InstallerIpc::InternalProgress {
                                    pct: 15,
                                    step: format!("⚠️ ไม่สามารถสร้างโฟลเดอร์ได้: {}", e),
                                    log: format!("ข้อผิดพลาด: {}", e),
                                });
                                return;
                            }

                            // Step 2: Write browser executable
                            let target_exe = target_dir.join("Nova Studio.exe");
                            let _ = proxy_worker.send_event(InstallerIpc::InternalProgress {
                                pct: 45,
                                step: "กำลังติดตั้งไฟล์โปรแกรม Nova Studio...".to_string(),
                                log: format!("🚀 บันทึกไฟล์: {}", target_exe.display()),
                            });
                            thread::sleep(Duration::from_millis(350));

                            if let Err(e) = fs::write(&target_exe, NOVA_BIN) {
                                let _ = proxy_worker.send_event(InstallerIpc::InternalProgress {
                                    pct: 45,
                                    step: format!("⚠️ เขียนไฟล์โปรแกรมไม่สำเร็จ: {}", e),
                                    log: format!("ข้อผิดพลาด: {}", e),
                                });
                                return;
                            }

                            // Step 3: Write EasyList / Adblock Rules
                            let target_rules = target_dir.join("adblock_rules.txt");
                            let _ = proxy_worker.send_event(InstallerIpc::InternalProgress {
                                pct: 70,
                                step: "กำลังติดตั้งฐานข้อมูล EasyList Adblock...".to_string(),
                                log: "🛡️ ติดตั้งกฎบล็อกโฆษณาและระบบต่อต้านการติดตาม".to_string(),
                            });
                            let _ = fs::write(&target_rules, ADBLOCK_RULES);
                            thread::sleep(Duration::from_millis(300));

                            let exe_str = target_exe.to_string_lossy().to_string();

                            // Step 4: Create shortcuts
                            if desktopShortcut || startMenuShortcut {
                                let _ = proxy_worker.send_event(InstallerIpc::InternalProgress {
                                    pct: 85,
                                    step: "กำลังสร้างทางลัดบน Desktop และ Start Menu...".to_string(),
                                    log: "📌 สร้างทางลัด Windows".to_string(),
                                });

                                let mut shortcut_lines = vec![
                                    "$ws = New-Object -ComObject WScript.Shell".to_string()
                                ];
                                if desktopShortcut {
                                    shortcut_lines.push(format!(
                                        r#"$s1 = $ws.CreateShortcut([System.Environment]::GetFolderPath('Desktop') + '\Nova Studio.lnk')
$s1.TargetPath = '{}'
$s1.Description = 'Nova Studio — 2027 Spatial Web Browser'
$s1.Save()"#,
                                        exe_str.replace('\\', "\\\\")
                                    ));
                                }
                                if startMenuShortcut {
                                    shortcut_lines.push(format!(
                                        r#"$s2 = $ws.CreateShortcut([System.Environment]::GetFolderPath('Programs') + '\Nova Studio.lnk')
$s2.TargetPath = '{}'
$s2.Description = 'Nova Studio — 2027 Spatial Web Browser'
$s2.Save()"#,
                                        exe_str.replace('\\', "\\\\")
                                    ));
                                }

                                #[cfg(windows)]
                                let _ = Command::new("powershell")
                                    .args(["-NoProfile", "-NonInteractive", "-Command", &shortcut_lines.join("\n")])
                                    .creation_flags(0x08000000)
                                    .status();
                            }

                            // Step 5: Register in Windows Registry (Default browser & Uninstall)
                            let _ = proxy_worker.send_event(InstallerIpc::InternalProgress {
                                pct: 95,
                                step: "กำลังลงทะเบียนโปรโตคอลระบบ Windows...".to_string(),
                                log: "⚙️ ลงทะเบียนใน Windows Registry (HKCU)".to_string(),
                            });

                            let mut reg_commands = vec![
                                format!(r#"reg add "HKCU\Software\Classes\NovaStudio.HTML" /ve /d "Nova Studio HTML Document" /f"#),
                                format!(r#"reg add "HKCU\Software\Classes\NovaStudio.HTML\DefaultIcon" /ve /d "\"{}\",0" /f"#, exe_str),
                                format!(r#"reg add "HKCU\Software\Classes\NovaStudio.HTML\shell\open\command" /ve /d "\"\"{}\"\" \"%1\"" /f"#, exe_str),
                                format!(r#"reg add "HKCU\Software\Clients\StartMenuInternet\NovaStudio" /ve /d "Nova Studio" /f"#),
                                format!(r#"reg add "HKCU\Software\Clients\StartMenuInternet\NovaStudio\DefaultIcon" /ve /d "\"{}\",0" /f"#, exe_str),
                                format!(r#"reg add "HKCU\Software\Clients\StartMenuInternet\NovaStudio\shell\open\command" /ve /d "\"\"{}\"\"" /f"#, exe_str),
                                format!(r#"reg add "HKCU\Software\Clients\StartMenuInternet\NovaStudio\Capabilities" /v "ApplicationName" /d "Nova Studio" /f"#),
                                format!(r#"reg add "HKCU\Software\Clients\StartMenuInternet\NovaStudio\Capabilities" /v "ApplicationDescription" /d "Nova Studio — 2027 Spatial Web Browser" /f"#),
                                format!(r#"reg add "HKCU\Software\RegisteredApplications" /v "NovaStudio" /d "Software\Clients\StartMenuInternet\NovaStudio\Capabilities" /f"#),
                                // Windows Add/Remove Programs (Uninstall entry)
                                format!(r#"reg add "HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\NovaStudio" /v "DisplayName" /d "Nova Studio" /f"#),
                                format!(r#"reg add "HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\NovaStudio" /v "DisplayVersion" /d "{}" /f"#, APP_VERSION),
                                format!(r#"reg add "HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\NovaStudio" /v "Publisher" /d "Nova Studio Team" /f"#),
                                format!(r#"reg add "HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\NovaStudio" /v "DisplayIcon" /d "\"{}\",0" /f"#, exe_str),
                                format!(r#"reg add "HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\NovaStudio" /v "InstallLocation" /d "\"{}\"" /f"#, target_dir.to_string_lossy()),
                                format!(r#"reg add "HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\NovaStudio" /v "UninstallString" /d "cmd.exe /C rmdir /S /Q \"{}\"" /f"#, target_dir.to_string_lossy()),
                            ];

                            if setDefaultBrowser {
                                reg_commands.push(format!(r#"reg add "HKCU\Software\Clients\StartMenuInternet\NovaStudio\Capabilities\URLAssociations" /v "http" /d "NovaStudio.HTML" /f"#));
                                reg_commands.push(format!(r#"reg add "HKCU\Software\Clients\StartMenuInternet\NovaStudio\Capabilities\URLAssociations" /v "https" /d "NovaStudio.HTML" /f"#));
                            }

                            for cmd in reg_commands {
                                #[cfg(windows)]
                                let _ = Command::new("cmd")
                                    .args(["/C", &cmd])
                                    .creation_flags(0x08000000)
                                    .status();
                            }

                            if setDefaultBrowser {
                                #[cfg(windows)]
                                let _ = Command::new("explorer")
                                    .arg("ms-settings:defaultapps")
                                    .spawn();
                            }

                            // Step 6: Complete!
                            let _ = proxy_worker.send_event(InstallerIpc::InternalProgress {
                                pct: 100,
                                step: "🎉 ติดตั้งเสร็จสมบูรณ์!".to_string(),
                                log: "✨ พร้อมใช้งาน Nova Studio แล้ว".to_string(),
                            });
                            thread::sleep(Duration::from_millis(400));

                            let auto_launched = if launchAfter {
                                #[cfg(windows)]
                                let _ = Command::new(&target_exe).spawn();
                                true
                            } else {
                                false
                            };

                            let _ = proxy_worker.send_event(InstallerIpc::InternalComplete {
                                target_exe: exe_str,
                                auto_launched,
                            });
                        });
                    }

                    InstallerIpc::InternalProgress { pct, step, log } => {
                        let step_json = serde_json::to_string(&step).unwrap_or_default();
                        let log_json = serde_json::to_string(&log).unwrap_or_default();
                        let _ = webview.evaluate_script(&format!(
                            "window.updateInstallProgress && window.updateInstallProgress({}, {}, {});",
                            pct, step_json, log_json
                        ));
                    }

                    InstallerIpc::InternalComplete { target_exe, auto_launched } => {
                        let exe_json = serde_json::to_string(&target_exe).unwrap_or_default();
                        let _ = webview.evaluate_script(&format!(
                            "window.onInstallComplete && window.onInstallComplete({}, {});",
                            exe_json, auto_launched
                        ));
                    }

                    InstallerIpc::LaunchApp { targetExe } => {
                        #[cfg(windows)]
                        let _ = Command::new(&targetExe).spawn();
                    }

                    InstallerIpc::CloseInstaller => {
                        *control_flow = ControlFlow::Exit;
                    }
                }
            }
            _ => {}
        }
    });
}
