import os
import sys
import shutil
import subprocess
import hashlib
from pathlib import Path

def main():
    root = Path(__file__).resolve().parent.parent
    os.chdir(root)
    print(f"📁 Working directory: {root}")

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
    portable_exe = dist_dir / "Nova-Studio-v6.2.0-Portable.exe"
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

    # 5. Deploy to Desktops and Installation Directory for instant testing
    user_home = Path.home()
    desktop_targets = [
        user_home / "Desktop",
        user_home / "OneDrive" / "Desktop"
    ]
    for d in desktop_targets:
        if d.exists():
            try:
                shutil.copy2(setup_exe, d / "Nova-Studio-Setup.exe")
                shutil.copy2(source_exe, d / "Nova Studio.exe")
                shutil.copy2(source_exe, d / "Nova Studio v6.0.exe")
                shutil.copy2(source_exe, d / "Nova Studio v6.2.0.exe")
                shutil.copy2(root / "sala.html", d / "sala.html")
                print(f"🚀 Deployed to Desktop: {d}")
            except Exception as e:
                print(f"⚠️ Could not copy to {d}: {e}")

    # Also deploy directly to D:\NovaStudio if it exists
    d_install = Path("D:/NovaStudio")
    if d_install.exists():
        try:
            shutil.copy2(source_exe, d_install / "Nova Studio.exe")
            print(f"🚀 Deployed to Installed Path: {d_install / 'Nova Studio.exe'}")
        except Exception as e:
            print(f"⚠️ Could not copy to D:/NovaStudio: {e}")

    print("\n🎉 Build & Packaging completed successfully!")

if __name__ == "__main__":
    main()
