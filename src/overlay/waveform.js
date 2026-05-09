const pill = document.getElementById('pill');
const bars = Array.from(document.querySelectorAll('.bar'));
const BASE_HEIGHT = 4;
const MAX_HEIGHT = 24;

let state = 'idle'; // idle | recording | processing | done
let pulseFrame = null;

function setHeights(heights) {
  bars.forEach((b, i) => { b.style.height = heights[i] + 'px'; });
}

function startPulse() {
  let t = 0;
  function tick() {
    t += 0.05;
    const heights = bars.map((_, i) =>
      BASE_HEIGHT + (MAX_HEIGHT - BASE_HEIGHT) * 0.3 *
      (0.5 + 0.5 * Math.sin(t + i * 0.8))
    );
    setHeights(heights);
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
  pill.classList.remove('done');
  if (newState === 'idle') {
    pill.classList.remove('visible');
    setHeights([8, 8, 8, 8, 8]);
  } else if (newState === 'recording') {
    pill.classList.add('visible');
    // heights driven by amplitude events
  } else if (newState === 'processing') {
    pill.classList.add('visible');
    startPulse();
  } else if (newState === 'done') {
    pill.classList.add('visible', 'done');
    setHeights([12, 16, 20, 16, 12]);
    setTimeout(() => setState('idle'), 350);
  }
}

const { event } = window.__TAURI__;

event.listen('amplitude', (e) => {
  if (state !== 'recording') return;
  const amp = Math.min(1.0, e.payload);
  const heights = bars.map((_, i) => {
    const wave = 0.5 + 0.5 * Math.sin(Date.now() / 80 + i * 1.2);
    return BASE_HEIGHT + amp * wave * (MAX_HEIGHT - BASE_HEIGHT);
  });
  setHeights(heights);
});

event.listen('recording-state', (e) => {
  setState(e.payload);
});
