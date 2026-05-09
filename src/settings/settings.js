const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

// --- Shortcut Recorder ---
const shortcutBox = document.getElementById('shortcut-box');
let listening = false;

invoke('get_config').then(cfg => {
  shortcutBox.textContent = cfg.shortcut;
});

shortcutBox.addEventListener('click', () => {
  listening = true;
  shortcutBox.classList.add('listening');
  shortcutBox.textContent = 'Taste drücken…';
});

// Codes that are pure modifier keys — never used as the primary key
const MODIFIER_CODES = new Set([
  'AltLeft','AltRight','ShiftLeft','ShiftRight',
  'ControlLeft','ControlRight','MetaLeft','MetaRight',
  'CapsLock','NumLock','ScrollLock',
]);

document.addEventListener('keydown', (e) => {
  if (!listening) return;
  e.preventDefault();

  // Skip if only a modifier key was pressed — wait for the actual key
  if (MODIFIER_CODES.has(e.code)) return;

  const mods = [];
  if (e.altKey) mods.push('Alt');
  if (e.ctrlKey) mods.push('Ctrl');
  if (e.metaKey) mods.push('Super');
  if (e.shiftKey) mods.push('Shift');

  // Use e.code (physical key, layout-independent)
  const CODE_MAP = {
    'Space': 'Space', 'Enter': 'Enter', 'Backspace': 'Backspace',
    'Tab': 'Tab', 'Delete': 'Delete', 'Escape': 'Escape', 'Home': 'Home',
    'End': 'End', 'PageUp': 'PageUp', 'PageDown': 'PageDown', 'Insert': 'Insert',
    'ArrowUp': 'ArrowUp', 'ArrowDown': 'ArrowDown',
    'ArrowLeft': 'ArrowLeft', 'ArrowRight': 'ArrowRight',
    'F1':'F1','F2':'F2','F3':'F3','F4':'F4','F5':'F5','F6':'F6',
    'F7':'F7','F8':'F8','F9':'F9','F10':'F10','F11':'F11','F12':'F12',
  };

  let key;
  const code = e.code;
  if (code.startsWith('Key')) {
    key = code.slice(3);      // 'KeyA' → 'A'
  } else if (code.startsWith('Digit')) {
    key = code.slice(5);      // 'Digit1' → '1'
  } else {
    key = CODE_MAP[code] || code;
  }

  // Need at least one modifier for a valid shortcut
  if (mods.length === 0) return;

  const shortcut = [...mods, key].join('+');
  shortcutBox.textContent = shortcut;
  shortcutBox.classList.remove('listening');
  listening = false;
  invoke('set_shortcut', { shortcut });
});

// --- Model List ---
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
      <div class="model-name">${m.name}</div>
      <div class="model-meta">${sizeMb} MB · ${m.quality}</div>
      <div class="progress" id="prog-${m.name}" style="display:none">
        <div class="progress-bar" id="progbar-${m.name}"></div>
      </div>`;

    const btnDiv = document.createElement('div');
    const btn = document.createElement('button');
    if (m.installed) {
      btn.className = 'danger';
      btn.textContent = 'Löschen';
      btn.addEventListener('click', () => deleteModel(m.name, m.filename));
    } else {
      btn.textContent = 'Download';
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

// --- Permissions ---
const PERM_LABELS = {
  granted: 'Erteilt',
  denied: 'Verweigert',
  not_determined: 'Nicht erteilt',
  restricted: 'Eingeschränkt',
  unknown: '–',
};

function renderPermRow(rowId, status, paneName) {
  const row = document.getElementById(rowId);
  if (!row) return;
  const badge = row.querySelector('.perm-status');
  const btn = row.querySelector('button');
  badge.className = 'perm-status ' + status;
  badge.textContent = PERM_LABELS[status] || status;
  // For not_determined → ask app to prompt; otherwise → open System Settings
  btn.textContent = status === 'not_determined' ? 'Anfragen' : 'Öffnen';
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
// Re-poll periodically so the badge updates after the user toggles in System Settings
setInterval(loadPermissions, 2000);

// --- Updates ---
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
  return d.toLocaleString('de-DE', { dateStyle: 'short', timeStyle: 'short' });
}

function renderUpdateState(s) {
  updateVersionEl.textContent = `Version ${s.current_version}`;
  if (s.installing) {
    updateStatusEl.textContent = 'Installation läuft…';
    updateStatusEl.className = 'update-meta';
    updateCheckBtn.disabled = true;
    updateInstallBtn.disabled = true;
    return;
  }
  updateCheckBtn.disabled = false;
  updateInstallBtn.disabled = false;

  if (s.last_error) {
    updateStatusEl.textContent = `Fehler: ${s.last_error}`;
    updateStatusEl.className = 'update-meta update-status error';
    updateNotesEl.style.display = 'none';
    updateInstallLine.style.display = 'none';
    return;
  }

  if (s.latest_version) {
    updateStatusEl.textContent = `Update verfügbar: v${s.latest_version}`;
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
      ? `Aktuell — letzter Check: ${when}`
      : 'Noch nicht geprüft.';
    updateStatusEl.className = 'update-meta';
  }
}

function loadUpdateState() {
  invoke('get_update_state').then(renderUpdateState);
}

updateCheckBtn.addEventListener('click', () => {
  updateCheckBtn.disabled = true;
  updateStatusEl.textContent = 'Prüfe…';
  updateStatusEl.className = 'update-meta';
  invoke('check_for_updates')
    .then(renderUpdateState)
    .catch(err => {
      updateStatusEl.textContent = `Fehler: ${err}`;
      updateStatusEl.className = 'update-meta update-status error';
    })
    .finally(() => { updateCheckBtn.disabled = false; });
});

updateInstallBtn.addEventListener('click', () => {
  updateInstallBtn.disabled = true;
  updateProgress.style.display = 'block';
  updateProgressBar.style.width = '0%';
  updateProgressTxt.textContent = 'Lade…';
  invoke('install_update').catch(err => {
    updateProgressTxt.textContent = `Fehler: ${err}`;
    updateInstallBtn.disabled = false;
  });
});

listen('update-progress', (e) => {
  const { downloaded, total } = e.payload;
  if (total) {
    const pct = (downloaded / total) * 100;
    updateProgressBar.style.width = pct.toFixed(1) + '%';
    updateProgressTxt.textContent =
      `Lade ${(downloaded / 1_000_000).toFixed(1)} / ${(total / 1_000_000).toFixed(1)} MB`;
  } else {
    updateProgressTxt.textContent =
      `Lade ${(downloaded / 1_000_000).toFixed(1)} MB`;
  }
});

listen('update-checked', (e) => {
  renderUpdateState(e.payload);
});

loadUpdateState();
