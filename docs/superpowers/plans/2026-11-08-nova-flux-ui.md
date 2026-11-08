# Nova Flux UI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans (inline) to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Restyle `ui.html`, `new_tab.html`, `panel.html` ด้วย design system "Nova Flux" (แก้วนิ่งพรีเมียมดาร์ก) โดยคงสัญญา IPC/IDs 100%

**Architecture:** แก้ HTML/CSS/JS ฝั่ง frontend ล้วน 3 ไฟล์ ไม่แตะ `main.rs` — ทุกไฟล์คง element IDs, ฟังก์ชันที่ backend เรียกเข้า (`renderBrowserState`, `renderPanelData`, `updateAiReport`, `updateBlockedCount`) และชื่อ IPC cmd เดิมทั้งหมด

**Tech Stack:** HTML/CSS/JS (vanilla), IBM Plex Sans Thai via Google Fonts, Rust `include_str!` (no build step สำหรับ HTML)

## Global Constraints

- **ห้ามแก้:** `src/main.rs`, `src/injected.js`, `Cargo.toml` โดยเด็ดขาด
- **Design tokens ต้องใช้ชุดเดียวกันทั้ง 3 ไฟล์** (task 1 นิยาม, task 2-3 ใช้ตาม)
- Font: `IBM Plex Sans Thai` + `IBM Plex Mono` จาก Google Fonts, fallback `'Segoe UI', sans-serif`
- ฟอนต์โหลดด้วย `<link href="https://fonts.googleapis.com/css2?family=IBM+Plex+Sans+Thai:wght@400;500;600;700&family=IBM+Plex+Mono:wght@500;700&display=swap">`
- Motion: `--ease-spring: cubic-bezier(0.16, 1, 0.3, 1); --ease-snappy: cubic-bezier(0.2, 0, 0, 1);` + `@media (prefers-reduced-motion: reduce) { * { animation: none !important; transition: none !important; } }`
- ไม่มี commit step (Mimosa hook บล็อก commit จากปัญหา pre-existing ในไฟล์อื่น — ผู้ใช้เลือกข้าม)
- UI text ภาษาไทย, comment ภาษาไทยตามสไตล์เดิมของไฟล์

## Shared Design Tokens (นิยามครั้งเดียว — ใช้ทั้ง 3 ไฟล์)

```css
:root {
    --bg-base: #0a0a0f;
    --bg-surface: #101017;
    --bg-elevated: rgba(20, 20, 29, 0.86);
    --bg-glass: rgba(16, 16, 24, 0.72);
    --bg-omnibox: #0b0b11;

    --accent: #6366f1;
    --accent-bright: #818cf8;
    --accent-glow: rgba(99, 102, 241, 0.22);
    --accent-subtle: rgba(99, 102, 241, 0.11);

    --border: rgba(255, 255, 255, 0.07);
    --border-subtle: rgba(255, 255, 255, 0.045);
    --border-highlight: rgba(255, 255, 255, 0.13);
    --inner-specular: inset 0 1px 0 0 rgba(255, 255, 255, 0.06);

    --text-primary: #f2f2f5;
    --text-secondary: #a3a3af;
    --text-muted: #64646f;

    --tab-inactive: #8d8d98;
    --tab-hover-bg: rgba(255, 255, 255, 0.04);
    --tab-active-bg: #17171f;

    --danger: #f43f5e;
    --success: #10b981;
    --warning: #f59e0b;

    --radius-sm: 6px; --radius-md: 10px; --radius-lg: 14px; --radius-full: 999px;

    --ease-spring: cubic-bezier(0.16, 1, 0.3, 1);
    --ease-snappy: cubic-bezier(0.2, 0, 0, 1);

    --font-main: 'IBM Plex Sans Thai', 'Segoe UI', sans-serif;
    --font-mono: 'IBM Plex Mono', ui-monospace, Menlo, monospace;
}
body.theme-gx { /* คงโครงเดิม: accent rose #f43f5e, bg แดงอมม่วงเข้มนุ่มขึ้น */ }
body.theme-matrix { /* คงโครงเดิม: accent green #10b981, bg เขียวเข้มนุ่มขึ้น */ }
```

---

### Task 1: Restyle `src/ui.html` (เชลล์หลัก)

**Files:**
- Modify: `src/ui.html` (restyle ทั้งไฟล์ 1776 บรรทัด — เขียนทับ section `<style>` + แก้จุด HTML/JS ตามรายการ)

**Interfaces:**
- Produces: คงเดิมทุกอย่าง — `renderBrowserState(state)`, `updateBlockedCount(n)`, ทุก element IDs, ทุกฟังก์ชัน, ทุก cmd, FAV_SVGS, mockState + preview fallback (setTimeout 60ms)

**การเปลี่ยนแปลง:**
1. แทนที่ token block ทั้งชุดด้วย Shared Design Tokens ด้านบน (ปรับ theme-gx/theme-matrix ให้นุ่ม: ลด glow ~30%)
2. `<head>` เพิ่ม Google Fonts link; `font-family` ทุกกฎเปลี่ยนเป็น `var(--font-main)`; ตัวเลข badge/monospace ใช้ `var(--font-mono)`
3. Tier 1 สูง 36px (เดิม 38), tab สูง 28px, radius ตาม scale
4. **Badge ซ่อนเมื่อ 0:** ใน `renderBrowserState()` เพิ่มตรรกะ — `closedBadge/histBadge` แสดงเมื่อค่า > 0, เมื่อ 0 ตั้ง `style.display='none'`, > 0 ตั้ง `style.display=''`
5. **ธีม persist:** แก้ `CycleThemePreview()` ให้ส่ง `send({cmd:'SetTheme', theme: theme})` หลังเปลี่ยน class — คง showToast เดิม
6. Noise texture: `<body>` ต่อด้วย `::before` overlay SVG feTurbulence opacity 0.025, `pointer-events:none`, `z-index:999998`
7. `@media (prefers-reduced-motion: reduce)` block ต่อท้าย `<style>`
8. ห้ามแต่: ทุก `id=`, ทุก `onclick=` cmd เดิม, `<script>` logic ส่วนที่ไม่ได้ระบุในข้อ 4-5

**Steps:**
- [ ] แทนที่ `<style>` ด้วยชุดใหม่ตาม tokens (คงทุก selector เดิม: .top-tier, .tab, .omnibox, .cmd-*, .ctx-*, .bento-*, .tools-popover, .nova-toast ฯลฯ)
- [ ] เพิ่ม fonts link + noise overlay + reduced-motion
- [ ] แก้ CycleThemePreview (persist) + badge-zero hiding ใน renderBrowserState
- [ ] ตรวจ contract ด้วย grep (คำสั่งใน Task 4)
- [ ] เปิดในเบราว์เซอร์ดู preview mock: tab/palette/popover/ctx menu/ธีม 3 แบบ

### Task 2: Rebuild `src/new_tab.html` ตามภาษา Flux

**Files:**
- Modify: `src/new_tab.html` (231 บรรทัด — เขียนใหม่ทั้งไฟล์)

**Interfaces:**
- Produces: คงฟังก์ชัน `go(url)`, `searchWeb()`, ID `searchInp` — fallback `window.location.href = url` เมื่อไม่มี `window.ipc` (คงเดิม)

**การเปลี่ยนแปลง:**
1. ใช้ tokens เดียวกับ Task 1 (พื้นหลัง aurora glow นุ่มจากบน: `radial-gradient(ellipse 80% 50% at 50% -10%, rgba(99,102,241,0.16), transparent)` บน `--bg-base`)
2. ตัด badge "NOVA HYPER v5.2 • PERFORMANCE EDITION" — เหลือโลโก้ + นาฬิกา + คำทักทายไทยตามเวลา + วันที่ไทย (`toLocaleDateString('th-TH', { weekday:'long', day:'numeric', month:'long' })`)

```js
function greeting() {
    const h = new Date().getHours();
    if (h < 12) return 'สวัสดีตอนเช้า';
    if (h < 16) return 'สวัสดีตอนสาย';
    if (h < 20) return 'สวัสดีตอนเย็น';
    return 'สวัสดีตอนกลางคืน';
}
function tickClock() {
    const now = new Date();
    document.getElementById('clock').textContent = now.toLocaleTimeString('th-TH', { hour: '2-digit', minute: '2-digit' });
    document.getElementById('greet').textContent = greeting();
    document.getElementById('dateLine').textContent = now.toLocaleDateString('th-TH', { weekday: 'long', day: 'numeric', month: 'long' });
}
setInterval(tickClock, 10000); tickClock();
```

3. แถบค้นหา: glass card, สูง 54px, radius-full, focus ring `0 0 0 3px var(--accent-glow)` + ขอบ accent-bright
4. Speed dial 7 เว็บเดิมครบ (Google, YouTube, GitHub, ChatGPT, Rust, Wikipedia, X) — การ์ด 96px, radius-lg, hairline border, hover: translateY(-4px) + border accent
5. โลโก้: ใช้ base64 PNG เดิม (ขนาด 72px, radius-lg) — ไม่ต้องหาใหม่

**Steps:**
- [ ] เขียนไฟล์ใหม่ (โครง: โลโก้ → นาฬิกา/ทักทาย/วันที่ → ค้นหา → speed dial)
- [ ] เปิดในเบราว์เซอร์ตรวจ: นาฬิกาเดิน, ทักทายถูกช่วงเวลา, ค้นหา Enter ได้, การ์ดคลิกได้

### Task 3: Restyle `src/panel.html` ตามภาษา Flux

**Files:**
- Modify: `src/panel.html` (415 บรรทัด)

**Interfaces:**
- Produces: คง `window.renderPanelData(data)`, `window.updateAiReport(report)`, ทุก element IDs, ทุกฟังก์ชัน, ทุก cmd

**การเปลี่ยนแปลง:**
1. แทน tokens เดิม (#0c0c12/#262638/#312e81 + Segoe UI) ด้วย Shared Design Tokens + var(--font-main)
2. Panel tabs: icon+text, active = pill accent-subtle ทั้งบล็อก (เลิก border-top 2px แบบเก่า)
3. **ปุ่มอันตรายต้องยืนยัน** — แก้ 2 จุด:

```html
<button class="btn danger-btn" onclick="if(confirm('ยืนยันล้างประวัติทั้งหมด?')) send({cmd:'ClearHistory'})">ล้างทั้งหมด</button>
<button class="btn danger-btn" onclick="if(confirm('ยืนยันล้างรายการดาวน์โหลด?')) send({cmd:'ClearDownloads'})">ล้าง</button>
```

4. การ์ด/ปุ่ม/input/hist-row/dl-card restyle ด้วย tokens (hairline border, radius-md, hover เนียน)
5. GPU status card คงข้อความเดิม ปรับโทน success จางลง

**Steps:**
- [ ] Restyle ทั้งไฟล์ + ใส่ confirm 2 จุด
- [ ] เปิดในเบราว์เซอร์ตรวจ switch section + confirm ทำงาน

### Task 4: Verification รวม

**Files:** ไม่แก้ไฟล์ — ตรวจเท่านั้น

**Steps:**
- [ ] **Contract grep (ui.html):** ทุก ID/ฟังก์ชันใน spec ต้องพบ — `grep -c 'id="urlInput"\|id="tabBar"\|id="blkCount"\|id="histBadge"\|id="closedBadge"\|id="adBadge"\|id="gridBtn"\|id="gridBtnLabel"\|id="panelBtn"\|id="reopenBtn"\|id="ws-main"\|id="ws-work"\|id="ws-ai"\|id="cmdPalette"\|id="cmdInput"\|id="cmdList"\|id="toolsPopover"\|id="novaToast"\|id="tabContextMenu"' src/ui.html` = 19, `grep -c 'renderBrowserState\|updateBlockedCount\|__novaToggleCommandPalette\|handlePreviewSimulation\|getFaviconHtml\|escapeHtml\|CycleThemePreview'` ≥ 7
- [ ] **Contract grep (panel.html):** `grep -c 'renderPanelData\|updateAiReport\|switchSection\|renderHistoryList\|filterHistory\|renderDownloadList\|runGoal\|saveNotes' src/panel.html` ≥ 8 และ confirm 2 จุดพบ
- [ ] **Contract grep (new_tab.html):** `grep -c 'function go(\|searchWeb\|searchInp' src/new_tab.html` ≥ 3
- [ ] **IPC cmd ครบ:** grep ชื่อ cmd เดิมทั้งหมด (NewTab, SwitchTab, CloseTab, Navigate, SetTheme, ... ) ทั้ง 3 ไฟล์
- [ ] `cargo +stable build` สำเร็จ (2>&1 tail)
- [ ] `cargo +stable clippy` ไม่มี warning ใหม่
- [ ] สรุปผลให้ผู้ใช้ + แนะนำ `cargo run` ทดสอบจริง

## Self-Review

- **Spec coverage:** tokens/ui/new_tab/panel/persist/confirm/badge-zero/reduced-motion — ครบทุกข้อใน spec (Task 1 ข้อ 1-8, Task 2, Task 3)
- **Placeholder scan:** ไม่มี TBD — จุดที่ย่อไว้ (theme-gx/matrix) มีค่าชัดเจนจากไฟล์เดิมที่แก้เท่านั้น ไม่ใช่ช่องว่าง
- **Type consistency:** ชื่อ ID/ฟังก์ชันตรงกับ spec ทุกตัว (ตรวจซ้ำด้วย grep ใน Task 4)
