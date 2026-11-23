/* Sanser website: language, theme, OS detection and live release links. */
(function () {
  'use strict';

  var REPO = 'D205-VN/Sanser';
  var root = document.documentElement;

  function store(key, value) {
    try {
      if (value === undefined) return localStorage.getItem(key);
      localStorage.setItem(key, value);
    } catch (e) { return null; }
  }

  /* ---------- i18n ---------- */
  var EN = {
    'skip': 'Skip to content',
    'nav.features': 'Features', 'nav.how': 'How it works', 'nav.mobile': 'Mobile',
    'nav.download': 'Download', 'nav.faq': 'FAQ', 'nav.cta': 'Download free',
    'hero.badge': 'New', 'hero.badgeText': 'Version', 'hero.badgeMore': 'tidier screen sharing',
    'hero.title1': 'Your computer.', 'hero.title2': 'Always within reach.',
    'hero.lead': 'Sanser lets you control and view computers remotely with low latency — across Mac, Windows, Android and iPhone. With system audio, group voice and screen sharing to several computers at once.',
    'hero.cta': 'Download Sanser', 'hero.secondary': 'See features', 'hero.illus': 'Illustrative interface',
    'strip.1a': 'Hardware encoding', 'strip.1b': 'H.264 via VideoToolbox &amp; Media Foundation',
    'strip.2a': 'Direct or Relay', 'strip.2b': 'Picks the best route automatically',
    'strip.3a': 'Up to 3 viewers', 'strip.3b': 'Share one screen with several computers',
    'strip.4a': 'Free to download', 'strip.4b': 'Installers on GitHub Releases',
    'feat.eyebrow': 'Features',
    'feat.title': 'Everything you need to work — and play — remotely.',
    'feat.sub': 'Native engines for each operating system handle capture, encoding, audio and input. The app only shows a feature when the machine actually supports it.',
    'f.video.t': 'Smooth, low-latency video',
    'f.video.d': 'Screen capture with ScreenCaptureKit on Mac and Desktop Duplication on Windows, hardware H.264 encoding. On a weak link a Mac host lowers resolution on its own (e.g. 1080p → 720p) and returns when the network recovers — no reconnect needed.',
    'f.route.t': 'Auto · Direct · Relay',
    'f.route.d': 'Direct UDP when possible, UDP relay when not, or end-to-end encrypted WSS/443 when the network blocks UDP. Lost connections retry automatically.',
    'f.audio.t': 'System audio',
    'f.audio.d': 'Hear the remote computer over Opus with packet-loss concealment. Each viewer mutes or unmutes independently.',
    'f.voice.t': 'Group voice, clearer',
    'f.voice.d': 'The host and up to 3 viewers talk live inside the session. RNNoise noise suppression, redundant packets against dropouts, and a buffer that adapts to the network. Microphones are always off by default — you decide when to speak.',
    'f.share.t': 'Share with several computers',
    'f.share.d': 'One encoder, up to 3 viewing computers in the same account. Invite, remove one viewer, or stop everyone with a single button.',
    'f.input.t': 'Keyboard &amp; shortcuts',
    'f.input.d': 'Key forwarding, Command/Ctrl chords between Mac and Windows, Unicode typing. On phones: touchpad, zoom and game control keys.',
    'f.clip.t': 'Clipboard &amp; file transfer',
    'f.clip.d': 'Text clipboard sync when you allow it. Two-way file transfer — the other side must accept before anything is received.',
    'f.sec.t': 'Secure from the start',
    'f.sec.d': 'Authenticated sessions, Argon2id password hashing, short-lived rotating tokens. Secrets live in Keychain / Credential Manager. Windows UAC prompts are supported when you enable that option.',
    'how.eyebrow': 'How it works', 'how.title': 'Three steps to connect.',
    'how.1t': 'Install &amp; sign in', 'how.1d': 'Download Sanser for your computer and phone, and sign in with the same account on every device.',
    'how.2t': 'Go online as host', 'how.2d': 'On the computer you want to control, choose <b>Host → Go online</b>. On a Mac, grant Screen Recording and Accessibility.',
    'how.3t': 'Connect', 'how.3d': 'Pick the computer from the list and press <b>Connect</b> — or <b>Start sharing → Invite</b> to let others watch your screen.',
    'mob.eyebrow': 'On your phone', 'mob.title': 'Your computer, in your pocket.',
    'mob.sub': 'The Sanser app for Android and iPhone turns touch into a touchpad, with zoom, extended keys and game controls. Rotate to landscape for full screen.',
    'mob.c1': 'Tap to click, two fingers to scroll, hold for right-click',
    'mob.c2': 'External keyboard and controller support',
    'mob.c3': 'Dedicated tablet layouts',
    'mob.c4': 'Watch view-only screen shares from a computer',
    'mob.note': 'The mobile app is a preview. Phones do not join group voice yet.',
    'mob.illus': 'Interface design — illustrative data (shown in Vietnamese)',
    'alt.device': 'Choosing how to connect', 'alt.home': 'List of computers',
    'alt.sharing': 'Screen sharing invitation', 'alt.remote': 'Controlling a computer in landscape',
    'new.eyebrow': "What's new in", 'new.title': 'Ten new features and a Vietnamese interface.',
    'new.1t': 'Vietnamese and English', 'new.1d': 'The app starts in Vietnamese; switch to English with the VI | EN button in the title bar.',
    'new.2t': 'Choose a display, hide your screen', 'new.2d': "Switch between the host's monitors. The host can turn on a black privacy screen so people nearby can't see while it is being controlled.",
    'new.3t': 'Security and remote help', 'new.3d': 'A new computer or phone confirms sign-in with a code sent to your email, and one-time support codes help people without an account.',
    'new.4t': 'Chat, pen, recording', 'new.4d': 'Chat and draw during a session, record it to MP4, wake computers with Wake-on-LAN and review connection history.',
    'new.all': 'Read the full release notes',
    'dl.eyebrow': 'Download', 'dl.title': 'Choose your platform.',
    'dl.sub': "Every installer downloads directly from the project's GitHub Releases.",
    'dl.mac.req': 'Apple Silicon (M1 or later)', 'dl.mac.btn': 'Download .dmg', 'dl.mac.zip': 'Or download the .zip',
    'dl.mac.note': 'Not notarized by Apple yet: the first time, right-click Sanser → Open. Then allow Screen Recording and Accessibility in System Settings.',
    'dl.win.req': 'Windows 64-bit (x64)', 'dl.win.btn': 'Download installer', 'dl.win.portable': 'Or the portable version (no install)',
    'dl.win.note': 'The installer updates itself to new versions. If SmartScreen warns you, choose “More info → Run anyway”.',
    'dl.and.req': 'ARM64 · preview', 'dl.and.btn': 'Download .apk',
    'dl.and.note': 'Installs directly from the APK file: allow “Install unknown apps” when asked.',
    'dl.ios.req': 'Unsigned IPA · for technical users', 'dl.ios.btn': 'Download .ipa',
    'dl.ios.note': 'Not on the App Store. You need to sign it with your own Apple Account via AltStore, SideStore or Xcode before installing.',
    'dl.foot': 'Verify integrity:', 'dl.all': 'All versions',
    'faq.eyebrow': 'FAQ', 'faq.title': 'Frequently asked questions.',
    'faq.1q': 'Is Sanser free?',
    'faq.1a': 'You can download and use the releases on GitHub without paying. The source code is not licensed for copying or redistribution.',
    'faq.2q': 'Which devices can control each other?',
    'faq.2a': 'Mac and Windows work as both host and controller, in every direction: Mac ↔ Windows, Mac ↔ Mac, Windows ↔ Windows. Android and iPhone control or view computers.',
    'faq.3q': "What's the difference between Direct and Relay?",
    'faq.3a': 'Direct sends data straight between the two computers over UDP — usually the fastest. Relay goes through an intermediate server when the computers cannot reach each other; if the network blocks UDP, Sanser uses WSS on port 443 with end-to-end encryption. Auto chooses for you.',
    'faq.4q': 'Why does macOS warn me the first time?',
    'faq.4a': "The Mac build is signed with the project's own certificate but not yet notarized by Apple, so Gatekeeper asks again. Right-click the app → Open. Sanser needs Screen &amp; System Audio Recording and Accessibility to share the screen and accept control.",
    'faq.5q': 'Does group voice turn my mic on automatically?',
    'faq.5a': 'No. Microphones are always off by default. You can join to listen only, unmute when you want to speak, or deafen the conversation at any time.',
    'faq.6q': 'How many people can watch at once?',
    'faq.6a': 'Sharing lets up to 3 computers in the same account watch your screen view-only, with system audio and group voice.',
    'faq.7q': 'How do I install on iPhone?',
    'faq.7a': 'Sanser is not on the App Store or TestFlight. The IPA is unsigned; sign it with your own Apple Account via AltStore, SideStore or Xcode. With a free account, Apple requires re-signing every 7 days.',
    'faq.8q': 'How do I update?',
    'faq.8a': 'The desktop app has a signed updater. Update every computer in a session to get the new features.',
    'cta.title': 'Ready to connect?', 'cta.sub': 'Download Sanser for your computer and get started in minutes.', 'cta.btn': 'Download now',
    'foot.tag': 'Your space, anywhere.', 'foot.product': 'Product', 'foot.res': 'Resources',
    'foot.notes': 'Release notes', 'foot.rights': 'All rights reserved.',
    'theme': 'Toggle light/dark theme',
    'badge.rec': 'Recommended for you',
    'cta.dl.mac': 'Download for Mac', 'cta.dl.win': 'Download for Windows',
    'cta.dl.android': 'Download for Android', 'cta.dl.ios': 'Get it for iPhone'
  };
  var VI_EXTRA = {
    'badge.rec': 'Đề xuất cho bạn',
    'cta.dl.mac': 'Tải cho Mac', 'cta.dl.win': 'Tải cho Windows',
    'cta.dl.android': 'Tải cho Android', 'cta.dl.ios': 'Tải cho iPhone',
    'hero.cta': 'Tải Sanser'
  };

  var VI = {};
  var textNodes = document.querySelectorAll('[data-i18n]');
  textNodes.forEach(function (el) { VI[el.getAttribute('data-i18n')] = el.innerHTML; });
  var altNodes = document.querySelectorAll('[data-i18n-alt]');
  altNodes.forEach(function (el) { VI['alt:' + el.getAttribute('data-i18n-alt')] = el.alt; });
  Object.keys(VI_EXTRA).forEach(function (k) { if (!(k in VI)) VI[k] = VI_EXTRA[k]; });
  VI.theme = 'Đổi giao diện sáng/tối';

  var lang = 'vi';
  function t(key) { return lang === 'en' ? (EN[key] || VI[key] || '') : (VI[key] || ''); }

  function setLang(next) {
    lang = next === 'en' ? 'en' : 'vi';
    root.lang = lang;
    textNodes.forEach(function (el) {
      var k = el.getAttribute('data-i18n');
      var v = lang === 'en' ? EN[k] : VI[k];
      if (v !== undefined) el.innerHTML = v;
    });
    altNodes.forEach(function (el) {
      var k = el.getAttribute('data-i18n-alt');
      el.alt = lang === 'en' ? (EN[k] || el.alt) : VI['alt:' + k];
    });
    var btn = document.getElementById('lang-toggle');
    btn.querySelector('.lang-code').textContent = lang === 'en' ? 'VI' : 'EN';
    btn.setAttribute('aria-label', lang === 'en' ? 'Chuyển sang tiếng Việt' : 'Switch to English');
    document.getElementById('theme-toggle').setAttribute('aria-label', t('theme'));
    document.title = lang === 'en' ? 'Sanser — Remote desktop for Mac, Windows & mobile' : 'Sanser — Điều khiển máy tính từ xa';
    applyOS();
    store('sanser-lang', lang);
  }

  document.getElementById('lang-toggle').addEventListener('click', function () {
    setLang(lang === 'en' ? 'vi' : 'en');
  });

  /* ---------- theme ---------- */
  function currentTheme() {
    var set = root.dataset.theme;
    if (set) return set;
    return window.matchMedia && window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
  }
  document.getElementById('theme-toggle').addEventListener('click', function () {
    var next = currentTheme() === 'dark' ? 'light' : 'dark';
    root.dataset.theme = next;
    store('sanser-theme', next);
  });

  /* ---------- OS detection ---------- */
  function detectOS() {
    var ua = navigator.userAgent || '';
    var platform = (navigator.userAgentData && navigator.userAgentData.platform) || navigator.platform || '';
    if (/android/i.test(ua)) return 'android';
    if (/iPhone|iPad|iPod/i.test(ua) || (platform === 'MacIntel' && navigator.maxTouchPoints > 1)) return 'ios';
    if (/Mac/i.test(platform) || /Macintosh/i.test(ua)) return 'mac';
    if (/Win/i.test(platform) || /Windows/i.test(ua)) return 'win';
    return null;
  }
  var os = detectOS();
  var osButton = { mac: 'dl-mac', win: 'dl-win', android: 'dl-android', ios: 'dl-ios' };

  function applyOS() {
    document.querySelectorAll('.dl-card').forEach(function (c) {
      c.classList.remove('recommended');
      c.removeAttribute('data-badge');
    });
    var hero = document.getElementById('hero-download');
    var label = document.getElementById('hero-download-label');
    if (!os) { label.innerHTML = t('hero.cta'); hero.setAttribute('href', '#download'); return; }
    var card = document.querySelector('.dl-card[data-os="' + os + '"]');
    if (card) {
      card.classList.add('recommended');
      card.setAttribute('data-badge', t('badge.rec'));
    }
    label.textContent = t('cta.dl.' + os);
    // Phones and iOS go to the download section (they need instructions); desktops download directly.
    var direct = (os === 'mac' || os === 'win') ? document.getElementById(osButton[os]) : null;
    hero.setAttribute('href', direct ? direct.getAttribute('href') : '#download');
  }

  /* ---------- live release links ---------- */
  var ASSETS = [
    { id: 'dl-mac', re: /^Sanser-macOS-[\d.]+-arm64\.dmg$/i, meta: 'dl-mac-meta' },
    { id: 'dl-mac-zip', re: /^Sanser-macOS-[\d.]+-arm64\.zip$/i },
    { id: 'dl-win', re: /^Sanser-Windows-[\d.]+-x64-setup\.exe$/i, meta: 'dl-win-meta' },
    { id: 'dl-win-portable', re: /^Sanser-Windows-[\d.]+-x64-portable\.exe$/i },
    { id: 'dl-android', re: /^Sanser-Android-[\d.]+-arm64(-preview)?\.apk$/i, meta: 'dl-android-meta' },
    { id: 'dl-ios', re: /^Sanser-iOS-[\d.]+-device-unsigned\.ipa$/i, meta: 'dl-ios-meta' }
  ];

  function mb(bytes) { return (bytes / 1048576).toFixed(1) + ' MB'; }

  function applyReleases(releases) {
    var desktopVersion = null;
    ASSETS.forEach(function (spec) {
      for (var i = 0; i < releases.length; i++) {
        var r = releases[i];
        if (r.draft || r.prerelease) continue;
        var asset = (r.assets || []).filter(function (a) { return spec.re.test(a.name); })[0];
        if (!asset) continue;
        var el = document.getElementById(spec.id);
        if (el) el.setAttribute('href', asset.browser_download_url);
        var version = String(r.tag_name || '').replace(/^v/, '');
        if (spec.meta) {
          var m = document.getElementById(spec.meta);
          if (m) m.textContent = 'v' + version + ' · ' + mb(asset.size);
        }
        if (spec.id === 'dl-mac') {
          desktopVersion = version;
          var sums = (r.assets || []).filter(function (a) { return a.name === 'SHA256SUMS.txt'; })[0];
          if (sums) document.getElementById('dl-sums').setAttribute('href', sums.browser_download_url);
        }
        break;
      }
    });
    if (desktopVersion) {
      document.querySelectorAll('.js-desktop-version').forEach(function (s) { s.textContent = desktopVersion; });
    }
    applyOS();
  }

  function getJSON(url, headers) {
    return fetch(url, { headers: headers || {}, cache: 'no-cache' })
      .then(function (res) { if (!res.ok) throw new Error(res.status); return res.json(); });
  }

  function slimReleases(data) {
    return data.map(function (r) {
      return {
        tag_name: r.tag_name, draft: r.draft, prerelease: r.prerelease,
        assets: (r.assets || []).map(function (a) { return { name: a.name, size: a.size, browser_download_url: a.browser_download_url }; })
      };
    });
  }

  // The links baked into index.html at deploy time are already current.
  // Then check the GitHub API directly, so a release published after the
  // last deploy still shows up. Its 60 requests/hour per network can run
  // out; releases.json (written by the deploy, no quota) covers that case.
  function loadReleases() {
    if (!window.fetch) return;
    getJSON('https://api.github.com/repos/' + REPO + '/releases?per_page=100', { Accept: 'application/vnd.github+json' })
      .then(slimReleases)
      .catch(function () { return getJSON('releases.json'); })
      .then(function (list) { if (list && list.length) applyReleases(list); })
      .catch(function () { /* Baked-in links in the HTML stay in place. */ });
  }

  /* ---------- nav + reveal ---------- */
  var nav = document.querySelector('.nav');
  function onScroll() { nav.classList.toggle('scrolled', window.scrollY > 8); }
  window.addEventListener('scroll', onScroll, { passive: true });
  onScroll();

  if ('IntersectionObserver' in window) {
    var targets = document.querySelectorAll('.section-head, .card, .steps li, .news-item, .dl-card, .faq details, .gallery, .mobile-copy');
    var io = new IntersectionObserver(function (entries) {
      entries.forEach(function (e) {
        if (e.isIntersecting) { e.target.classList.add('in'); io.unobserve(e.target); }
      });
    }, { rootMargin: '0px 0px -8% 0px', threshold: 0.05 });
    targets.forEach(function (el) { el.classList.add('reveal'); io.observe(el); });
  }

  /* ---------- init ---------- */
  var params = new URLSearchParams(location.search);
  // Vietnamese-first: English only when chosen explicitly (toggle or ?lang=en).
  var initial = params.get('lang') || store('sanser-lang') || 'vi';
  setLang(initial);
  loadReleases();
})();
