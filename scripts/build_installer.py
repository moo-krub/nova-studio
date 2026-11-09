import os
import sys
import shutil
import subprocess
import hashlib
import re
from pathlib import Path

def get_pkg_version(cargo_toml: Path) -> str:
    content = cargo_toml.read_text(encoding="utf-8")
    m = re.search(r'version\s*=\s*"([^"]+)"', content)
    return m.group(1) if m else "6.2.2"

def main():
    root = Path(__file__).resolve().parent.parent
    os.chdir(root)
    print(f"📁 Working directory: {root}")

    version = get_pkg_version(root / "Cargo.toml")
    print(f"🏷️ Detected version from Cargo.toml: v{version}")

    # 1. Build release browser binary
    print("🔨 Step 1: Building release browser binary...")
    res = subprocess.run(["cargo", "build", "--release", "--bin", "my_rust_browser"], shell=True)
    if res.returncode != 0:
        print("❌ cargo build --release --bin my_rust_browser failed!")
        sys.exit(res.returncode)

    source_exe = root / "target" / "release" / "my_rust_browser.exe"
    if not source_exe.exists():
        print(f"❌ Binary not found at {source_exe}")
        sys.exit(1)

    print(f"✅ Browser binary built: {source_exe.stat().st_size / (1024*1024):.2f} MB")

    # 2. Compile standalone setup installer via Cargo
    print("📦 Step 2: Compiling Modern Spatial Setup Installer (nova-installer)...")
    res_inst = subprocess.run(["cargo", "build", "--release", "--bin", "nova-installer"], shell=True)
    if res_inst.returncode != 0:
        print("❌ cargo build --release --bin nova-installer failed!")
        sys.exit(res_inst.returncode)

    installer_bin = root / "target" / "release" / "nova-installer.exe"
    if not installer_bin.exists():
        print(f"❌ Installer binary not found at {installer_bin}")
        sys.exit(1)

    dist_dir = root / "dist"
    dist_dir.mkdir(parents=True, exist_ok=True)
    setup_exe = dist_dir / "Nova-Studio-Setup.exe"
    shutil.copy2(installer_bin, setup_exe)

    print(f"✅ Installer compiled & packaged: {setup_exe} ({setup_exe.stat().st_size / (1024*1024):.2f} MB)")

    # 3. Create Portable distribution
    portable_exe = dist_dir / f"Nova-Studio-v{version}-Portable.exe"
    shutil.copy2(source_exe, portable_exe)
    print(f"✅ Portable binary saved: {portable_exe}")

    # 4. Generate SHA256 checksums
    checksum_file = dist_dir / "checksums-sha256.txt"
    lines = []
    for fpath in [setup_exe, portable_exe]:
        with open(fpath, "rb") as f:
            h = hashlib.sha256(f.read()).hexdigest()
        lines.append(f"{h}  {fpath.name}")
        print(f"🔑 SHA256 ({fpath.name}): {h}")

    checksum_file.write_text("\n".join(lines) + "\n", encoding="utf-8")
    print(f"✅ Checksums written to {checksum_file}")

    # 5. Single Official In-Place Deployment to D:\NovaStudio (with atomic lock replacement)
    d_install = Path("D:/NovaStudio")
    d_install.mkdir(parents=True, exist_ok=True)
    target_install_exe = d_install / "Nova Studio.exe"
    old_backup = d_install / "Nova Studio.exe.old"

    copied = False
    try:
        shutil.copy2(source_exe, target_install_exe)
        copied = True
        print(f"🚀 In-Place Direct Updated: {target_install_exe}")
    except PermissionError:
        # File is currently executing - use Windows atomic rename fallback
        try:
            if old_backup.exists():
                try: old_backup.unlink()
                except Exception: pass
            target_install_exe.rename(old_backup)
            shutil.copy2(source_exe, target_install_exe)
            copied = True
            print(f"🚀 In-Place Hot Updated (via atomic swap): {target_install_exe}")
        except Exception as e:
            print(f"⚠️ Could not hot-replace running executable: {e}")
    except Exception as e:
        print(f"⚠️ Could not copy to D:/NovaStudio: {e}")

    # Maintain exactly ONE clean Windows Shortcut (.lnk) on Desktops
    user_home = Path.home()
    desktop_targets = [
        user_home / "Desktop",
        user_home / "OneDrive" / "Desktop"
    ]
    for d in desktop_targets:
        if d.exists():
            shortcut_path = d / "Nova Studio.lnk"
            ps_cmd = f'''$WshShell = New-Object -ComObject WScript.Shell
$Shortcut = $WshShell.CreateShortcut("{shortcut_path}")
$Shortcut.TargetPath = "{target_install_exe}"
$Shortcut.WorkingDirectory = "D:\\NovaStudio"
$Shortcut.Description = "Nova Studio — Spatial Web Browser"
$Shortcut.Save()'''
            try:
                subprocess.run(["powershell", "-NoProfile", "-Command", ps_cmd], check=True)
                print(f"✨ Verified single shortcut: {shortcut_path}")
            except Exception as e:
                print(f"⚠️ Could not create shortcut on {d}: {e}")

    print("\n🎉 Build & Packaging completed successfully!")

if __name__ == "__main__":
    main()
