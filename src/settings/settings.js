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
