# Nova Browser Fundamentals — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:executing-plans (inline)

**Goal:** เติมระบบพื้นฐานของเบราว์เซอร์ที่ Nova ยังขาด ให้เป็นเบราว์เซอร์เต็มรูปแบบ ครบทุกระบบ ไม่ขาด

**Architecture:** main.rs เพิ่ม 4 คำสั่ง/อีเวนต์ (MoveTab, InternalLoadState, toast ดาวน์โหลด, bookmarks-bar state) + ui.html (ลากแท็บ, คีย์ลัดมาตรฐาน, ล็อก https, แถบรายการโปรด) + panel.html (favicon ประวัติ + bookmarks จัดการได้)

## รายการงาน (8 รายการ)

### T1 ลากจัดเรียงแท็บ (drag reorder) — มาตรฐานเบราว์เซอร์ทุกตัว
- main.rs: enum เพิ่ม `MoveTab { from: usize, to: usize }` — handler: ถอด id จาก tab_order[from] แล้ว insert ที่ to, sync
- ui.html: HTML5 drag events บน .tab (draggable=true, dragstart/dragover/drop) → send MoveTab ด้วย "ตำแหน่งใน tab list ที่มองเห็น" (tab list ของ workspace ปัจจุบัน — ต้อง map ผ่าน id จริง: ส่ง MoveTabById { id, before_id } ง่ายกว่า) → **ใช้ `MoveTabById { id: usize, before_id: Option<usize> }`** — handler: หา index ของ id ใน tab_order, ถอดออก, หา index ของ before_id (None = ท้ายสุด), insert

### T2 สถานะโหลด (loading spinner บนแท็บ) — wry `with_on_page_load_handler`
- main.rs: `InternalLoadState { id: usize, loading: bool }` + proxy_load ต่อแท็บ; handler: set tab.loading (เพิ่ม field `loading: bool` ใน struct Tab + init false) → sync → ui.html render spinner CSS บนแท็บที่ loading=true (keyframes spin บน favicon wrap)

### T3 ดาวน์โหลดเสร็จ → toast บนเชลล์
- main.rs InternalDownloadCompleted handler: `ui_webview.evaluate_script("window.showToast && window.showToast('✅ ...')")`

### T4 คีย์ลัดมาตรฐานทั้งชุด (ui.html keydown)
- Ctrl+Tab / Ctrl+Shift+Tab → วนแท็บ (PrevTab/NextTab — ส่ง SwitchTab ด้วย id เพื่อนบ้านจาก __lastTabs ตามลำดับที่ render)
- Ctrl+1..9 → กระโดดแท็บที่ N
- Ctrl+= / Ctrl+- / Ctrl+0 → Zoom ±0.1 / ResetZoom
- F5 → Reload, F12 → ToggleDevTools (cmd มีอยู่แล้ว)

### T5 กลางคลิก (middle-click) ปิดแท็บ — มาตรฐานสากล
- ui.html tab element: `onauxclick` button===1 → CloseTab

### T6 ล็อก https + zoom badge ใน omnibox
- ui.html: badge ใหม่หลัง shield-badge แสดง 🔒(https)/⚠(http)/ชื่อไฟล์(about) จาก state.activeUrl + zoom badge เมื่อ state.zoom ≠ 1.0 (ต้องเพิ่ม zoom ใน state json ของ main.rs sync — ส่ง `zoomLevel` ของ active tab)

### T7 แถบรายการโปรด (bookmarks bar) — toggle Ctrl+Shift+B จำค่า
- main.rs: struct PersistentData เพิ่ม `#[serde(default)] bookmarks_bar: bool`; คำสั่ง ToggleBookmarksBar; sync ส่ง `bookmarksBar` + bookmarks อยู่แล้ว
- ui.html: แถบใหม่ใต้ nav-tier (แสดงเมื่อ bookmarksBar=true), ปุ่มรายการโปรดคลิก = Navigate, ว่าง = ข้อความเรียบ ๆ

### T8 panel: favicon ในประวัติ + ส่วนจัดการรายการโปรด
- panel.html: เพิ่ม favicon จาก google s2 ให้ hist-row; ส่วน bookmarks ใหม่ในแท็บ "ประวัติ" (หรือแท็บใหม่) แสดง bookmarks (renderPanelData ต้องได้รับ bookmarks — main.rs panel_data เพิ่ม "bookmarks": storage.bookmarks) พร้อมปุ่มลบ → RemoveBookmark { index }

## Verification
- build + clippy (baseline 9) + junk scan + JS balance + contract greps (IDs เดิมครบ + ใหม่: kbdHelp, bmBar)
- Ctrl+Tab/1-9/=/-/0/F5/F12, ลากแท็บ, กลางคลิก, spinner, toast ดาวน์โหลด, ล็อก https, แถบรายการโปรด Ctrl+Shift+B
