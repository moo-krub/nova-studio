// src/installer.rs
// Nova Studio 1-Click Setup Installer for Windows 10/11
#![windows_subsystem = "windows"]

use std::fs;
use std::path::PathBuf;
use std::process::Command;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

// Embed the compiled Nova Studio release binary
const NOVA_BIN: &[u8] = include_bytes!("../target/release/my_rust_browser.exe");
const APP_VERSION: &str = "6.0.0";

fn main() {
    let local_app_data = match std::env::var("LOCALAPPDATA") {
        Ok(v) => PathBuf::from(v),
        Err(_) => {
            let user_profile = std::env::var("USERPROFILE").unwrap_or_else(|_| "C:\\".to_string());
            PathBuf::from(user_profile).join("AppData").join("Local")
        }
    };

    let install_dir = local_app_data.join("Programs").join("NovaStudio");
    let _ = fs::create_dir_all(&install_dir);

    let target_exe = install_dir.join("Nova Studio.exe");
    if let Err(e) = fs::write(&target_exe, NOVA_BIN) {
        #[cfg(windows)]
        {
            let _ = Command::new("powershell")
                .args(["-NoProfile", "-Command", &format!("[System.Windows.Forms.MessageBox]::Show('Installation failed: {}', 'Nova Studio Installer')", e)])
                .creation_flags(0x08000000)
                .status();
        }
        return;
    }

    let exe_str = target_exe.to_string_lossy().to_string();

    // 1. Create Shortcuts via PowerShell COM WScript.Shell
    let ps_script = format!(
        r#"$ws = New-Object -ComObject WScript.Shell
$desktop = [System.Environment]::GetFolderPath('Desktop')
$s1 = $ws.CreateShortcut("$desktop\Nova Studio.lnk")
$s1.TargetPath = "{}"
$s1.Description = "Nova Studio — 2027 Spatial Web Browser"
$s1.Save()

$startMenu = [System.Environment]::GetFolderPath('Programs')
$s2 = $ws.CreateShortcut("$startMenu\Nova Studio.lnk")
$s2.TargetPath = "{}"
$s2.Description = "Nova Studio — 2027 Spatial Web Browser"
$s2.Save()
"#,
        exe_str.replace('\\', "\\\\"),
        exe_str.replace('\\', "\\\\")
    );

    #[cfg(windows)]
    {
        let _ = Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", &ps_script])
            .creation_flags(0x08000000)
            .status();

        // 2. Register Windows Default Browser and Add/Remove Programs
        let reg_commands = [
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
            format!(r#"reg add "HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\NovaStudio" /v "DisplayName" /d "Nova Studio" /f"#),
            format!(r#"reg add "HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\NovaStudio" /v "DisplayVersion" /d "{}" /f"#, APP_VERSION),
            format!(r#"reg add "HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\NovaStudio" /v "Publisher" /d "Nova Studio Team" /f"#),
            format!(r#"reg add "HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\NovaStudio" /v "DisplayIcon" /d "\"{}\",0" /f"#, exe_str),
            format!(r#"reg add "HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\NovaStudio" /v "InstallLocation" /d "\"{}\"" /f"#, install_dir.to_string_lossy()),
            format!(r#"reg add "HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\NovaStudio" /v "UninstallString" /d "cmd.exe /C rmdir /S /Q \"{}\"" /f"#, install_dir.to_string_lossy()),
        ];

        for cmd in reg_commands {
            let _ = Command::new("cmd")
                .args(["/C", &cmd])
                .creation_flags(0x08000000)
                .status();
        }

        // 3. Launch Nova Studio immediately
        let _ = Command::new(&target_exe).spawn();
    }
}
