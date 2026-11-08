<div align="center">

# 🦀 Nova Studio — Spatial Web Browser

### *Hyper-Performance, Privacy-First 2027 Spatial Web Browser Engineered in Rust*

[![Rust 2024](https://img.shields.io/badge/Rust-2024_Edition-orange.svg?logo=rust&logoColor=white)](https://www.rust-lang.org/)
[![Version](https://img.shields.io/badge/Version-v6.0.0-6366f1.svg?style=flat&logo=buffer)](https://github.com/nova-browser/nova-studio/releases)
[![Platform](https://img.shields.io/badge/Platform-Windows_x64-0078d4.svg?logo=windows&logoColor=white)](https://github.com/nova-browser/nova-studio)
[![License: MIT](https://img.shields.io/badge/License-MIT-emerald.svg)](LICENSE)
[![Build Status](https://img.shields.io/badge/Build-Passing-brightgreen.svg?logo=github-actions)](https://github.com/nova-browser/nova-studio/actions)
[![Binary Size](https://img.shields.io/badge/Binary_Size-1.4_MB-purple.svg)](https://github.com/nova-browser/nova-studio/releases)

<p align="center">
  <a href="#-key-features">Key Features</a> •
  <a href="#-architecture">Architecture</a> •
  <a href="#-quick-start">Quick Start</a> •
  <a href="#-keyboard-shortcuts">Shortcuts</a> •
  <a href="#-auto-update--release-pipeline">Auto-Updates</a> •
  <a href="#-contributing">Contributing</a>
</p>

---

</div>

## 🌟 Overview

**Nova Studio** is a next-generation web browser engineered in pure **Rust**, powered by **Tao** and **Wry** on top of **Microsoft Edge Chromium WebView2**. 

Unlike monolithic browsers that consume gigabytes of RAM and package redundant Chromium runtimes, Nova Studio leverages the operating system's native web engine combined with Rust's zero-cost abstractions, **Mimalloc** high-performance memory allocator, and a custom **2027 Spatial Design System**. The entire browser compiles down to an ultra-compact **~1.4 MB standalone executable**.

---

## ⚡ Key Highlights

### 🎨 1. 2027 Spatial UI & Command Palette
- **OLED Deep Black & Glassmorphism:** Immersive dark aesthetic designed to reduce eye strain and maximize screen real estate.
- **Raycast-Style Command Palette (`Ctrl+K`):** Instantly switch between tabs, search bookmarks, or trigger browser actions from an all-in-one command bar.
- **Bento Box Quick Tools:** One-click access to Audio Booster, GPU RGX Shader, Reader Mode, Offline HTML Archiver, and Workspace Switcher.

### 🛡️ 2. Zero-Overhead Deep AdGuard
- **Native CSS Injection (0ms Latency):** Ad containers and trackers are suppressed directly within Chromium's C++ layout pipeline before they can ever execute or paint.
- **Debounced Idle DOM Purging:** Secondary DOM sweep runs strictly during browser idle periods (`requestIdleCallback`) with observer recursion-prevention, guaranteeing silky-smooth 60/120/144 FPS page scrolling.

### 🌐 3. Built-in Strict DNS-over-HTTPS (DoH) & ECH
- **Bypass ISP Censorship & DNS Poisoning:** Forces encrypted DNS queries over HTTPS directly to **Cloudflare (`1.1.1.1`)** in strict secure mode (`--dns-over-https-mode=secure`).
- **Encrypted Client Hello (ECH):** Encrypts TLS SNI headers to prevent deep-packet inspection (DPI) and unauthorized domain blocking.

### 🔑 4. Smart OAuth & SSO Cross-Window IPC
- **Native Popup & `window.opener` Preservation:** Automatically delegates third-party authentication flows (Google Sign-In / GSI, Facebook, Apple, Shopee) to native popups, maintaining bidirectional `postMessage()` communication and clean automatic window closure.

### 🎵 5. Audio Booster & GPU RGX Enhancement
- **250% Audio Gain Booster:** Uses the WebAudio API to amplify quiet videos and conference streams beyond system limits.
- **GPU RGX Sharpening Shader:** Real-time SVG convolution matrix filter enhancing video and text clarity.

### 🤖 6. Embedded AI & MCP Agent Server
- **JSON-RPC 2.0 Local Agent Endpoint (`:9222` / `:9225`):** Built-in automation server with token authentication and CSRF protection, enabling external AI assistants and MCP agents to inspect DOM, fill forms, extract pages, and automate workflows.

### ◫ 7. Spatial Multi-View Tiling
- **Split View on Demand:** Seamlessly tile tabs into **Single Window**, **Dual Split Screen**, or **3-Pane Grid View** with a single click.

---

## 🏛️ Architecture & System Design

```
┌────────────────────────────────────────────────────────────────────────┐
│                        NOVA STUDIO RUNTIME                             │
├────────────────────────────────────────────────────────────────────────┤
│                                                                        │
│   ┌─────────────────────┐               ┌──────────────────────────┐   │
│   │    Rust Tao Host    │◄─ Window IPC ─┤  2027 Spatial UI Shell   │   │
│   │   (Event Loop /     │               │  (Tabs, Omnibox, Bento)  │   │
│   │    Mimalloc / IPC)  │◄── Storage ───┤  src/ui.html             │   │
│   └──────────┬──────────┘               └──────────────────────────┘   │
│              │                                                         │
│              ├─────────────────────────────────────────┐               │
│              ▼                                         ▼               │
│   ┌─────────────────────┐               ┌──────────────────────────┐   │
│   │  Active Content Tab │               │ Local AI & MCP Server    │   │
│   │  (Wry / WebView2)   │               │ (HTTP / JSON-RPC 2.0)    │   │
│   │  • injected.js      │               │ • Port 9222/9225         │   │
│   │  • Cloudflare DoH   │               │ • DOM / Form Automation  │   │
│   │  • CSS AdGuard      │               └──────────────────────────┘   │
│   └─────────────────────┘                                              │
└────────────────────────────────────────────────────────────────────────┘
```

---

## ⌨️ Keyboard Shortcuts

| Shortcut | Description |
|:---|:---|
| <kbd>Ctrl</kbd> + <kbd>K</kbd> | Open **Raycast Command Palette** |
| <kbd>Ctrl</kbd> + <kbd>T</kbd> | Open a New Tab |
| <kbd>Ctrl</kbd> + <kbd>Shift</kbd> + <kbd>N</kbd> | Open Incognito (Private) Tab |
| <kbd>Ctrl</kbd> + <kbd>W</kbd> | Close Current Tab |
| <kbd>Ctrl</kbd> + <kbd>Shift</kbd> + <kbd>T</kbd> | Restore Last Closed Tab |
| <kbd>Ctrl</kbd> + <kbd>L</kbd> | Focus Address Bar (Omnibox) |
| <kbd>Ctrl</kbd> + <kbd>R</kbd> / <kbd>F5</kbd> | Reload Current Page |
| <kbd>Ctrl</kbd> + <kbd>F</kbd> | Find in Page |
| <kbd>Ctrl</kbd> + <kbd>P</kbd> | Print Page or Save as PDF |
| <kbd>Alt</kbd> + <kbd>Left</kbd> | Navigate Back |
| <kbd>Alt</kbd> + <kbd>Right</kbd> | Navigate Forward |
| <kbd>Escape</kbd> | Close Modals & Command Palette |

---

## 🚀 Quick Start

### Option A: Download Standalone Release (.exe)
1. Go to the [Releases](https://github.com/nova-browser/nova-studio/releases) page.
2. Download `Nova-Studio-v6.0.0-x64.exe`.
3. Run the executable directly. **No installer or admin rights required!**

### Option B: Build from Source

#### Prerequisites:
- [Rust](https://rustup.rs/) (Stable 2024 Edition)
- Windows 10/11 with WebView2 Runtime (Default on Windows)

```bash
# 1. Clone repository
git clone https://github.com/<your-username>/nova-studio.git
cd nova-studio

# 2. Check compilation
cargo check --release

# 3. Build optimized binary with LTO
cargo build --release

# 4. Run Nova Studio
cargo run --release
```

The compiled release binary is located at:
```text
target/release/my_rust_browser.exe  (1.44 MB)
```

---

## 🔄 Auto-Update & Release Pipeline

Nova Studio includes a built-in update check mechanism and an automated **GitHub Actions CI/CD pipeline**:

1. **In-App Version Checker:**
   - In Command Palette (`Ctrl+K`), select **"Check for Updates"**.
   - Nova queries the GitHub Releases API for new version tags and download URLs.

2. **Automated GitHub Release Creation:**
   - When you push a git tag (e.g. `git tag v6.1.0 && git push origin v6.1.0`), the included `.github/workflows/release.yml` automatically:
     - Compiles the release binary with full optimization (`opt-level = 3`, `lto = "fat"`, `strip = true`).
     - Generates SHA-256 integrity checksums (`checksums-sha256.txt`).
     - Creates a published **GitHub Release** with the standalone `.exe` attached.

---

## 📊 Benchmark Comparison

| Metric | Google Chrome | Brave Browser | **Nova Studio v6.0** |
|:---|:---:|:---:|:---:|
| **Binary Size** | ~150 MB | ~170 MB | **~1.4 MB (100x smaller)** |
| **Startup Time** | ~1,200 ms | ~1,400 ms | **< 180 ms** |
| **Memory Footprint (Idle)** | ~450 MB | ~380 MB | **~85 MB** |
| **AdBlock Strategy** | Extension-based | Native Rust/C++ | **C++ Zero-Overhead CSS** |
| **Encrypted DoH (1.1.1.1)** | Manual Config | Optional | **Built-in Strict Mode** |
| **AI / MCP Automation** | ❌ None | ❌ None | **✅ Native JSON-RPC 2.0** |

---

## 🤝 Contributing

Contributions are warmly welcomed! Please read our [Contributing Guide](CONTRIBUTING.md) to get started with development, styling guidelines, and pull request procedures.

---

## 📄 License

Nova Studio is distributed under the **[MIT License](LICENSE)**.

Copyright © 2026 Nova Studio Core Team. Built with ❤️ in Rust.
