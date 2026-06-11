const pill  = document.getElementById('pill');
const bars  = Array.from(document.querySelectorAll('.bar'));
const label = document.getElementById('label');

const BASE = 4;
const MAX  = 26;
// The live amplitude payload is the audio RMS (0..1), which is tiny for speech
// (~0.03–0.15). A linear scale barely moves the bars, so we map it through a
// perceptual sqrt curve (lifts quiet/medium levels) with a strong gain so
// normal speech fills most of the bar height. Silence still collapses to BASE.
// Tune the liveliness here: higher gain = taller bars for the same loudness.
const AMP_GAIN = 2.4;

let state = 'idle';
let pulseFrame = null;
// True while the always-visible toggle is on: pill stays on screen in a dimmed
// "Ready" state when idle, and is draggable.
let pinned = false;

// Kick off i18n load — overlay strings are tiny so we don't block on it.
// Until the JSON resolves, `t()` returns the key, which the fallback
// label text covers visually anyway.
const t = (key) => (window.i18n ? window.i18n.t(key) : key);
if (window.i18n) {
  window.i18n.initI18n('en').then(() => window.i18n.applyI18n());
}

function setHeights(heights) {
  bars.forEach((b, i) => { b.style.height = heights[i] + 'px'; });
}

function startPulse() {
  let t = 0;
  function tick() {
    t += 0.07;
    setHeights(bars.map((_, i) =>
      BASE + (MAX - BASE) * 0.45 * (0.5 + 0.5 * Math.sin(t + i * 0.9))
    ));
    pulseFrame = requestAnimationFrame(tick);
  }
  tick();
}

function stopPulse() {
  if (pulseFrame) { cancelAnimationFrame(pulseFrame); pulseFrame = null; }
}

function readyLabel() {
  const s = t('overlay.ready');
  return s === 'overlay.ready' ? 'Ready' : s;
}

function setPinned(p) {
  pinned = p;
  pill.classList.toggle('pinned', p);
  // Re-render the idle state so it flips between hidden and the "Ready" pill.
  if (state === 'idle') setState('idle');
}

// Report the pill's bounds (CSS px within the window) to Rust. The window is
// wider than the pill and stays click-through; the Rust hover poll uses these
// bounds to accept cursor events only while the cursor is over the pill, so
// the transparent margins don't block the app underneath. offsetWidth/
// offsetLeft ignore the entrance transform, so this yields the pill's resting
// bounds even when measured mid-animation.
function reportHitbox() {
  const w = pill.offsetWidth;
  const h = pill.offsetHeight;
  const x = pill.offsetLeft - w / 2; // left:50% + translateX(-50%)
  const y = pill.offsetTop;
  invoke('overlay_set_hitbox', { x, y, w, h }).catch(() => {});
}

function setState(newState) {
  state = newState;
  stopPulse();
  pill.classList.remove('visible', 'processing', 'done', 'no-speech', 'ready');

  if (newState === 'idle') {
    setHeights([6, 6, 6, 6, 6]);
    // When pinned, stay on screen as a dimmed "Ready" pill instead of hiding.
    if (pinned) {
      pill.classList.add('visible', 'ready');
      label.textContent = readyLabel();
    }
  } else if (newState === 'recording') {
    pill.classList.add('visible');
    label.textContent = t('overlay.recording');
    setHeights([6, 6, 6, 6, 6]);
  } else if (newState === 'processing') {
    pill.classList.add('visible', 'processing');
    label.textContent = t('overlay.processing');
    startPulse();
  } else if (newState === 'done') {
    pill.classList.add('visible', 'done');
    label.textContent = t('overlay.done');
    setHeights([4, 10, 16, 10, 4]);
    setTimeout(() => setState('idle'), 380);
  } else if (newState === 'no-speech') {
    pill.classList.add('visible', 'no-speech');
    label.textContent = t('overlay.no_speech');
    setHeights([6, 6, 6, 6, 6]);
  }

  // The label (and thus the auto-width pill) just changed size.
  reportHitbox();
}

const { event } = window.__TAURI__;
const { invoke } = window.__TAURI__.core;

event.listen('amplitude', (e) => {
  if (state !== 'recording') return;
  const amp = Math.min(1.0, Math.sqrt(Math.max(0, e.payload)) * AMP_GAIN);
  setHeights(bars.map((_, i) => {
    const wave = 0.5 + 0.5 * Math.sin(Date.now() / 80 + i * 1.2);
    return BASE + amp * wave * (MAX - BASE);
  }));
});

event.listen('recording-state', (e) => {
  setState(e.payload);
});

event.listen('overlay-pinned', (e) => {
  setPinned(!!e.payload);
});

// ─── Manual drag (pinned mode) ───────────────────────────────────────────────
// The window is only focusable while pinned (so the recording overlay never
// steals focus mid-dictation), and Tauri's native drag region doesn't move a
// transparent always-on-top window reliably. So we drag it ourselves: track the
// cursor's screen delta and reposition via Rust commands (plain `invoke`, no
// dependency on the JS window API which may not be exposed on the global).
// Pointer capture keeps move events flowing even if the cursor outruns the pill.
let drag = null;
let pendingPos = null;
let rafId = null;

function flushPos() {
  rafId = null;
  if (pendingPos) {
    invoke('overlay_set_position', pendingPos).catch(() => {});
    pendingPos = null;
  }
}

pill.addEventListener('pointerdown', async (e) => {
  if (!pinned || e.button !== 0) return;
  e.preventDefault();
  try { pill.setPointerCapture(e.pointerId); } catch (_) {}
  // Suspend the Rust hover poll for the whole drag — set before the await so
  // the poll can't turn the window click-through while we fetch the anchor.
  invoke('overlay_drag_state', { dragging: true }).catch(() => {});
  try {
    const [wx, wy] = await invoke('overlay_outer_position');
    drag = { mx: e.screenX, my: e.screenY, wx, wy, f: window.devicePixelRatio || 1 };
  } catch (_) {
    drag = null;
    invoke('overlay_drag_state', { dragging: false }).catch(() => {});
  }
});

pill.addEventListener('pointermove', (e) => {
  if (!drag) return;
  // screenX/Y are logical (CSS) px; window position is physical → scale the delta.
  const dx = Math.round((e.screenX - drag.mx) * drag.f);
  const dy = Math.round((e.screenY - drag.my) * drag.f);
  pendingPos = { x: drag.wx + dx, y: drag.wy + dy };
  if (!rafId) rafId = requestAnimationFrame(flushPos);
});

function endDrag(e) {
  if (!drag) return;
  drag = null;
  try { pill.releasePointerCapture(e.pointerId); } catch (_) {}
  invoke('overlay_drag_state', { dragging: false }).catch(() => {});
}
pill.addEventListener('pointerup', endDrag);
pill.addEventListener('pointercancel', endDrag);

// Pick up the initial pinned state directly from config, independent of event
// timing at startup (the Rust-side broadcast may fire before this listener).
invoke('get_config')
  .then((cfg) => setPinned(!!cfg.pill_always_visible))
  .catch(() => {});

// Seed the hitbox right away so the hover poll has bounds even before the
// first state change (e.g. pinned "Ready" pill straight after startup).
reportHitbox();
