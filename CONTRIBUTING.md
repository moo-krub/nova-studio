# Contributing to Nova Studio 🦀✨

Thank you for your interest in contributing to **Nova Studio**! We welcome bug reports, feature suggestions, architectural improvements, and code contributions from engineers around the globe.

---

## 🛠️ Development Setup

### Prerequisites
1. **Rust Toolchain:** Stable Rust (2024 Edition recommended).
   ```bash
   rustup update stable
   ```
2. **Microsoft Edge WebView2 Runtime:** Pre-installed on Windows 10/11.
3. **C++ Build Tools:** Visual Studio Build Tools with the "Desktop development with C++" workload.

### Clone and Run
```bash
# 1. Clone the repository
git clone https://github.com/<your-username>/nova-studio.git
cd nova-studio

# 2. Run in development mode
cargo run

# 3. Build optimized release binary
cargo build --release
```

The compiled standalone binary will be located at:
`target/release/my_rust_browser.exe` (~1.4 MB).

---

## 🏗️ Architecture Overview

- **`src/main.rs`:** The core browser orchestration layer written in Rust. Manages Tao windowing, Wry WebViews, tab lifecycle, session persistence, secure DoH network arguments, and the local AI/MCP automation server.
- **`src/ui.html`:** The 2027 Spatial UI shell (Omnibox, tab strip, Raycast-style Command Palette, window controls, and quick settings).
- **`src/panel.html`:** Side panel drawer managing browsing history, bookmarks, downloads, and AI Copilot extracts.
- **`src/injected.js`:** Content script bridging web pages to the host runtime. Manages zero-latency CSS adblocking, SPA URL tracking, and OAuth / SSO popup coordination.
- **`src/new_tab.html`:** Modern, distraction-free start page.

---

## 📜 Pull Request Guidelines

1. **Keep it Fast & Lean:** Nova Studio is designed for sub-millisecond responsiveness. Avoid adding heavy third-party crates unless strictly necessary.
2. **Preserve Security Invariants:**
   - All dynamic titles, URLs, and filenames rendered into HTML must be properly sanitized with `escapeHtml()`.
   - Never expose untrusted web content to privileged IPC commands.
3. **Test Before Submitting:**
   ```bash
   cargo check --release
   cargo test
   ```

---

## 📄 License
By contributing to Nova Studio, you agree that your contributions will be licensed under the [MIT License](LICENSE).
