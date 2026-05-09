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

document.addEventListener('keydown', (e) => {
  if (!listening) return;
  e.preventDefault();
  const mods = [];
  if (e.altKey) mods.push('Alt');
  if (e.ctrlKey) mods.push('Ctrl');
  if (e.metaKey) mods.push('Super');
  if (e.shiftKey) mods.push('Shift');
  const key = e.key.toUpperCase();
  if (['ALT', 'CONTROL', 'META', 'SHIFT'].includes(key)) return;
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
    row.innerHTML = `
      <div>
        <div class="model-name">${m.name}</div>
        <div class="model-meta">${sizeMb} MB · ${m.quality}</div>
        <div class="progress" id="prog-${m.name}" style="display:none">
          <div class="progress-bar" id="progbar-${m.name}"></div>
        </div>
      </div>
      <div>
        ${m.installed
          ? `<button class="danger" onclick="deleteModel('${m.name}','${m.filename}')">Löschen</button>`
          : `<button onclick="downloadModel('${m.name}','${m.filename}')">Download</button>`
        }
      </div>`;
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
