import subprocess
import time
import socket

exe = r"C:\Users\User\OneDrive\Desktop\Nova Studio v6.0.exe"
proc = subprocess.Popen([exe, "--shadow", "--port", "9225"])
time.sleep(3)

s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
try:
    s.connect(("127.0.0.1", 9225))
    print("✅ [Smoke Test] Successfully connected to Nova Studio v6.0 Agent Server on port 9225!")
    s.sendall(b'{"action":"status"}\n')
    res = s.recv(1024)
    print("✅ [Smoke Test] Response received:", res.decode("utf-8", errors="ignore")[:100])
except Exception as e:
    print("❌ Connection error:", e)
finally:
    s.close()
    proc.terminate()
    try:
        proc.wait(timeout=2)
    except:
        proc.kill()
    print("✅ [Smoke Test] Nova Studio process cleanly terminated")
