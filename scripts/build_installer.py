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

    # 1. Build release binary
    print("🔨 Step 1: Building release browser binary...")
    res = subprocess.run(["cargo", "build", "--release"], shell=True)
    if res.returncode != 0:
        print("❌ cargo build --release failed!")
        sys.exit(res.returncode)

    source_exe = root / "target" / "release" / "my_rust_browser.exe"
    if not source_exe.exists():
        print(f"❌ Binary not found at {source_exe}")
        sys.exit(1)

    print(f"✅ Browser binary built: {source_exe.stat().st_size / (1024*1024):.2f} MB")

    # 2. Compile standalone setup installer
    print("📦 Step 2: Compiling 1-Click Setup Installer (Nova-Studio-Setup.exe)...")
    dist_dir = root / "dist"
    dist_dir.mkdir(parents=True, exist_ok=True)
    setup_exe = dist_dir / "Nova-Studio-Setup.exe"

    rustc_cmd = [
        "rustc",
        "-O",
        "-C", "panic=abort",
        "-C", "lto=fat",
        "-C", "codegen-units=1",
        str(root / "src" / "installer.rs"),
        "-o", str(setup_exe)
    ]
    res_inst = subprocess.run(rustc_cmd)
    if res_inst.returncode != 0:
        print("❌ Failed to compile Nova-Studio-Setup.exe with rustc!")
        sys.exit(res_inst.returncode)

    print(f"✅ Installer compiled: {setup_exe} ({setup_exe.stat().st_size / (1024*1024):.2f} MB)")

    # 3. Create Portable distribution
    portable_exe = dist_dir / "Nova-Studio-v6.0.0-Portable.exe"
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

    # 5. Deploy to Desktops for instant testing
    user_home = Path.home()
    desktop_targets = [
        user_home / "Desktop",
        user_home / "OneDrive" / "Desktop"
    ]
    for d in desktop_targets:
        if d.exists():
            try:
                dest_setup = d / "Nova-Studio-Setup.exe"
                dest_browser = d / "Nova Studio v6.0.exe"
                shutil.copy2(setup_exe, dest_setup)
                shutil.copy2(source_exe, dest_browser)
                print(f"🚀 Deployed to Desktop: {d}")
            except Exception as e:
                print(f"⚠️ Could not copy to {d}: {e}")

    print("\n🎉 Build & Packaging completed successfully!")

if __name__ == "__main__":
    main()
