'use strict';

const { invoke, convertFileSrc } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const $ = (id) => document.getElementById(id);

const C = {
  graphite: '#121315', carbon: '#1A1C1F', line: '#2B2D31', steel: '#5C5F66',
  ash: '#7F838B', silver: '#9A9DA3', text: '#E8E9EB', white: '#FFFFFF',
  yellow: '#FFE500', error: '#FF4D3D',
};
const MONO = '500 10px "JetBrains Mono", ui-monospace, monospace';
const RULER = 26;

const DEFAULTS = {
  targetAuto: true, target: -9, strength: 100, window: 45, smooth: 10,
  boost: 6, cut: 12, limiter: true, ceiling: -1, fadeIn: 2, fadeOut: 6, curve: 'smooth',
};

const S = {
  a: null,
  plan: null,
  p: { ...DEFAULTS },
  format: 'wav24',
  start: 0,
  end: 0,
  view: { t0: 0, t1: 1 },
  playhead: 0,
  ab: 'proc',
  hoverX: null,
  lastExport: null,
};

/* ---------- formatting ---------- */

const num = (v, d = 1) =>
  (v < 0 ? '−' : '') + Math.abs(v).toLocaleString('de-DE', { minimumFractionDigits: d, maximumFractionDigits: d });
const signed = (v, d = 1) => (Math.abs(v) < 0.05 ? '±' : v > 0 ? '+' : '−') + num(Math.abs(v), d);

function fmtTime(s, dec = 1) {
  const f = 10 ** dec;
  s = Math.round(Math.max(0, s) * f) / f;
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = (s % 60).toFixed(dec).padStart(dec ? dec + 3 : 2, '0');
  return h ? `${h}:${String(m).padStart(2, '0')}:${sec}` : `${m}:${sec}`;
}

function parseTime(str) {
  const parts = str.trim().replace(',', '.').split(':');
  if (!parts.length || parts.length > 3 || parts.some((p) => p === '' || isNaN(+p))) return NaN;
  return parts.reduce((acc, p) => acc * 60 + +p, 0);
}

const basename = (p) => p.split(/[\\/]/).pop();

/* ---------- persistence (per-viewer convenience only) ---------- */

function loadSettings() {
  try {
    const s = JSON.parse(localStorage.getItem('lufsy.settings') || '{}');
    if (s.p) S.p = { ...DEFAULTS, ...s.p };
    if (s.format) S.format = s.format;
  } catch (_) { /* storage unavailable */ }
}
function saveSettings() {
  try { localStorage.setItem('lufsy.settings', JSON.stringify({ p: S.p, format: S.format })); } catch (_) {}
}

/* ---------- toast / busy ---------- */

let toastTimer;
function toast(msg, ok = false) {
  const t = $('toast');
  t.textContent = String(msg);
  t.classList.toggle('ok', ok);
  t.hidden = false;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => (t.hidden = true), ok ? 3000 : 6000);
}

function showBusy(label, file) {
  $('busyLabel').textContent = label;
  $('busyFile').textContent = file || '';
  setBusy(0);
  $('busy').hidden = false;
}
function setBusy(v) {
  $('busyPct').textContent = Math.round(v * 100);
  $('busyBar').style.width = `${v * 100}%`;
}
const hideBusy = () => ($('busy').hidden = true);

listen('progress', (e) => setBusy(e.payload.value));
$('cancelBtn').addEventListener('click', () => invoke('cancel'));

/* ---------- controls ---------- */

const ctls = {};

function buildControls() {
  document.querySelectorAll('.ctl').forEach((el) => {
    const k = el.dataset.k;
    const min = +el.dataset.min, max = +el.dataset.max, step = +el.dataset.step;
    const dec = +el.dataset.dec, unit = el.dataset.unit;
    el.innerHTML = `
      <div class="ctl-head"><label for="r-${k}">${el.dataset.label || 'Ziel'}</label><span class="ctl-val" title="Klicken zum Eintippen"></span></div>
      <input type="range" id="r-${k}" min="${min}" max="${max}" step="${step}">
      ${el.dataset.hint ? `<p class="ctl-hint">${el.dataset.hint}</p>` : ''}`;
    const range = el.querySelector('input');
    const val = el.querySelector('.ctl-val');

    const show = () => {
      const v = S.p[k];
      range.value = v;
      range.style.setProperty('--p', `${((v - min) / (max - min)) * 100}%`);
      val.textContent = `${k === 'boost' ? '+' : k === 'cut' ? '−' : ''}${num(v, dec)} ${unit}`;
    };
    const set = (v) => {
      v = Math.min(max, Math.max(min, Math.round(v / step) * step));
      S.p[k] = +v.toFixed(3);
      show();
      saveSettings();
      schedulePlan();
    };
    range.addEventListener('input', () => set(+range.value));
    range.addEventListener('dblclick', () => set(DEFAULTS[k]));
    val.addEventListener('click', () => {
      const inp = document.createElement('input');
      inp.value = String(S.p[k]).replace('.', ',');
      val.replaceChildren(inp);
      inp.focus();
      inp.select();
      const done = (commit) => {
        const v = parseFloat(inp.value.replace(',', '.').replace('−', '-'));
        if (commit && !isNaN(v)) set(v);
        show();
      };
      inp.addEventListener('keydown', (e) => {
        if (e.key === 'Enter') done(true);
        if (e.key === 'Escape') done(false);
        e.stopPropagation();
      });
      inp.addEventListener('blur', () => done(true));
    });
    ctls[k] = { el, show };
    show();
  });
}

function segment(id, get, set) {
  const seg = $(id);
  const sync = () => seg.querySelectorAll('button').forEach((b) => b.classList.toggle('on', b.dataset.v === get()));
  seg.addEventListener('click', (e) => {
    const b = e.target.closest('button');
    if (!b) return;
    set(b.dataset.v);
    sync();
  });
  sync();
  return sync;
}

function syncEnabled() {
  ctls.target.el.classList.toggle('disabled', S.p.targetAuto);
  ctls.ceiling.el.classList.toggle('disabled', !S.p.limiter);
  const sw = $('limSwitch');
  sw.classList.toggle('on', S.p.limiter);
  sw.setAttribute('aria-checked', S.p.limiter);
}

/* ---------- planning ---------- */

function params() {
  const p = S.p;
  return {
    startS: S.start, endS: S.end,
    targetLufs: p.targetAuto ? null : p.target,
    strength: p.strength / 100, windowS: p.window, smoothS: p.smooth,
    maxBoostDb: p.boost, maxCutDb: p.cut,
    limiter: p.limiter, ceilingDb: p.ceiling,
    fadeInS: p.fadeIn, fadeOutS: p.fadeOut, fadeCurve: p.curve,
  };
}

let planBusy = false, planAgain = false;
async function schedulePlan() {
  if (!S.a) return;
  if (planBusy) { planAgain = true; return; }
  planBusy = true;
  try {
    S.plan = await invoke('plan', { params: params() });
    renderStats();
    drawBase();
    applyGain();
  } catch (e) {
    toast(e);
  }
  planBusy = false;
  if (planAgain) { planAgain = false; schedulePlan(); }
}

function renderStats() {
  const pl = S.plan;
  if (!pl) return;
  $('spreadBefore').textContent = num(pl.spreadBefore, 1);
  $('spreadAfter').textContent = num(pl.spreadAfter, 1);
  $('lufsBefore').textContent = num(pl.lufsBefore, 1);
  $('lufsAfter').textContent = `≈ ${num(pl.lufsAfter, 1)}`;
  $('refLufs').textContent = num(pl.referenceLufs, 1);
  $('boostUsed').textContent = signed(pl.maxBoostUsed);
  $('cutUsed').textContent = signed(-pl.maxCutUsed);
  const lim = $('limInfo');
  if (!S.p.limiter) lim.textContent = 'aus';
  else if (pl.limiterShare <= 0) lim.textContent = 'greift nicht ein';
  else {
    const heavy = pl.limiterMaxDb > 4;
    lim.innerHTML = `${num(pl.limiterShare * 100, 1)} % der Zeit, max. ${num(pl.limiterMaxDb, 1)} dB` +
      (heavy ? ' <span class="warn">· viel, Ziel senken?</span>' : '');
  }
  $('targetNote').textContent = S.p.targetAuto
    ? `Hält ${num(pl.referenceLufs, 1)} LUFS, das typische Niveau deines Sets.`
    : `Set-Mittel liegt bei ${num(pl.referenceLufs, 1)} LUFS, also ${signed(pl.targetLufs - pl.referenceLufs)} dB.`;
}

/* ---------- loading ---------- */

async function pickAndLoad() {
  const path = await invoke('pick_input');
  if (path) load(path);
}

async function load(path) {
  stop();
  showBusy('Analysiere', basename(path));
  try {
    const a = await invoke('analyze', { path });
    S.a = a;
    S.plan = null;
    S.start = 0;
    S.end = a.durationS;
    S.playhead = 0;
    S.view = { t0: 0, t1: a.durationS };
    S.lastExport = null;
    $('report').hidden = true;
    $('fileName').textContent = a.fileName;
    $('fileCodec').textContent = a.codec.replace(/_.*/, '');
    $('fileRate').textContent = `${num(a.sampleRate / 1000, a.sampleRate % 1000 ? 1 : 0)} kHz · ${a.channels === 1 ? 'Mono' : 'Stereo'}`;
    $('fileDur').textContent = fmtTime(a.durationS, 0);
    $('clockDur').textContent = fmtTime(a.durationS, 0);
    $('fileBar').hidden = false;
    $('empty').hidden = true;
    $('work').hidden = false;
    setupAudio(path);
    syncCut();
    resizeCanvas();
    await schedulePlan();
  } catch (e) {
    if (e !== 'Abgebrochen') toast(e);
  } finally {
    hideBusy();
  }
}

/* ---------- cut ---------- */

function setCut(start, end) {
  const d = S.a.durationS;
  start = Math.max(0, Math.min(start, d - 1));
  end = Math.min(d, Math.max(end, start + 1));
  S.start = start;
  S.end = end;
  syncCut();
  drawBase();
  schedulePlan();
}

function syncCut() {
  if (document.activeElement !== $('inStart')) $('inStart').value = fmtTime(S.start);
  if (document.activeElement !== $('inEnd')) $('inEnd').value = fmtTime(S.end);
  $('cutLen').textContent = fmtTime(S.end - S.start, 1);
}

function bindTimeInput(id, apply) {
  const el = $(id);
  const commit = () => {
    const v = parseTime(el.value);
    el.classList.toggle('bad', isNaN(v));
    if (!isNaN(v)) apply(v);
  };
  el.addEventListener('keydown', (e) => {
    e.stopPropagation();
    if (e.key === 'Enter') { commit(); el.blur(); }
    if (e.key === 'Escape') { el.classList.remove('bad'); el.blur(); syncCut(); }
  });
  el.addEventListener('blur', () => { commit(); syncCut(); });
}

/* ---------- timeline ---------- */

const base = $('tlBase'), over = $('tlOver'), wrap = $('tlWrap');
const bctx = base.getContext('2d'), octx = over.getContext('2d');
let W = 0, H = 0, DPR = 1;

function resizeCanvas() {
  const r = wrap.getBoundingClientRect();
  DPR = window.devicePixelRatio || 1;
  W = Math.max(1, Math.floor(r.width));
  H = Math.max(1, Math.floor(r.height));
  for (const c of [base, over]) {
    c.width = W * DPR;
    c.height = H * DPR;
  }
  bctx.setTransform(DPR, 0, 0, DPR, 0, 0);
  octx.setTransform(DPR, 0, 0, DPR, 0, 0);
  drawBase();
  drawOver();
}
new ResizeObserver(() => S.a && resizeCanvas()).observe(wrap);

const xOf = (t) => ((t - S.view.t0) / (S.view.t1 - S.view.t0)) * W;
const tOf = (x) => S.view.t0 + (x / W) * (S.view.t1 - S.view.t0);

function lanes() {
  const h = H - RULER;
  const wave = Math.round(h * 0.5);
  const loud = Math.round(h * 0.3);
  return {
    wave: { y: RULER, h: wave },
    loud: { y: RULER + wave, h: loud },
    gain: { y: RULER + wave + loud, h: h - wave - loud },
  };
}

const dbLin = (db) => 10 ** (db / 20);

function fadeGain(curve, x) {
  x = Math.min(1, Math.max(0, x));
  if (curve === 'linear') return x;
  if (curve === 'smooth') return 0.5 - 0.5 * Math.cos(Math.PI * x);
  return Math.max(0, (10 ** (3 * (x - 1)) - 0.001) / 0.999);
}

function fadeAt(t) {
  let g = 1;
  const { fadeIn, fadeOut, curve } = S.p;
  if (fadeIn > 0 && t - S.start < fadeIn) g *= fadeGain(curve, (t - S.start) / fadeIn);
  if (fadeOut > 0 && S.end - t < fadeOut) g *= fadeGain(curve, (S.end - t) / fadeOut);
  return g;
}

/** Gain in dB from the plan at time t (block-centre interpolation, like the renderer). */
function planGainDb(t) {
  const pl = S.plan;
  if (!pl || !pl.gainDb.length) return 0;
  const pos = t / S.a.blockS - 0.5 - pl.b0;
  if (pos <= 0) return pl.gainDb[0];
  const k = Math.floor(pos);
  if (k + 1 >= pl.gainDb.length) return pl.gainDb[pl.gainDb.length - 1];
  return pl.gainDb[k] + (pl.gainDb[k + 1] - pl.gainDb[k]) * (pos - k);
}

function columns(fn) {
  const a = S.a, bs = a.blockS, n = a.peakDb.length;
  for (let x = 0; x < W; x++) {
    const i0 = Math.max(0, Math.floor(tOf(x) / bs));
    const i1 = Math.min(n, Math.max(i0 + 1, Math.ceil(tOf(x + 1) / bs)));
    if (i0 >= n) break;
    fn(x, i0, i1);
  }
}

function hatch(ctx, x0, x1, y0, y1) {
  if (x1 <= x0) return;
  ctx.save();
  ctx.beginPath();
  ctx.rect(x0, y0, x1 - x0, y1 - y0);
  ctx.clip();
  ctx.fillStyle = 'rgba(18,19,21,0.74)';
  ctx.fillRect(x0, y0, x1 - x0, y1 - y0);
  ctx.strokeStyle = C.line;
  ctx.lineWidth = 1;
  ctx.beginPath();
  for (let x = x0 - (y1 - y0) - ((x0 % 8) + 8); x < x1; x += 8) {
    ctx.moveTo(x, y1);
    ctx.lineTo(x + (y1 - y0), y0);
  }
  ctx.stroke();
  ctx.restore();
}

function laneLabel(ctx, text, x, y, color = C.ash) {
  ctx.font = MONO;
  ctx.fillStyle = color;
  ctx.textBaseline = 'top';
  ctx.fillText(text, x, y);
}

function drawRuler(ctx) {
  const span = S.view.t1 - S.view.t0;
  const steps = [0.5, 1, 2, 5, 10, 15, 30, 60, 120, 300, 600, 900, 1800, 3600];
  const step = steps.find((s) => (s / span) * W >= 84) || 3600;
  ctx.fillStyle = C.graphite;
  ctx.fillRect(0, 0, W, RULER);
  ctx.strokeStyle = C.line;
  ctx.beginPath();
  ctx.moveTo(0, RULER - 0.5);
  ctx.lineTo(W, RULER - 0.5);
  ctx.font = MONO;
  ctx.textBaseline = 'middle';
  for (let t = Math.ceil(S.view.t0 / step) * step; t <= S.view.t1; t += step) {
    const x = Math.round(xOf(t)) + 0.5;
    ctx.moveTo(x, RULER - 7);
    ctx.lineTo(x, RULER);
    ctx.fillStyle = C.ash;
    ctx.fillText(fmtTime(t, step < 1 ? 1 : 0), x + 5, RULER / 2 - 1);
  }
  ctx.stroke();
}

function drawBase() {
  if (!S.a || !W) return;
  const ctx = bctx, a = S.a, pl = S.plan;
  const L = lanes();
  ctx.clearRect(0, 0, W, H);
  ctx.fillStyle = C.carbon;
  ctx.fillRect(0, RULER, W, H - RULER);

  const ceil = S.p.limiter ? dbLin(S.p.ceiling) : Infinity;
  const inCut = (i) => pl && i >= pl.b0 && i < pl.b0 + pl.gainDb.length;

  // Waveform: before above the centre line, after below.
  const wv = L.wave, mid = wv.y + wv.h / 2, half = wv.h / 2 - 8;
  columns((x, i0, i1) => {
    const fade = pl ? fadeAt(tOf(x + 0.5)) : 1;
    let pk = 0, rms = 0, pkA = 0, rmsA = 0;
    for (let i = i0; i < i1; i++) {
      const p = dbLin(a.peakDb[i]), r = dbLin(a.rmsDb[i]);
      pk = Math.max(pk, p);
      rms = Math.max(rms, r);
      if (inCut(i)) {
        const g = dbLin(pl.gainDb[i - pl.b0]);
        pkA = Math.max(pkA, Math.min(p * g, ceil) * fade);
        rmsA = Math.max(rmsA, Math.min(r * g, ceil) * fade);
      }
    }
    ctx.fillStyle = '#34373c';
    ctx.fillRect(x, mid - pk * half, 1, pk * half);
    ctx.fillStyle = C.steel;
    ctx.fillRect(x, mid - rms * half, 1, rms * half);
    if (pkA > 0) {
      ctx.fillStyle = C.ash;
      ctx.fillRect(x, mid + 1, 1, Math.min(1, pkA) * half);
      ctx.fillStyle = C.text;
      ctx.fillRect(x, mid + 1, 1, Math.min(1, rmsA) * half);
    }
  });
  ctx.fillStyle = C.line;
  ctx.fillRect(0, mid, W, 1);
  laneLabel(ctx, 'VORHER', 10, wv.y + 8);
  laneLabel(ctx, 'NACHHER', 10, wv.y + wv.h - 20, C.silver);

  // Fade envelopes over the "after" half.
  if (pl) {
    ctx.strokeStyle = C.yellow;
    ctx.lineWidth = 1;
    const drawFade = (t0, t1) => {
      const x0 = Math.max(0, Math.floor(xOf(t0))), x1 = Math.min(W, Math.ceil(xOf(t1)));
      if (x1 <= x0) return;
      ctx.beginPath();
      for (let x = x0; x <= x1; x++) {
        const y = mid + 1 + fadeAt(tOf(x)) * half;
        x === x0 ? ctx.moveTo(x, y) : ctx.lineTo(x, y);
      }
      ctx.stroke();
    };
    if (S.p.fadeIn > 0) drawFade(S.start, S.start + S.p.fadeIn);
    if (S.p.fadeOut > 0) drawFade(S.end - S.p.fadeOut, S.end);
  }

  // Loudness lane.
  const ld = L.loud;
  ctx.fillStyle = C.line;
  ctx.fillRect(0, ld.y, W, 1);
  const ref = pl ? pl.referenceLufs : a.integratedLufs;
  const target = pl ? pl.targetLufs : ref;
  const hi = Math.ceil((Math.max(ref, target) + 7) / 3) * 3;
  const lo = hi - 30;
  const yL = (v) => ld.y + 6 + (1 - (Math.min(hi, Math.max(lo, v)) - lo) / (hi - lo)) * (ld.h - 12);
  ctx.font = MONO;
  ctx.textBaseline = 'middle';
  for (let v = hi - 6; v > lo; v -= 6) {
    ctx.fillStyle = '#222428';
    ctx.fillRect(0, Math.round(yL(v)), W, 1);
    ctx.fillStyle = C.steel;
    ctx.fillText(num(v, 0), W - 34, yL(v) - 7);
  }
  // Short-term loudness as a quiet texture.
  ctx.fillStyle = '#26282c';
  columns((x, i0, i1) => {
    let m = -90;
    for (let i = i0; i < i1; i++) m = Math.max(m, a.shortTermLufs[i]);
    const y = yL(m);
    ctx.fillRect(x, y, 1, ld.y + ld.h - y);
  });
  if (pl) {
    const line = (arr, color, width) => {
      ctx.strokeStyle = color;
      ctx.lineWidth = width;
      ctx.beginPath();
      let started = false;
      columns((x, i0, i1) => {
        let s = 0, c = 0;
        for (let i = i0; i < i1; i++) if (inCut(i)) { s += arr[i - pl.b0]; c++; }
        if (!c) { started = false; return; }
        const y = yL(s / c);
        started ? ctx.lineTo(x, y) : ctx.moveTo(x, y);
        started = true;
      });
      ctx.stroke();
    };
    line(pl.levelBefore, C.steel, 1.5);
    line(pl.levelAfter, C.white, 1.5);
    ctx.setLineDash([4, 4]);
    ctx.strokeStyle = C.yellow;
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.moveTo(0, Math.round(yL(target)) + 0.5);
    ctx.lineTo(W, Math.round(yL(target)) + 0.5);
    ctx.stroke();
    ctx.setLineDash([]);
  }
  laneLabel(ctx, 'LAUTHEIT · LUFS', 10, ld.y + 8);
  laneLabel(ctx, `ZIEL ${num(target, 1)}`, 128, ld.y + 8, C.yellow);

  // Gain lane.
  const gl = L.gain;
  ctx.fillStyle = C.line;
  ctx.fillRect(0, gl.y, W, 1);
  const range = Math.max(6, Math.ceil(Math.max(S.p.boost, S.p.cut, pl ? Math.abs(pl.targetLufs - pl.referenceLufs) + 6 : 0) / 3) * 3);
  const yG = (db) => gl.y + gl.h / 2 - (Math.max(-range, Math.min(range, db)) / range) * (gl.h / 2 - 10);
  ctx.fillStyle = '#26282c';
  ctx.fillRect(0, Math.round(yG(0)), W, 1);
  ctx.font = MONO;
  ctx.fillStyle = C.steel;
  ctx.fillText(`+${range}`, W - 34, yG(range) + 2);
  ctx.fillText(`−${range}`, W - 34, yG(-range) - 2);
  if (pl) {
    if (S.p.limiter) {
      ctx.fillStyle = 'rgba(255,77,61,0.55)';
      columns((x, i0, i1) => {
        let m = 0;
        for (let i = i0; i < i1; i++) if (inCut(i)) m = Math.max(m, pl.limitDb[i - pl.b0]);
        if (m > 0.3) {
          const h = Math.min(1, m / 6) * (gl.h * 0.35);
          ctx.fillRect(x, gl.y + gl.h - h, 1, h);
        }
      });
    }
    ctx.strokeStyle = C.yellow;
    ctx.lineWidth = 1.5;
    ctx.beginPath();
    let started = false;
    columns((x, i0, i1) => {
      let s = 0, c = 0;
      for (let i = i0; i < i1; i++) if (inCut(i)) { s += pl.gainDb[i - pl.b0]; c++; }
      if (!c) { started = false; return; }
      const y = yG(s / c);
      started ? ctx.lineTo(x, y) : ctx.moveTo(x, y);
      started = true;
    });
    ctx.stroke();
  }
  laneLabel(ctx, 'GAIN · dB', 10, gl.y + 8);
  if (S.p.limiter) laneLabel(ctx, 'LIMITER', 90, gl.y + 8, 'rgba(255,77,61,0.85)');

  // Everything outside the cut is greyed and hatched.
  const xs = xOf(S.start), xe = xOf(S.end);
  hatch(ctx, 0, Math.max(0, Math.min(W, xs)), RULER, H);
  hatch(ctx, Math.max(0, xe), W, RULER, H);

  drawRuler(ctx);

  // Cut handles.
  for (const [t, label] of [[S.start, 'IN'], [S.end, 'OUT']]) {
    const x = Math.round(xOf(t));
    if (x < -40 || x > W + 40) continue;
    ctx.fillStyle = C.yellow;
    ctx.fillRect(x, 0, 1, H);
    const w = label === 'IN' ? 26 : 34;
    const bx = label === 'IN' ? x : x - w + 1;
    ctx.fillRect(bx, 4, w, 18);
    ctx.fillStyle = C.graphite;
    ctx.font = '600 10px "JetBrains Mono", monospace';
    ctx.textBaseline = 'middle';
    ctx.fillText(label, bx + 5, 13.5);
  }
}

function drawOver() {
  if (!S.a || !W) return;
  const ctx = octx;
  ctx.clearRect(0, 0, W, H);
  const x = Math.round(xOf(S.playhead)) + 0.5;
  if (x >= 0 && x <= W) {
    ctx.strokeStyle = C.white;
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.moveTo(x, RULER);
    ctx.lineTo(x, H);
    ctx.stroke();
    ctx.fillStyle = C.white;
    ctx.beginPath();
    ctx.moveTo(x - 5, RULER - 6);
    ctx.lineTo(x + 5, RULER - 6);
    ctx.lineTo(x, RULER);
    ctx.fill();
  }
  if (S.hoverX != null) {
    ctx.strokeStyle = C.steel;
    ctx.setLineDash([2, 3]);
    ctx.beginPath();
    ctx.moveTo(S.hoverX + 0.5, RULER);
    ctx.lineTo(S.hoverX + 0.5, H);
    ctx.stroke();
    ctx.setLineDash([]);
  }
}

function updateTip() {
  const tip = $('tlTip');
  if (S.hoverX == null || !S.a || drag) { tip.hidden = true; return; }
  const t = tOf(S.hoverX);
  const pl = S.plan;
  let html = `<b>${fmtTime(t)}</b>`;
  const i = Math.floor(t / S.a.blockS);
  if (pl && t >= S.start && t <= S.end && i - pl.b0 >= 0 && i - pl.b0 < pl.gainDb.length) {
    const k = i - pl.b0;
    html += `<br>${num(pl.levelBefore[k], 1)} → <b>${num(pl.levelAfter[k], 1)}</b> LUFS` +
      `<br><span class="y">${signed(planGainDb(t))} dB</span>`;
  } else if (t < S.start || t > S.end) {
    html += '<br>wird abgeschnitten';
  }
  tip.innerHTML = html;
  tip.hidden = false;
  const w = tip.offsetWidth;
  tip.style.left = `${S.hoverX + 12 + w > W ? S.hoverX - w - 12 : S.hoverX + 12}px`;
}

/* timeline interaction */

let drag = null;

function hitHandle(x) {
  const xs = xOf(S.start), xe = xOf(S.end);
  if (Math.abs(x - xs) <= 7) return 'start';
  if (Math.abs(x - xe) <= 7) return 'end';
  return null;
}

over.addEventListener('mousedown', (e) => {
  if (!S.a || e.button !== 0) return;
  const x = e.offsetX;
  const h = hitHandle(x);
  drag = h ? { kind: h } : { kind: 'pan', x0: x, view: { ...S.view }, moved: false };
});

window.addEventListener('mousemove', (e) => {
  if (!S.a || $('work').hidden) return;
  const r = over.getBoundingClientRect();
  const x = Math.max(0, Math.min(W, e.clientX - r.left));
  const inside = e.clientX >= r.left && e.clientX <= r.right && e.clientY >= r.top && e.clientY <= r.bottom;
  if (drag) {
    if (drag.kind === 'start') setCut(tOf(x), S.end);
    else if (drag.kind === 'end') setCut(S.start, tOf(x));
    else if (drag.kind === 'pan') {
      if (Math.abs(x - drag.x0) > 3) drag.moved = true;
      if (drag.moved) {
        const span = drag.view.t1 - drag.view.t0;
        setView(drag.view.t0 - ((x - drag.x0) / W) * span, span);
      }
    }
  }
  S.hoverX = inside ? x : null;
  over.style.cursor = drag?.kind === 'pan' && drag.moved ? 'grabbing' : inside && hitHandle(x) ? 'ew-resize' : 'crosshair';
  drawOver();
  updateTip();
});

window.addEventListener('mouseup', (e) => {
  if (drag && drag.kind === 'pan' && !drag.moved) {
    const r = over.getBoundingClientRect();
    seek(tOf(e.clientX - r.left));
  }
  drag = null;
});

over.addEventListener('mouseleave', () => {
  S.hoverX = null;
  drawOver();
  updateTip();
});

over.addEventListener('dblclick', () => setView(0, S.a.durationS));

function setView(t0, span) {
  const d = S.a.durationS;
  span = Math.min(d, Math.max(20, span));
  t0 = Math.min(d - span, Math.max(0, t0));
  S.view = { t0, t1: t0 + span };
  drawBase();
  drawOver();
}

over.addEventListener('wheel', (e) => {
  if (!S.a) return;
  e.preventDefault();
  const span = S.view.t1 - S.view.t0;
  if (e.shiftKey || Math.abs(e.deltaX) > Math.abs(e.deltaY)) {
    const d = e.shiftKey && !e.deltaX ? e.deltaY : e.deltaX;
    setView(S.view.t0 + (d / W) * span, span);
  } else {
    const t = tOf(e.offsetX);
    const ns = span * Math.exp(e.deltaY * 0.0025);
    setView(t - (e.offsetX / W) * Math.min(S.a.durationS, Math.max(20, ns)), ns);
  }
}, { passive: false });

$('zoomAll').addEventListener('click', () => setView(0, S.a.durationS));

/* ---------- preview playback ---------- */

const audio = $('audio');
let actx = null, gainNode = null, shaper = null, previewOk = true;

function setupAudio(path) {
  previewOk = true;
  $('playBtn').disabled = false;
  $('playBtn').title = 'Abspielen (Leertaste)';
  audio.src = convertFileSrc(path);
  audio.load();
}

audio.addEventListener('error', () => {
  previewOk = false;
  $('playBtn').disabled = true;
  $('playBtn').title = 'Vorschau für dieses Format nicht verfügbar. Export geht trotzdem.';
});

function ensureGraph() {
  if (actx) return;
  actx = new AudioContext();
  const src = actx.createMediaElementSource(audio);
  gainNode = actx.createGain();
  // The shaper sees the signal at -12 dB so boosted peaks reach its soft knee
  // instead of the hard edge of its input range.
  const pre = actx.createGain();
  pre.gain.value = 0.25;
  shaper = actx.createWaveShaper();
  shaper.oversample = '2x';
  src.connect(gainNode).connect(pre).connect(shaper).connect(actx.destination);
  updateShaper();
}

function updateShaper() {
  if (!shaper) return;
  const n = 4096, curve = new Float32Array(n);
  const limit = S.ab === 'proc' && S.p.limiter;
  const c = dbLin(S.p.ceiling), k = 0.8 * c;
  for (let i = 0; i < n; i++) {
    const v = ((i / (n - 1)) * 2 - 1) * 4;
    let y = v;
    if (limit) {
      const a = Math.abs(v);
      y = a <= k ? v : Math.sign(v) * (k + (c - k) * Math.tanh((a - k) / (c - k)));
    }
    curve[i] = y;
  }
  shaper.curve = curve;
}

function previewGain(t) {
  if (S.ab === 'orig' || !S.plan) return 1;
  if (t < S.start || t > S.end) return 0;
  return dbLin(planGainDb(t)) * fadeAt(t);
}

function applyGain() {
  if (!gainNode) return;
  gainNode.gain.setTargetAtTime(previewGain(audio.currentTime), actx.currentTime, 0.012);
}

const playing = () => !audio.paused;

async function play() {
  if (!previewOk || !S.a) return;
  ensureGraph();
  if (actx.state === 'suspended') await actx.resume();
  if (S.ab === 'proc' && (S.playhead < S.start || S.playhead >= S.end)) S.playhead = S.start;
  audio.currentTime = S.playhead;
  gainNode.gain.value = previewGain(S.playhead);
  try {
    await audio.play();
  } catch (e) {
    toast(`Vorschau nicht möglich: ${e.message || e}`);
    return;
  }
  $('playBtn').classList.add('playing');
  requestAnimationFrame(tick);
}

function stop() {
  audio.pause();
  $('playBtn').classList.remove('playing');
}

function tick() {
  if (!playing()) { $('playBtn').classList.remove('playing'); return; }
  S.playhead = audio.currentTime;
  applyGain();
  if (S.ab === 'proc' && S.playhead >= S.end) {
    stop();
    S.playhead = S.end;
  }
  // Keep the playhead in view while zoomed in.
  const span = S.view.t1 - S.view.t0;
  if (!drag && (S.playhead > S.view.t1 || S.playhead < S.view.t0)) setView(S.playhead - span * 0.1, span);
  $('clockNow').textContent = fmtTime(S.playhead);
  drawOver();
  requestAnimationFrame(tick);
}

function seek(t) {
  S.playhead = Math.max(0, Math.min(S.a.durationS, t));
  if (previewOk && audio.readyState > 0) audio.currentTime = S.playhead;
  $('clockNow').textContent = fmtTime(S.playhead);
  applyGain();
  drawOver();
}

$('playBtn').addEventListener('click', () => (playing() ? stop() : play()));

const syncAb = segment('abSeg', () => S.ab, (v) => {
  S.ab = v;
  updateShaper();
  applyGain();
});

/* ---------- export ---------- */

async function doExport() {
  if (!S.a) return;
  const ext = S.format === 'mp3' ? 'mp3' : 'wav';
  const stem = S.a.fileName.replace(/\.[^.]+$/, '');
  const path = await invoke('pick_output', { defaultName: `${stem} (LUFSy).${ext}`, extension: ext });
  if (!path) return;
  stop();
  showBusy('Exportiere', basename(path));
  try {
    const r = await invoke('export', { params: params(), format: S.format, path });
    S.lastExport = r.path;
    $('repFile').textContent = basename(r.path);
    $('repLufs').innerHTML = `<b>${num(r.integratedLufs, 1)}</b> LUFS`;
    $('repTp').innerHTML = `<b>${num(r.truePeakDb, 1)}</b> dBTP`;
    $('repLra').innerHTML = `<b>${num(r.loudnessRange, 1)}</b> LU`;
    $('repDur').textContent = fmtTime(r.durationS, 1);
    $('report').hidden = false;
    $('report').scrollIntoView({ block: 'nearest' });
    toast('Export fertig.', true);
  } catch (e) {
    if (e !== 'Abgebrochen') toast(e);
  } finally {
    hideBusy();
  }
}

$('exportBtn').addEventListener('click', doExport);
$('revealBtn').addEventListener('click', () => {
  if (S.lastExport) invoke('plugin:opener|reveal_item_in_dir', { paths: [S.lastExport] }).catch(toast);
});

/* ---------- wiring ---------- */

loadSettings();
buildControls();

segment('targetSeg', () => (S.p.targetAuto ? 'auto' : 'fixed'), (v) => {
  S.p.targetAuto = v === 'auto';
  if (!S.p.targetAuto && S.plan && !S.p.targetTouched) {
    // Start from where the set already is, so switching does not jump.
    S.p.target = Math.round(S.plan.referenceLufs * 2) / 2;
    ctls.target.show();
  }
  S.p.targetTouched = true;
  syncEnabled();
  saveSettings();
  schedulePlan();
});
segment('curveSeg', () => S.p.curve, (v) => { S.p.curve = v; saveSettings(); schedulePlan(); });
segment('formatSeg', () => S.format, (v) => { S.format = v; saveSettings(); });
$('limSwitch').addEventListener('click', () => {
  S.p.limiter = !S.p.limiter;
  syncEnabled();
  updateShaper();
  saveSettings();
  schedulePlan();
});
syncEnabled();

$('openBtn').addEventListener('click', pickAndLoad);
$('pickBtn').addEventListener('click', pickAndLoad);
bindTimeInput('inStart', (v) => setCut(v, S.end));
bindTimeInput('inEnd', (v) => setCut(S.start, v));
$('setStart').addEventListener('click', () => setCut(S.playhead, S.end));
$('setEnd').addEventListener('click', () => setCut(S.start, S.playhead));
$('trimSilence').addEventListener('click', () => setCut(S.a.contentStartS, S.a.contentEndS));
$('resetCut').addEventListener('click', () => setCut(0, S.a.durationS));

window.addEventListener('keydown', (e) => {
  if (!S.a || $('work').hidden || !$('busy').hidden) return;
  if (e.target.tagName === 'INPUT' && e.target.type !== 'range') return;
  const k = e.key.toLowerCase();
  if (e.key === ' ') { e.preventDefault(); playing() ? stop() : play(); }
  else if (k === 'i') setCut(S.playhead, S.end);
  else if (k === 'o') setCut(S.start, S.playhead);
  else if (k === 'a') { S.ab = S.ab === 'proc' ? 'orig' : 'proc'; syncAb(); updateShaper(); applyGain(); }
  else if (e.key === 'ArrowLeft' || e.key === 'ArrowRight') {
    if (e.target.type === 'range') return;
    e.preventDefault();
    seek(S.playhead + (e.key === 'ArrowLeft' ? -1 : 1) * (e.shiftKey ? 30 : 5));
  }
});

// Drag & drop from Finder / Explorer.
window.__TAURI__.webview.getCurrentWebview().onDragDropEvent((e) => {
  const t = e.payload.type;
  $('dropping').hidden = !(t === 'enter' || t === 'over');
  $('drop')?.classList.toggle('hot', t === 'enter' || t === 'over');
  if (t === 'drop' && e.payload.paths?.length) load(e.payload.paths[0]);
});

invoke('initial_path').then((p) => p && load(p));

