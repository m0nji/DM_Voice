// Tiny i18n loader, exposed as `window.i18n`.
// Loaded as a regular (non-module) <script>; runs before settings.js / waveform.js.
//
// Usage:
//   await window.i18n.initI18n('en');   // fetch & cache strings
//   window.i18n.applyI18n(document);    // replace `data-i18n` attributes
//   window.i18n.t('key.name', { var: 1 });
//
// HTML elements with `data-i18n="key"` get their textContent replaced.
// HTML elements with `data-i18n-attr="attr:key"` get the given attribute set.

(function () {
  let messages = {};

  // Compute the directory of this script so en.json fetch works regardless
  // of which page (settings/ or overlay/) loaded i18n.js.
  const scriptUrl = document.currentScript ? document.currentScript.src : '';
  const baseDir = scriptUrl.substring(0, scriptUrl.lastIndexOf('/'));

  async function initI18n(locale) {
    locale = locale || 'en';
    try {
      const res = await fetch(`${baseDir}/${locale}.json`, { cache: 'no-cache' });
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      messages = await res.json();
    } catch (e) {
      // Leaves HTML fallback text in place; logs for QA visibility.
      console.error('[i18n] failed to load locale', locale, e);
      messages = {};
    }
  }

  function t(key, params) {
    let s = messages[key];
    if (s === undefined) return key;
    if (params) {
      for (const k of Object.keys(params)) {
        s = s.split(`{${k}}`).join(String(params[k]));
      }
    }
    return s;
  }

  function applyI18n(root) {
    root = root || document;
    const els = root.querySelectorAll('[data-i18n]');
    for (let i = 0; i < els.length; i++) {
      const el = els[i];
      el.textContent = t(el.getAttribute('data-i18n'));
    }
    const attrEls = root.querySelectorAll('[data-i18n-attr]');
    for (let i = 0; i < attrEls.length; i++) {
      const spec = attrEls[i].getAttribute('data-i18n-attr');
      const idx = spec.indexOf(':');
      if (idx < 0) continue;
      const attr = spec.substring(0, idx);
      const key = spec.substring(idx + 1);
      attrEls[i].setAttribute(attr, t(key));
    }
  }

  window.i18n = { initI18n, t, applyI18n };
})();
