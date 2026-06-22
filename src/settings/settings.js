const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const { t, initI18n, applyI18n } = window.i18n;

// --- Typing-speed presets (kept in sync with src-tauri/src/config.rs) ---
const PRESET_CPM = { beginner: 120, average: 200, fast: 300 };
const PRESET_WPM = { beginner: 24, average: 40, fast: 60 };

// Run all UI init after i18n strings are loaded so static elements are localized
// before any dynamic update overwrites them.
(async function boot() {
  await initI18n('en');
  applyI18n();
  initNav();
  initShortcut();
  initSounds();
  initLowercase();
  initPill();
  initHandsfree();
  initTimesaved();
  initVocabulary();
  initSymbols();
  initModels();
  initPermissions();
  initUpdates();
})();

// ─── Sidebar navigation ──────────────────────────────────────────────────────
// Shows exactly one settings category at a time; defaults to "general" on every
// open (no persistence). Independent of the init* functions, which bind by id.
function initNav() {
  const items = document.querySelectorAll('.settings-nav-item');
  const sections = document.querySelectorAll('.settings-content section');
  function show(nav) {
    items.forEach(b => {
      const active = b.dataset.nav === nav;
      b.classList.toggle('active', active);
      if (active) b.setAttribute('aria-current', 'page');
      else b.removeAttribute('aria-current');
    });
    sections.forEach(s => { s.style.display = s.dataset.section === nav ? 'block' : 'none'; });
  }
  items.forEach(b => b.addEventListener('click', () => show(b.dataset.nav)));
  show('general');
}

// ─── Shortcut ──────────────────────────────────────────────────────────────
function initShortcut() {
  const shortcutBox = document.getElementById('shortcut-box');
  let listening = false;

  invoke('get_config').then(cfg => {
    shortcutBox.textContent = cfg.shortcut;
  });

  shortcutBox.addEventListener('click', () => {
    listening = true;
    shortcutBox.classList.add('listening');
    shortcutBox.textContent = t('settings.shortcut.listening');
  });

  const MODIFIER_CODES = new Set([
    'AltLeft', 'AltRight', 'ShiftLeft', 'ShiftRight',
    'ControlLeft', 'ControlRight', 'MetaLeft', 'MetaRight',
    'CapsLock', 'NumLock', 'ScrollLock',
  ]);

  document.addEventListener('keydown', (e) => {
    if (!listening) return;
    e.preventDefault();
    if (MODIFIER_CODES.has(e.code)) return;

    const mods = [];
    if (e.altKey) mods.push('Alt');
    if (e.ctrlKey) mods.push('Ctrl');
    if (e.metaKey) mods.push('Super');
    if (e.shiftKey) mods.push('Shift');

    const CODE_MAP = {
      'Space': 'Space', 'Enter': 'Enter', 'Backspace': 'Backspace',
      'Tab': 'Tab', 'Delete': 'Delete', 'Escape': 'Escape', 'Home': 'Home',
      'End': 'End', 'PageUp': 'PageUp', 'PageDown': 'PageDown', 'Insert': 'Insert',
      'ArrowUp': 'ArrowUp', 'ArrowDown': 'ArrowDown',
      'ArrowLeft': 'ArrowLeft', 'ArrowRight': 'ArrowRight',
      'F1': 'F1', 'F2': 'F2', 'F3': 'F3', 'F4': 'F4', 'F5': 'F5', 'F6': 'F6',
      'F7': 'F7', 'F8': 'F8', 'F9': 'F9', 'F10': 'F10', 'F11': 'F11', 'F12': 'F12',
    };

    let key;
    const code = e.code;
    if (code.startsWith('Key')) key = code.slice(3);
    else if (code.startsWith('Digit')) key = code.slice(5);
    else key = CODE_MAP[code] || code;

    if (mods.length === 0) return;

    const shortcut = [...mods, key].join('+');
    shortcutBox.textContent = shortcut;
    shortcutBox.classList.remove('listening');
    listening = false;
    invoke('set_shortcut', { shortcut });
  });
}

// ─── Sounds toggle ─────────────────────────────────────────────────────────
function initSounds() {
  const soundsToggle = document.getElementById('sounds-toggle');
  invoke('get_config').then(cfg => {
    soundsToggle.checked = cfg.sounds_enabled;
  });
  soundsToggle.addEventListener('change', () => {
    invoke('set_sounds_enabled', { enabled: soundsToggle.checked });
  });
}

// ─── Lowercase output ────────────────────────────────────────────────────────
function initLowercase() {
  const toggle = document.getElementById('lowercase-toggle');
  invoke('get_config').then(cfg => {
    toggle.checked = !!cfg.lowercase_output;
  });
  toggle.addEventListener('change', () => {
    invoke('set_lowercase_output', { enabled: toggle.checked });
  });
}

// ─── Overlay pill (always-visible / draggable) ───────────────────────────────
function initPill() {
  const pillToggle = document.getElementById('pill-toggle');
  invoke('get_config').then(cfg => {
    pillToggle.checked = !!cfg.pill_always_visible;
  });
  pillToggle.addEventListener('change', () => {
    invoke('set_pill_always_visible', { enabled: pillToggle.checked });
  });
}

// ─── Hands-free (wake word) ──────────────────────────────────────────────────
const WAKE_WORDS = ['hey_jarvis', 'alexa', 'hey_mycroft']; // keep in sync with wake_word.rs AVAILABLE_MODELS
function initHandsfree() {
  const toggle = document.getElementById('handsfree-toggle');
  const options = document.getElementById('handsfree-options');
  const wakeSel = document.getElementById('handsfree-wakeword');
  const sensSel = document.getElementById('handsfree-sensitivity');
  const silence = document.getElementById('handsfree-silence');
  const silenceVal = document.getElementById('handsfree-silence-value');

  WAKE_WORDS.forEach(w => {
    const o = document.createElement('option');
    o.value = w;
    o.textContent = w.replace(/_/g, ' ').replace(/\b\w/g, c => c.toUpperCase());
    wakeSel.appendChild(o);
  });

  function renderSilence(ms) {
    silenceVal.textContent = t('settings.handsfree.silenceValue', { secs: (ms / 1000).toFixed(1) });
  }

  invoke('get_config').then(cfg => {
    toggle.checked = !!cfg.wake_word_enabled;
    options.style.display = toggle.checked ? 'block' : 'none';
    wakeSel.value = cfg.wake_word_model || 'hey_jarvis';
    sensSel.value = cfg.wake_word_sensitivity || 'medium';
    silence.value = cfg.silence_timeout_ms || 2000;
    renderSilence(silence.value);
  });

  toggle.addEventListener('change', () => {
    options.style.display = toggle.checked ? 'block' : 'none';
    invoke('set_wake_word_enabled', { enabled: toggle.checked });
  });
  wakeSel.addEventListener('change', () => {
    invoke('set_wake_word_model', { name: wakeSel.value });
  });
  sensSel.addEventListener('change', () => {
    invoke('set_wake_word_sensitivity', { preset: sensSel.value }).catch(console.error);
  });
  silence.addEventListener('input', () => renderSilence(silence.value));
  silence.addEventListener('change', () => {
    invoke('set_silence_timeout', { ms: parseInt(silence.value, 10) }).catch(console.error);
  });
}

// ─── Custom Vocabulary ─────────────────────────────────────────────────────
function initVocabulary() {
  const textarea = document.getElementById('vocab-textarea');
  const statusEl = document.getElementById('vocab-status');
  let lastSavedValue = '';
  let saveTimer = null;

  function currentWords() {
    return textarea.value
      .split('\n')
      .map(s => s.trim())
      .filter(s => s.length > 0);
  }

  function showSaved() {
    statusEl.classList.add('visible');
    clearTimeout(saveTimer);
    saveTimer = setTimeout(() => statusEl.classList.remove('visible'), 1500);
  }

  async function save() {
    const words = currentWords();
    // Normalize on the JS side so we can compare against last-saved without
    // a roundtrip — avoids a save when the user just whitespace-noodled.
    const normalized = words.join('\n');
    if (normalized === lastSavedValue) return;
    try {
      await invoke('set_custom_vocabulary', { words });
      lastSavedValue = normalized;
      showSaved();
    } catch (e) {
      console.error('set_custom_vocabulary failed', e);
    }
  }

  invoke('get_config').then(cfg => {
    const list = Array.isArray(cfg.custom_vocabulary) ? cfg.custom_vocabulary : [];
    textarea.value = list.join('\n');
    lastSavedValue = list.join('\n');
  });

  textarea.addEventListener('blur', save);
  // Cmd/Ctrl+Enter to save explicitly without leaving the field.
  textarea.addEventListener('keydown', (e) => {
    if ((e.metaKey || e.ctrlKey) && e.key === 'Enter') {
      e.preventDefault();
      save();
    }
  });
}

// ─── Symbol replacements ───────────────────────────────────────────────────
function initSymbols() {
  const toggle  = document.getElementById('symbols-toggle');
  const options = document.getElementById('symbols-options');
  const rowsEl  = document.getElementById('symbols-rows');
  const addBtn  = document.getElementById('symbols-add');

  function makeRow(spoken, symbol) {
    const row = document.createElement('div');
    row.className = 'symbols-row';

    const spokenIn = document.createElement('input');
    spokenIn.type = 'text';
    spokenIn.className = 'symbols-spoken';
    spokenIn.spellcheck = false;
    spokenIn.value = spoken || '';

    const symbolIn = document.createElement('input');
    symbolIn.type = 'text';
    symbolIn.className = 'symbols-symbol';
    symbolIn.spellcheck = false;
    symbolIn.value = symbol || '';

    const del = document.createElement('button');
    del.type = 'button';
    del.className = 'symbols-del';
    del.textContent = '✕';
    del.addEventListener('click', () => { row.remove(); save(); });

    spokenIn.addEventListener('blur', save);
    symbolIn.addEventListener('blur', save);

    row.append(spokenIn, symbolIn, del);
    return row;
  }

  function currentItems() {
    return Array.from(rowsEl.querySelectorAll('.symbols-row'))
      .map(row => ({
        spoken: row.querySelector('.symbols-spoken').value.trim(),
        symbol: row.querySelector('.symbols-symbol').value,
      }))
      .filter(it => it.spoken.length > 0 && it.symbol.length > 0);
  }

  async function save() {
    try {
      await invoke('set_symbol_replacements', { items: currentItems() });
    } catch (e) {
      console.error('set_symbol_replacements failed', e);
    }
  }

  invoke('get_config').then(cfg => {
    const list = Array.isArray(cfg.symbol_replacements) ? cfg.symbol_replacements : [];
    rowsEl.replaceChildren(...list.map(it => makeRow(it.spoken, it.symbol)));
    toggle.checked = !!cfg.symbol_replacements_enabled;
    options.style.display = toggle.checked ? 'block' : 'none';
  });

  toggle.addEventListener('change', () => {
    options.style.display = toggle.checked ? 'block' : 'none';
    invoke('set_symbol_replacements_enabled', { enabled: toggle.checked })
      .catch(e => console.error('set_symbol_replacements_enabled failed', e));
  });

  addBtn.addEventListener('click', () => {
    const row = makeRow('', '');
    rowsEl.appendChild(row);
    row.querySelector('.symbols-spoken').focus();
  });
}

// ─── Time Saved ────────────────────────────────────────────────────────────
function initTimesaved() {
  const select = document.getElementById('typing-speed-select');

  function render(stats, preset) {
    const cpm = PRESET_CPM[preset] || PRESET_CPM.average;
    const wpm = PRESET_WPM[preset] || PRESET_WPM.average;
    const minutesEl  = document.getElementById('timesaved-minutes');
    const subtitleEl = document.getElementById('timesaved-subtitle');
    const basisEl    = document.getElementById('timesaved-basis');

    if (!stats || stats.dictation_count === 0) {
      minutesEl.textContent = '0';
      subtitleEl.textContent = t('settings.timesaved.empty');
    } else {
      const typingMin = stats.total_chars / cpm;
      const actualMin = (stats.total_recording_s + stats.total_processing_s) / 60;
      const saved = Math.max(0, typingMin - actualMin);

      if (saved < 1 && stats.dictation_count > 0) {
        minutesEl.textContent = t('settings.timesaved.lessThanMinute');
        minutesEl.style.fontSize = '18px';
      } else {
        minutesEl.textContent = `${Math.round(saved)} ${t('settings.timesaved.minSuffix')}`;
        minutesEl.style.fontSize = '';
      }

      const subtitleKey = stats.dictation_count === 1
        ? 'settings.timesaved.subtitle.one'
        : 'settings.timesaved.subtitle';
      subtitleEl.textContent = t(subtitleKey, {
        month: stats.month_label,
        count: stats.dictation_count,
      });
    }

    basisEl.textContent = t('settings.timesaved.basis', { wpm, cpm });
  }

  async function load() {
    const [stats, cfg] = await Promise.all([
      invoke('get_current_month_stats'),
      invoke('get_config'),
    ]);
    select.value = cfg.typing_speed_preset || 'average';
    render(stats, select.value);
  }

  select.addEventListener('change', async () => {
    const preset = select.value;
    try {
      await invoke('set_typing_speed_preset', { preset });
    } catch (e) {
      console.error('set_typing_speed_preset failed', e);
    }
    const stats = await invoke('get_current_month_stats');
    render(stats, preset);
  });

  load();
  // Refresh occasionally so the panel reflects dictations made while the
  // settings window is open (rare, but cheap).
  setInterval(async () => {
    const stats = await invoke('get_current_month_stats');
    render(stats, select.value);
  }, 5000);
}

// ─── Models ────────────────────────────────────────────────────────────────
function initModels() {
  const modelList = document.getElementById('model-list');

  function renderModels(models) {
    modelList.innerHTML = '';
    models.forEach(m => {
      const row = document.createElement('div');
      row.className = 'model-row';
      row.id = `model-${m.name}`;
      const sizeMb = Math.round(m.size_bytes / 1_000_000);

      const infoDiv = document.createElement('div');
      infoDiv.innerHTML = `
        <div class="model-name"></div>
        <div class="model-meta"></div>
        <div class="progress" id="prog-${m.name}" style="display:none">
          <div class="progress-bar" id="progbar-${m.name}"></div>
        </div>`;
      infoDiv.querySelector('.model-name').textContent = m.name;
      infoDiv.querySelector('.model-meta').textContent = `${sizeMb} MB · ${m.quality}`;

      const btnDiv = document.createElement('div');
      const btn = document.createElement('button');
      if (m.installed) {
        btn.className = 'danger';
        btn.textContent = t('settings.models.delete');
        btn.addEventListener('click', () => deleteModel(m.name, m.filename));
      } else {
        btn.textContent = t('settings.models.download');
        btn.addEventListener('click', () => downloadModel(m.name, m.filename));
      }
      btnDiv.appendChild(btn);

      row.appendChild(infoDiv);
      row.appendChild(btnDiv);
      modelList.appendChild(row);
    });
  }

  function downloadModel(name, filename) {
    document.getElementById(`prog-${name}`).style.display = 'block';
    invoke('download_model', { filename });
  }

  function deleteModel(name, filename) {
    invoke('delete_model', { filename }).then(() => loadModels());
  }

  function loadModels() {
    invoke('list_models').then(renderModels);
  }

  listen('model-download-progress', (e) => {
    const { name, progress } = e.payload;
    const bar = document.getElementById(`progbar-${name}`);
    if (bar) bar.style.width = (progress * 100) + '%';
    if (progress >= 1.0) setTimeout(loadModels, 500);
  });

  loadModels();
}

// ─── Permissions ───────────────────────────────────────────────────────────
function initPermissions() {
  function renderPermRow(rowId, status, paneName) {
    const row = document.getElementById(rowId);
    if (!row) return;
    const badge = row.querySelector('.perm-status');
    const btn = row.querySelector('button');
    badge.className = 'perm-status ' + status;
    badge.textContent = t('settings.permissions.status.' + status) || status;
    btn.textContent = status === 'not_determined'
      ? t('settings.permissions.action.request')
      : t('settings.permissions.action.open');
    btn.onclick = () => {
      if (status === 'not_determined') {
        invoke('request_permissions').then(() => setTimeout(loadPermissions, 500));
      } else {
        invoke('open_privacy_pane', { pane: paneName });
      }
    };
  }

  function loadPermissions() {
    invoke('get_permissions').then(s => {
      renderPermRow('perm-mic', s.microphone, 'Microphone');
      renderPermRow('perm-ax', s.accessibility, 'Accessibility');
    });
  }

  loadPermissions();
  setInterval(loadPermissions, 2000);
}

// ─── Updates ───────────────────────────────────────────────────────────────
function initUpdates() {
  const updateVersionEl   = document.getElementById('update-version');
  const updateStatusEl    = document.getElementById('update-status');
  const updateNotesEl     = document.getElementById('update-notes');
  const updateInstallLine = document.getElementById('update-install-line');
  const updateInstallBtn  = document.getElementById('update-install-btn');
  const updateCheckBtn    = document.getElementById('update-check-btn');
  const updateProgress    = document.getElementById('update-progress');
  const updateProgressBar = document.getElementById('update-progress-bar');
  const updateProgressTxt = document.getElementById('update-progress-text');

  function fmtTime(unix) {
    if (!unix) return null;
    const d = new Date(unix * 1000);
    return d.toLocaleString(undefined, { dateStyle: 'short', timeStyle: 'short' });
  }

  function renderUpdateState(s) {
    updateVersionEl.textContent = t('settings.updates.version', { version: s.current_version });
    if (s.installing) {
      updateStatusEl.textContent = t('settings.updates.installing');
      updateStatusEl.className = 'update-meta';
      updateCheckBtn.disabled = true;
      updateInstallBtn.disabled = true;
      return;
    }
    updateCheckBtn.disabled = false;
    updateInstallBtn.disabled = false;

    if (s.last_error) {
      updateStatusEl.textContent = t('settings.updates.error', { msg: s.last_error });
      updateStatusEl.className = 'update-meta update-status error';
      updateNotesEl.style.display = 'none';
      updateInstallLine.style.display = 'none';
      return;
    }

    if (s.latest_version) {
      updateStatusEl.textContent = t('settings.updates.available', { version: s.latest_version });
      updateStatusEl.className = 'update-meta update-status available';
      if (s.notes && s.notes.trim()) {
        updateNotesEl.textContent = s.notes.trim();
        updateNotesEl.style.display = 'block';
      } else {
        updateNotesEl.style.display = 'none';
      }
      updateInstallLine.style.display = 'flex';
    } else {
      updateInstallLine.style.display = 'none';
      updateNotesEl.style.display = 'none';
      const when = fmtTime(s.last_check_unix);
      updateStatusEl.textContent = when
        ? t('settings.updates.upToDate', { when })
        : t('settings.updates.notChecked');
      updateStatusEl.className = 'update-meta';
    }
  }

  function loadUpdateState() {
    invoke('get_update_state').then(renderUpdateState);
  }

  updateCheckBtn.addEventListener('click', () => {
    updateCheckBtn.disabled = true;
    updateStatusEl.textContent = t('settings.updates.checking');
    updateStatusEl.className = 'update-meta';
    invoke('check_for_updates')
      .then(renderUpdateState)
      .catch(err => {
        updateStatusEl.textContent = t('settings.updates.error', { msg: err });
        updateStatusEl.className = 'update-meta update-status error';
      })
      .finally(() => { updateCheckBtn.disabled = false; });
  });

  updateInstallBtn.addEventListener('click', () => {
    updateInstallBtn.disabled = true;
    updateProgress.style.display = 'block';
    updateProgressBar.style.width = '0%';
    updateProgressTxt.textContent = t('settings.updates.downloadingUnknown', { done: '0.0' });
    invoke('install_update').catch(err => {
      updateProgressTxt.textContent = t('settings.updates.error', { msg: err });
      updateInstallBtn.disabled = false;
    });
  });

  listen('update-progress', (e) => {
    const { downloaded, total } = e.payload;
    const done = (downloaded / 1_000_000).toFixed(1);
    if (total) {
      const pct = (downloaded / total) * 100;
      updateProgressBar.style.width = pct.toFixed(1) + '%';
      updateProgressTxt.textContent = t('settings.updates.downloading', {
        done,
        total: (total / 1_000_000).toFixed(1),
      });
    } else {
      updateProgressTxt.textContent = t('settings.updates.downloadingUnknown', { done });
    }
  });

  listen('update-checked', (e) => {
    renderUpdateState(e.payload);
  });

  loadUpdateState();
}
