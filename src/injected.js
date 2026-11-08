(function() {
    const __SECRET = "__NOVA_SECRET_PLACEHOLDER__";
    const AD_PATTERNS = [
        'doubleclick.net', 'googlesyndication.com', 'googleadservices.com',
        'adservice.google.com', 'adnxs.com', 'taboola.com', 'outbrain.com',
        'popads.net', 'adsterra.com', 'scorecardresearch.com', 'criteo.com',
        'rubiconproject.com', 'pubmatic.com', 'amazon-adsystem.com'
    ];
    const AD_SELECTORS = [
        'ins.adsbygoogle', '[id^="google_ads_iframe"]', '[id^="div-gpt-ad"]',
        'iframe[src*="doubleclick.net"]', 'iframe[src*="googlesyndication.com"]',
        'iframe[src*="adnxs.com"]', '.ytd-companion-slot-renderer', '#player-ads',
        '.ad-container', '.advertisement', '.ad-box', '.ad-banner',
        '[data-ad-unit]', '[data-ad-slot]', '[id*="-advertisement"]',
        'iframe[src*="adsterra.com"]', 'iframe[src*="popads.net"]',
        '.taboola-ad', '.outbrain-ad', '.criteo-ad'
    ];

    function isAdUrl(url) {
        if (!window.__NOVA_ADBLOCK || !url) return false;
        const lower = String(url).toLowerCase();
        return AD_PATTERNS.some(p => lower.includes(p));
    }

    function reportBlockedAd(count) {
        try {
            window.ipc.postMessage(JSON.stringify({
                cmd: 'InternalAdBlocked',
                count: count || 1,
                secret: __SECRET
            }));
        } catch (e) {}
    }

    const origFetch = window.fetch;
    window.fetch = function(input, init) {
        const url = (typeof input === 'string') ? input : (input && input.url ? input.url : '');
        if (isAdUrl(url)) {
            reportBlockedAd(1);
            return Promise.reject(new Error('Blocked by Nova Deep AdGuard'));
        }
        return origFetch.apply(this, arguments);
    };

    const origOpen = XMLHttpRequest.prototype.open;
    XMLHttpRequest.prototype.open = function(method, url) {
        if (isAdUrl(url)) {
            this.__novaBlocked = true;
            reportBlockedAd(1);
        }
        return origOpen.apply(this, arguments);
    };
    const origSend = XMLHttpRequest.prototype.send;
    XMLHttpRequest.prototype.send = function() {
        if (this.__novaBlocked) return;
        return origSend.apply(this, arguments);
    };

    function injectAdblockCss() {
        if (!window.__NOVA_ADBLOCK) return;
        if (document.getElementById('__nova_adblock_css')) return;
        const style = document.createElement('style');
        style.id = '__nova_adblock_css';
        style.textContent = AD_SELECTORS.join(', ') + ' { display: none !important; visibility: hidden !important; height: 0 !important; width: 0 !important; pointer-events: none !important; }';
        const target = document.head || document.documentElement;
        if (target) target.appendChild(style);
    }

    let purgeTimer = null;
    function schedulePurge() {
        if (!window.__NOVA_ADBLOCK || purgeTimer) return;
        purgeTimer = setTimeout(() => {
            purgeTimer = null;
            if (window.requestIdleCallback) {
                window.requestIdleCallback(() => purgeDomAds(), { timeout: 2000 });
            } else {
                purgeDomAds();
            }
        }, 1200);
    }

    function purgeDomAds() {
        if (!window.__NOVA_ADBLOCK) return;
        injectAdblockCss();
        if (observer) observer.disconnect();
        let removed = 0;
        try {
            AD_SELECTORS.forEach(sel => {
                document.querySelectorAll(sel).forEach(el => { el.remove(); removed++; });
            });
            document.querySelectorAll('iframe, script').forEach(el => {
                if (el.src && isAdUrl(el.src)) { el.remove(); removed++; }
            });
        } catch (e) {}
        if (observer && (document.documentElement || document.body)) {
            observer.observe(document.documentElement || document.body, { childList: true, subtree: true });
        }
        if (removed > 0) reportBlockedAd(removed);
    }

    const observer = new MutationObserver(() => schedulePurge());
    if (document.documentElement) {
        observer.observe(document.documentElement, { childList: true, subtree: true });
        injectAdblockCss();
    } else {
        document.addEventListener('DOMContentLoaded', () => {
            if (document.documentElement) observer.observe(document.documentElement, { childList: true, subtree: true });
            injectAdblockCss();
        });
    }

    try {
        Object.defineProperty(navigator, 'webdriver', { get: () => undefined });
        window.chrome = window.chrome || { runtime: {}, app: {} };
        Object.defineProperty(navigator, 'languages', { get: () => ['th-TH', 'th', 'en-US', 'en'] });
    } catch (e) {}

    const _origWindowOpen = window.open;
    window.open = function(url, target, features) {
        // Delegate to native window.open so popup blocker checks pass and video players don\'t halt
        try {
            if (typeof _origWindowOpen === 'function') {
                return _origWindowOpen.apply(this, arguments);
            }
        } catch (e) {}
        if (url) {
            window.ipc.postMessage(JSON.stringify({ cmd: 'NewTab', url: url, incognito: false, workspace: null, pinned: false }));
        }
        return null;
    };

    // --- SPA Navigation Watcher (YouTube, Facebook, Twitter, Gmail, React router) ---
    function notifyUrlChange() {
        if (window.location.href && window.location.href !== 'about:blank') {
            try {
                window.ipc.postMessage(JSON.stringify({
                    cmd: 'InternalUrlChanged',
                    id: window.__NOVA_TAB_ID || 1,
                    url: window.location.href,
                    secret: __SECRET
                }));
            } catch (e) {}
        }
    }
    const _origPushState = history.pushState;
    history.pushState = function() {
        const ret = _origPushState.apply(this, arguments);
        notifyUrlChange();
        return ret;
    };
    const _origReplaceState = history.replaceState;
    history.replaceState = function() {
        const ret = _origReplaceState.apply(this, arguments);
        notifyUrlChange();
        return ret;
    };
    window.addEventListener('popstate', notifyUrlChange);
    window.addEventListener('hashchange', notifyUrlChange);

    window.addEventListener('DOMContentLoaded', () => {
        purgeDomAds();
        if (window.location.href !== 'about:blank') {
            window.ipc.postMessage(JSON.stringify({
                cmd: 'InternalUrlChanged',
                id: window.__NOVA_TAB_ID || 1,
                url: window.location.href,
                secret: __SECRET
            }));
        }
    });

    window.addEventListener('keydown', (e) => {
        if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'k') {
            e.preventDefault();
            window.__novaToggleCommandPalette();
        } else if ((e.ctrlKey || e.metaKey) && e.shiftKey && e.key.toLowerCase() === 't') {
            e.preventDefault();
            window.ipc.postMessage(JSON.stringify({ cmd: 'ReopenClosedTab' }));
        } else if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'f') {
            e.preventDefault();
            const q = prompt('🔎 ค้นหาคำในหน้าเว็บนี้:');
            if (q) window.find(q);
        } else if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'p') {
            e.preventDefault();
            window.ipc.postMessage(JSON.stringify({ cmd: 'PrintPage' }));
        }
    });

    window.__novaSavePageOffline = function() {
        const htmlContent = '<!DOCTYPE html>\n' + document.documentElement.outerHTML;
        window.ipc.postMessage(JSON.stringify({
            cmd: 'InternalSaveOfflineHtml',
            title: document.title || 'saved_page',
            html: htmlContent,
            secret: __SECRET
        }));
        alert('💾 บันทึกหน้าเว็บนี้ลงในโฟลเดอร์ saved_pages/ เรียบร้อยแล้ว!');
    };

    window.__novaToggleCommandPalette = function() {
        const existing = document.getElementById('__nova-cmd-palette');
        if (existing) { existing.remove(); return; }

        const modal = document.createElement('div');
        modal.id = '__nova-cmd-palette';
        modal.style.cssText = 'position:fixed;inset:0;background:rgba(0,0,0,0.65);backdrop-filter:blur(6px);z-index:2147483647;display:flex;align-items:flex-start;justify-content:center;padding-top:12vh;font-family:sans-serif;';
        modal.innerHTML = `
            <div style="width:520px;background:#13131c;border:1px solid #6366f1;border-radius:16px;padding:16px;box-shadow:0 20px 50px rgba(0,0,0,0.8);color:#fff;">
                <div style="font-size:12px;color:#818cf8;margin-bottom:8px;font-weight:700;">⚡ NOVA COMMAND PALETTE (Ctrl+K)</div>
                <input id="__nova-cmd-input" placeholder="พิมพ์ URL หรือคำค้นหาแล้วกด Enter..." style="width:100%;background:#09090d;border:1px solid #2e2e42;color:#fff;padding:12px;border-radius:10px;font-size:15px;outline:none;margin-bottom:12px;" />
                <div style="display:flex;gap:6px;flex-wrap:wrap;font-size:12px;">
                    <button onclick="window.__novaHighlightElements(true);document.getElementById('__nova-cmd-palette').remove();" style="background:#27273a;color:#fff;border:none;padding:6px 10px;border-radius:6px;cursor:pointer;">🎯 สแกนปุ่ม AI</button>
                    <button onclick="window.__novaTranslateToThai();document.getElementById('__nova-cmd-palette').remove();" style="background:#27273a;color:#fff;border:none;padding:6px 10px;border-radius:6px;cursor:pointer;">🌐 แปลไทย</button>
                    <button onclick="window.__novaSavePageOffline();document.getElementById('__nova-cmd-palette').remove();" style="background:#27273a;color:#fff;border:none;padding:6px 10px;border-radius:6px;cursor:pointer;">💾 เซฟเว็บ HTML</button>
                    <button onclick="document.getElementById('__nova-cmd-palette').remove();" style="background:#7f1d1d;color:#fff;border:none;padding:6px 10px;border-radius:6px;cursor:pointer;margin-left:auto;">ปิด (Esc)</button>
                </div>
            </div>
        `;
        document.body.appendChild(modal);
        const inp = document.getElementById('__nova-cmd-input');
        inp.focus();
        inp.addEventListener('keydown', (ev) => {
            if (ev.key === 'Escape') modal.remove();
            if (ev.key === 'Enter' && inp.value.trim()) {
                window.ipc.postMessage(JSON.stringify({ cmd: 'Navigate', url: inp.value.trim() }));
                modal.remove();
            }
        });
    };

    window.__novaBoostAudio = function(gainValue) {
        document.querySelectorAll('video, audio').forEach(el => {
            try {
                if (!el.__novaAudioCtx) {
                    const AudioCtx = window.AudioContext || window.webkitAudioContext;
                    const ctx = new AudioCtx();
                    const source = ctx.createMediaElementSource(el);
                    const gainNode = ctx.createGain();
                    source.connect(gainNode);
                    gainNode.connect(ctx.destination);
                    el.__novaAudioCtx = ctx;
                    el.__novaGainNode = gainNode;
                }
                if (el.__novaAudioCtx && el.__novaAudioCtx.state === 'suspended') {
                    el.__novaAudioCtx.resume();
                }
                el.__novaGainNode.gain.value = gainValue;
            } catch (e) {}
        });
    };

    window.__novaTranslateToThai = async function() {
        const selected = window.getSelection().toString().trim();
        const textToTranslate = selected || document.body.innerText.replace(/\s+/g, ' ').trim().substring(0, 900);
        if (!textToTranslate) return;

        try {
            const url = 'https://translate.googleapis.com/translate_a/single?client=gtx&sl=auto&tl=th&dt=t&q=' + encodeURIComponent(textToTranslate);
            const res = await fetch(url);
            const data = await res.json();
            const translated = data[0].map(item => item[0]).join('');

            const existing = document.getElementById('__nova-translate-box');
            if (existing) existing.remove();

            const box = document.createElement('div');
            box.id = '__nova-translate-box';
            box.style.cssText = 'position:fixed;bottom:20px;right:20px;width:420px;max-height:320px;overflow-y:auto;background:#13131c;color:#f4f4f5;border:2px solid #6366f1;border-radius:14px;padding:16px;z-index:2147483647;box-shadow:0 15px 40px rgba(0,0,0,0.75);font-family:sans-serif;line-height:1.6;font-size:14px;';
            box.innerHTML = `
                <div style="display:flex;justify-content:space-between;align-items:center;margin-bottom:8px;color:#818cf8;font-weight:700;">
                    <span>🌐 คำแปลภาษาไทย (Nova Translator)</span>
                    <button onclick="document.getElementById('__nova-translate-box').remove()" style="background:#ef4444;color:#fff;border:none;border-radius:6px;padding:2px 8px;cursor:pointer;">✕</button>
                </div>
                <div>${translated}</div>
            `;
            document.body.appendChild(box);
        } catch (e) {}
    };

    window.__novaScrapeData = function() {
        const headings = Array.from(document.querySelectorAll('h1, h2, h3')).map(h => h.innerText.trim()).filter(Boolean).slice(0, 20);
        const tables = Array.from(document.querySelectorAll('table')).slice(0, 5).map((tb, idx) => {
            const rows = Array.from(tb.querySelectorAll('tr')).slice(0, 10).map(tr =>
                Array.from(tr.querySelectorAll('th, td')).map(td => td.innerText.replace(/\s+/g, ' ').trim()).join(' | ')
            );
            return `--- ตารางที่ #${idx + 1} ---\n` + rows.join('\n');
        });
        const links = Array.from(document.querySelectorAll('a[href]')).slice(0, 25).map(a => `- ${a.innerText.trim() || 'Link'}: ${a.href}`);
        const scrapedReport = `📊 [ผลลัพธ์การดูดข้อมูลจากเว็บ]\n🌐 เว็บ: ${document.title}\n\n📌 หัวข้อสำคัญ (${headings.length}):\n${headings.join('\n')}\n\n📋 ตารางข้อมูล (${tables.length}):\n${tables.join('\n\n') || 'ไม่มีตารางในหน้านี้'}\n\n🔗 ลิงก์สำคัญ:\n${links.join('\n')}`;

        window.ipc.postMessage(JSON.stringify({
            cmd: 'InternalPageExtracted',
            report: scrapedReport,
            secret: __SECRET
        }));
    };

    window.__novaClearSiteData = function() {
        try {
            localStorage.clear();
            sessionStorage.clear();
            document.cookie.split(";").forEach(c => {
                document.cookie = c.replace(/^ +/, "").replace(/=.*/, "=;expires=" + new Date().toUTCString() + ";path=/");
            });
            location.reload();
        } catch (e) {}
    };

    window.__novaRunAutopilot = function(goal) {
        const g = goal.trim();
        const lower = g.toLowerCase();
        if (lower.startsWith('ค้นหา') || lower.startsWith('search') || lower.startsWith('พิมพ์')) {
            const query = g.replace(/^(ค้นหาคำว่า|ค้นหา|พิมพ์คำว่า|พิมพ์|search for|search)\s*/i, '').trim();
            const searchInput = document.querySelector('input[type="search"], input[name="q"], input[name="search_query"], textarea[name="q"], input[type="text"], textarea');
            if (searchInput && query) {
                searchInput.focus();
                searchInput.value = query;
                searchInput.dispatchEvent(new Event('input', { bubbles: true }));
                searchInput.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', code: 'Enter', keyCode: 13, bubbles: true }));
                if (searchInput.form) searchInput.form.submit();
                return;
            }
        }
        const targetKeyword = g.replace(/^(คลิกปุ่ม|คลิกที่|คลิก|กดปุ่ม|กด|click)\s*/i, '').trim().toLowerCase();
        const clickables = Array.from(document.querySelectorAll('a, button, input[type="submit"], [role="button"]'));
        const matched = clickables.find(el => (el.innerText || el.value || el.getAttribute('aria-label') || '').toLowerCase().includes(targetKeyword));
        if (matched) {
            matched.scrollIntoView({ behavior: 'smooth', block: 'center' });
            matched.click();
        } else {
            window.location.href = 'https://www.google.com/search?q=' + encodeURIComponent(g);
        }
    };

    window.__novaToggleRgxShader = function() {
        const existing = document.getElementById('__nova-rgx-shader');
        if (existing) { existing.remove(); return; }
        const wrapper = document.createElement('div');
        wrapper.id = '__nova-rgx-shader';
        wrapper.innerHTML = `<svg style="position:fixed;width:0;height:0;pointer-events:none;"><filter id="nova-gpu-sharpen"><feConvolveMatrix order="3" preserveAlpha="true" kernelMatrix="0 -1 0 -1 5 -1 0 -1 0"/></filter></svg><style>video, img { filter: url(#nova-gpu-sharpen) contrast(1.08) saturate(1.12) !important; }</style>`;
        document.body.appendChild(wrapper);
    };

    window.__novaToggleReaderMode = function() {
        const existing = document.getElementById('__nova-reader-overlay');
        if (existing) { existing.remove(); return; }
        const paragraphs = Array.from(document.querySelectorAll('article p, main p, p')).map(p => p.innerText.trim()).filter(t => t.length > 40);
        const overlay = document.createElement('div');
        overlay.id = '__nova-reader-overlay';
        overlay.style.cssText = 'position:fixed;inset:0;background:#121216;color:#e4e4e7;z-index:2147483646;overflow-y:auto;padding:60px 20px;font-family:Georgia,serif;';
        overlay.innerHTML = `<div style="max-width:720px;margin:0 auto;line-height:1.85;font-size:19px;"><button onclick="document.getElementById('__nova-reader-overlay').remove()" style="float:right;background:#4f46e5;color:#fff;border:none;padding:8px 16px;border-radius:8px;cursor:pointer;">✕ ปิดโหมดอ่าน</button><h1 style="color:#fff;font-size:32px;margin-bottom:24px;">${document.title}</h1><hr style="border:0;border-top:1px solid #27272a;margin-bottom:28px;"/>${paragraphs.map(p => `<p style="margin-bottom:20px;color:#d4d4d8;">${p}</p>`).join('')}</div>`;
        document.body.appendChild(overlay);
    };

    window.__novaToggleVoiceRead = function() {
        if (!('speechSynthesis' in window)) return;
        if (window.speechSynthesis.speaking) { window.speechSynthesis.cancel(); return; }
        const text = (window.getSelection().toString().trim()) || (document.body.innerText.replace(/\s+/g, ' ').trim().substring(0, 1500));
        const utter = new SpeechSynthesisUtterance(text);
        utter.lang = /[ก-๙]/.test(text) ? 'th-TH' : 'en-US';
        window.speechSynthesis.speak(utter);
    };

    window.__novaHighlightElements = function(show) {
        document.querySelectorAll('.__nova-som-badge').forEach(el => el.remove());
        if (!show) return 0;
        const elements = Array.from(document.querySelectorAll('a[href], button, input, textarea, select, [role="button"]')).filter(el => {
            const rect = el.getBoundingClientRect();
            return rect.width > 5 && rect.height > 5 && rect.top >= 0 && rect.top <= window.innerHeight;
        });
        let counter = 1;
        elements.forEach(el => {
            const id = counter++;
            el.setAttribute('data-nova-id', String(id));
            const rect = el.getBoundingClientRect();
            const badge = document.createElement('div');
            badge.className = '__nova-som-badge';
            badge.textContent = '#' + id;
            badge.style.cssText = `position:fixed;top:${Math.max(0, rect.top - 8)}px;left:${Math.max(0, rect.left - 8)}px;background:linear-gradient(135deg,#facc15,#f97316);color:#000;font-weight:800;font-size:11px;padding:1px 6px;border-radius:5px;z-index:2147483647;pointer-events:none;border:1px solid #000;`;
            document.body.appendChild(badge);
        });
        return counter - 1;
    };

    function resolveTarget(target) {
        const clean = String(target).replace(/^#/, '').trim();
        if (/^\d+$/.test(clean)) return document.querySelector(`[data-nova-id="${clean}"]`);
        return document.querySelector(target);
    }

    window.__novaClick = function(target) {
        const el = resolveTarget(target);
        if (el) { el.scrollIntoView({ behavior: 'smooth', block: 'center' }); el.click(); return true; }
        return false;
    };

    window.__novaType = function(target, text) {
        const el = resolveTarget(target);
        if (el) {
            el.focus();
            const submitAfter = text.endsWith('\n');
            const val = submitAfter ? text.slice(0, -1) : text;
            
            // React / Vue / Angular native property descriptor setter
            const nativeSetter = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value')?.set
                || Object.getOwnPropertyDescriptor(window.HTMLTextAreaElement.prototype, 'value')?.set;
            if (nativeSetter) {
                nativeSetter.call(el, val);
            } else {
                el.value = val;
            }
            el.dispatchEvent(new Event('input', { bubbles: true }));
            el.dispatchEvent(new Event('change', { bubbles: true }));
            if (submitAfter && el.form) el.form.submit();
            return true;
        }
        return false;
    };

    window.__novaExtractDOM = function() {
        const elements = Array.from(document.querySelectorAll('a[href], button, input, textarea, select, [role="button"]')).filter(el => {
            const r = el.getBoundingClientRect();
            return r.width > 5 && r.height > 5;
        });
        let interactiveList = [];
        elements.slice(0, 60).forEach((el, idx) => {
            const id = idx + 1;
            el.setAttribute('data-nova-id', String(id));
            const tag = el.tagName.toLowerCase();
            const label = (el.innerText || el.placeholder || el.value || '').replace(/\s+/g, ' ').trim().substring(0, 50);
            if (label || tag === 'input') interactiveList.push(`[#${id}] <${tag}> "${label}"`);
        });
        const bodyText = (document.body ? document.body.innerText : '').replace(/\s+/g, ' ').trim().substring(0, 2200);
        const report = `🌐 Title: ${document.title}\n🔗 URL: ${location.href}\n\n🎯 ปุ่มและช่องกรอก (${interactiveList.length} รายการ):\n${interactiveList.join('\n')}\n\n📄 เนื้อหา:\n${bodyText}`;
        window.ipc.postMessage(JSON.stringify({
            cmd: 'InternalPageExtracted',
            report: report,
            secret: __SECRET
        }));
    };

    window.__novaToggleDarkMode = function() {
        const existing = document.getElementById('__nova-dark-style');
        if (existing) { existing.remove(); return; }
        const style = document.createElement('style');
        style.id = '__nova-dark-style';
        style.textContent = `html { filter: invert(0.92) hue-rotate(180deg) !important; background: #111 !important; } img, video, iframe, canvas { filter: invert(1) hue-rotate(180deg) !important; }`;
        document.head.appendChild(style);
    };

    window.__novaTogglePiP = function() {
        const video = document.querySelector('video');
        if (video) {
            if (document.pictureInPictureElement) document.exitPictureInPicture();
            else video.requestPictureInPicture();
        }
    };

    window.__novaBoostVideo = function(rate) {
        document.querySelectorAll('video').forEach(v => { v.playbackRate = rate; });
        const skipBtn = document.querySelector('.ytp-ad-skip-button, .ytp-skip-ad-button, .ytp-ad-skip-button-modern');
        if (skipBtn) skipBtn.click();
    };

    // --- 🔊 Real Tab Muting Engine ---
    window.__novaIsMuted = false;
    window.__novaSetMute = function(muted) {
        window.__novaIsMuted = !!muted;
        document.querySelectorAll('audio, video').forEach(el => {
            el.muted = !!muted;
        });
    };
    window.__novaToggleMute = function() {
        window.__novaSetMute(!window.__novaIsMuted);
    };
    document.addEventListener('play', function(e) {
        if (window.__novaIsMuted && e.target && (e.target.tagName === 'AUDIO' || e.target.tagName === 'VIDEO')) {
            e.target.muted = true;
        }
    }, true);

    // =========================================================================
    // 🛡️ Canvas & Audio Fingerprint Anti-Tracking Shield (Brave/Mullvad Grade)
    // =========================================================================
    try {
        const _origGetImageData = CanvasRenderingContext2D.prototype.getImageData;
        CanvasRenderingContext2D.prototype.getImageData = function(sx, sy, sw, sh) {
            const imgData = _origGetImageData.apply(this, arguments);
            if (imgData && imgData.data && imgData.data.length >= 16) {
                const step = Math.floor(imgData.data.length / 4);
                for (let i = 0; i < 4; i++) {
                    const idx = (i * step) + (i % 3);
                    imgData.data[idx] = (imgData.data[idx] + 1) % 256;
                }
            }
            return imgData;
        };

        const _origToDataURL = HTMLCanvasElement.prototype.toDataURL;
        HTMLCanvasElement.prototype.toDataURL = function() {
            try {
                const ctx = this.getContext('2d');
                if (ctx && this.width > 0 && this.height > 0) {
                    const img = ctx.getImageData(0, 0, Math.min(this.width, 16), Math.min(this.height, 16));
                    if (img && img.data && img.data.length > 0) {
                        img.data[0] = (img.data[0] + 1) % 256;
                        ctx.putImageData(img, 0, 0);
                    }
                }
            } catch (e) {}
            return _origToDataURL.apply(this, arguments);
        };

        if (window.AudioBuffer && AudioBuffer.prototype.getChannelData) {
            const _origGetChannelData = AudioBuffer.prototype.getChannelData;
            AudioBuffer.prototype.getChannelData = function() {
                const channelData = _origGetChannelData.apply(this, arguments);
                if (channelData && channelData.length > 10) {
                    channelData[5] += 0.0000001 * (Math.random() > 0.5 ? 1 : -1);
                }
                return channelData;
            };
        }
    } catch (e) {}
})();
