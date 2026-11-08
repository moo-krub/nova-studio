import shutil
import os

src = r"C:\Users\User\.gemini\antigravity\scratch\my_rust_browser\target\release\my_rust_browser.exe"
destinations = [
    r"C:\Users\User\OneDrive\Desktop\Nova Studio v6.0.exe",
    r"C:\Users\User\OneDrive\Desktop\Nova Hyper v5.2.exe",
    r"C:\Users\User\Desktop\Nova Studio v6.0.exe",
    r"C:\Users\User\Desktop\Nova Hyper v5.2.exe",
]

if not os.path.exists(src):
    print(f"Error: Source not found: {src}")
    exit(1)

file_size = os.path.getsize(src)
print(f"Source binary size: {file_size / (1024*1024):.2f} MB")

for dst in destinations:
    try:
        os.makedirs(os.path.dirname(dst), exist_ok=True)
        shutil.copyfile(src, dst)
        print(f"✅ Deployed successfully to: {dst}")
    except Exception as e:
        print(f"⚠️ Could not copy to {dst}: {e}")
