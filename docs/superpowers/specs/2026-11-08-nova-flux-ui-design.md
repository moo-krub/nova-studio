# Nova Flux — UI/UX Modernization Design (2027 Edition)

**วันที่:** 2026-11-08
**สถานะ:** อนุมัติแล้วโดยผู้ใช้ (ผู้ใช้เลือกแนว "Nova Flux — แก้วนิ่งพรีเมียม")

---

## เป้าหมาย

ยกระดับหน้าตาและประสบการณ์ใช้งาน Nova Browser ให้ทันสมัยระดับ 2027 — สวย ใช้งานง่าย
และ**เงียบสงบ** (ไม่รก) — โดยไม่แตะ backend (`main.rs`) และไม่แตกสัญญา IPC เดิมแม้แต่บรรทัดเดียว

## ปัญหาของ UI ปัจจุบัน

1. 3 ไฟล์ UI (`ui.html`, `new_tab.html`, `panel.html`) ใช้ design language ต่างกัน 3 แบบ — ดูเหมือนแอปคนละแอป
2. ปุ่ม/badge/ข้อความตลอด (เช่น "NOVA HYPER v5.2 • PERFORMANCE EDITION") ทำให้จอรก
3. ฟอนต์ fallback (Inter/Segoe UI) ทำให้ตัวอักษรไทยไม่สวยเท่าที่ควร
4. ธีมที่สลับ (CycleThemePreview) ไม่ persist — backend มีคำสั่ง `SetTheme` แต่ UI ไม่ได้เรียก

## Design System "Nova Flux"

| ด้าน | ข้อตกลง |
|---|---|
| Typography | **IBM Plex Sans Thai** (ไทย+ละติน) น้ำหนัก 400/500/600/700 + **IBM Plex Mono** สำหรับตัวเลข/badge — โหลดจาก Google Fonts, fallback `Segoe UI, sans-serif` เมื่อออฟไลน์ |
| สี | ธีม 3 แบบเดิม (aurora/gx/matrix) ผ่าน CSS variables — ปรับค่าให้นุ่มขึ้น: ลด glow, เพิ่ม depth จาก layer จริงแทนเงาประดับ |
| Depth | 3 ระดับ elevation: base → surface → elevated (popover) — glass `backdrop-filter: blur+saturate`, ขอบ hairline 1px, specular inset บางๆ |
| Radii | สเกล 6/10/14/full |
| Motion | spring `cubic-bezier(0.16, 1, 0.3, 1)` 150–220ms + เคารพ `prefers-reduced-motion` |
| กติกาความรก | badge/ตัวเลขแสดงเมื่อค่า > 0; ปุ่มรอง icon ล้วน + tooltip; ฟีเจอร์เสริมอยู่ใน popover/palette |

## การเปลี่ยนแปลงรายไฟล์

### 1. `src/ui.html` (เชลล์หลัก — ปรับโฉม คงโครง)

- Tier 1 (36px): โลโก้ + workspace switcher + tab bar + ขวา (กู้คืน + Zen, icon ล้วน)
- Tier 2 (44px): ← → ↻ + omnibox กลาง (shield pill, input, แปล/บุ๊กมาร์ก/บูสต์/⌘K) + ขวา (split view, AI Copilot)
- Restyle ทุก component ด้วย tokens ใหม่: tab, omnibox, tools popover (bento), command palette, context menu, toast
- ตัวเลข badge (closedBadge/histBadge) ซ่อนเมื่อ 0
- `CycleThemePreview()` เพิ่มการส่ง `SetTheme { theme }` ไป backend เพื่อ persist

### 2. `src/new_tab.html` (สร้างใหม่ตามภาษา Flux)

- ตัดข้อความ "NOVA HYPER v5.2 • PERFORMANCE EDITION" ออก
- ใหม่: นาฬิกา + คำทักทายตามเวลาไทย (เช้า/สาย/บ่าย/ค่ำ) + วันที่
- แถบค้นหาแบบ Flux (glass + focus ring accent) + speed dial 7 เว็บเดิม (Google, YouTube, GitHub, ChatGPT, Rust, Wikipedia, X) การ์ด glass hover-lift
- พื้นหลังแสง aurora นุ่มจากด้านบน + noise texture จางๆ

### 3. `src/panel.html` (ปรับโฉม คงโครง 4 sections)

- Restyle ทุกส่วนด้วย tokens Flux: panel tabs, cards, ปุ่ม, input, history rows, download cards
- ปุ่มอันตราย (`ClearHistory`, `ClearDownloads`) เพิ่ม `confirm()` ก่อนส่งคำสั่ง
- GPU status card คงไว้ (ปรับโทนให้เข้าธีม)

## สัญญาที่ต้องคงไว้ 100% (Compatibility Contract)

### `ui.html`
- **ฟังก์ชันที่ backend เรียกเข้า:** `window.renderBrowserState(state)`, `window.updateBlockedCount(n)`
- **ฟิลด์ state ที่ต้องรองรับ:** `tabs[], workspace, activeUrl, bookmarks, adblock, blockedCount, historyCount, downloadCount, closedCount, tilingMode, panelOpen, zenMode, alwaysOnTop, theme`
- **Element IDs:** `urlInput, tabBar, blkCount, histBadge, closedBadge, adBadge, gridBtn, gridBtnLabel, panelBtn, reopenBtn, ws-main, ws-work, ws-ai, cmdPalette, cmdInput, cmdList, toolsPopover, boostPopoverBtn, novaToast, tabContextMenu, ctxPin, ctxMute, ctxHibernate, ctxReload, ctxCopyUrl, ctxClose, ctxCloseOthers`
- **ฟังก์ชันภายใน:** `send, navigate, renderBrowserState, updateBlockedCount, openCommandPalette, closeCommandPalette, toggleToolsPopover, closeToolsPopover, CycleThemePreview, showToast, getFaviconHtml, escapeHtml, showTabContextMenu, handlePreviewSimulation, mockState, window.__novaToggleCommandPalette`
- **IPC:** ส่งผ่าน `window.ipc.postMessage` ด้วยชื่อ cmd เดิมทั้งหมด (ตาม enum `UiCommand` ใน main.rs)
- Preview fallback (mock state + `setTimeout 60ms`) คงไว้

### `panel.html`
- **ฟังก์ชันที่ backend เรียกเข้า:** `window.renderPanelData({history, downloads, notes})`, `window.updateAiReport(report)`
- **Element IDs:** `tab-btn-hist, tab-btn-dl, tab-btn-ai, tab-btn-notes, sec-hist, sec-dl, sec-ai, sec-notes, histCount, histSearch, histList, dlCount, dlList, goalInput, aiReport, notesArea`
- **ฟังก์ชันภายใน:** `send, switchSection, runGoal, saveNotes, escapeHtml, formatTime, renderHistoryList, filterHistory, renderDownloadList`
- **IPC ที่ส่ง:** `ClearHistory, DeleteHistoryItem, Navigate, OpenDownloadsFolder, ClearDownloads, OpenDownloadedFile, SaveNotes, AiAutopilotGoal, ScrapePageData, AiHighlightElements, RequestPageExtract`

### `new_tab.html`
- **ฟังก์ชัน:** `go(url), searchWeb()`, ID `searchInp`
- โหลดใน content webview (ไม่มี `window.ipc`) → fallback `window.location.href = url` คงเดิม

## สิ่งที่ไม่ทำ (Out of Scope)

- ไม่แก้ `main.rs`, `injected.js`, Cargo.toml หรือ IPC ใดๆ
- ไม่แก้บั๊กจาก audit รอบก่อน (XSS, non-atomic save ฯลฯ — แยกเป็นงานอื่น)
- ไม่เพิ่มฟีเจอร์ใหม่ มีแต่ restyle + UX fix เล็ก 3 จุดตามด้านบน
- ไม่ทำโหมดไลต์ (คงดาร์กทั้งแอปตามแนวที่เลือก)

## การตรวจสอบ (Verification)

1. `cargo +stable build` สำเร็จ (ไฟล์ถูก `include_str!` เข้า binary — ถ้า HTML พัง syntax จะ build ไม่ผ่านเฉพาะเมื่อ Rust พัง; HTML ต้องเปิดตรวจเอง)
2. เปิด `ui.html` ในเบราว์เซอร์ — ตรวจ mock preview: tabs, palette (Ctrl+K), popover, context menu, สลับธีม 3 แบบ
3. เปิด `new_tab.html` — ตรวจนาฬิกา/คำทักทาย/ค้นหา/speed dial
4. เปิด `panel.html` — ตรวจ switch section, ปุ่มยืนยันก่อนลบ
5. `cargo +stable clippy` ไม่มี warning ใหม่
6. เรียกใช้งานจริง (`cargo run`) — แท็บ/omnibox/panel/ธีม persist ทำงานกับ backend จริง
