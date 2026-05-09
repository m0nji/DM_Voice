const pill  = document.getElementById('pill');
const bars  = Array.from(document.querySelectorAll('.bar'));
const label = document.getElementById('label');

const BASE = 4;
const MAX  = 20;

let state = 'idle';
let pulseFrame = null;

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

function setState(newState) {
  state = newState;
  stopPulse();
  pill.classList.remove('visible', 'processing', 'done');

  if (newState === 'idle') {
    setHeights([6, 6, 6, 6, 6]);
  } else if (newState === 'recording') {
    pill.classList.add('visible');
    label.textContent = 'Aufnahme';
    setHeights([6, 6, 6, 6, 6]);
  } else if (newState === 'processing') {
    pill.classList.add('visible', 'processing');
    label.textContent = 'Transkribiere …';
    startPulse();
  } else if (newState === 'done') {
    pill.classList.add('visible', 'done');
    label.textContent = 'Fertig';
    setHeights([4, 10, 16, 10, 4]);
    setTimeout(() => setState('idle'), 380);
  }
}

const { event } = window.__TAURI__;

event.listen('amplitude', (e) => {
  if (state !== 'recording') return;
  const amp = Math.min(1.0, e.payload);
  setHeights(bars.map((_, i) => {
    const wave = 0.5 + 0.5 * Math.sin(Date.now() / 80 + i * 1.2);
    return BASE + amp * wave * (MAX - BASE);
  }));
});

event.listen('recording-state', (e) => {
  setState(e.payload);
});
