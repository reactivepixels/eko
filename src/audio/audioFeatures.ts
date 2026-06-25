/**
 * AudioFeatureTracker — shared, framework-free audio feature extractor.
 *
 * Consumes raw 32-band spectrum data (0..1) from the Rust engine (via useSpectrum)
 * and derives the feature set needed by GPU visualizers.
 *
 * IMPORTANT: no import from @pro or src/pro — this file must compile in the free build.
 *
 * Feature pipeline ported from concepts/viz-galaxy.html's updateFeatures / updateClock
 * (LIVE path only — the offline whole-song pre-analysis is NOT ported).
 */

const NB = 32;

export interface AudioFeatures {
  /** Per-band auto-ranged 0..1 (peak/floor normalised per band so no band pins at max). */
  bands: Float32Array;
  /** Per-band relative excitement: how far each band has risen above its slow average (0..1). */
  react: Float32Array;
  /** Overall RMS energy envelope (snappy attack / slower release). */
  energy: number;
  /** Smoothed low-band average (bass, bands 0–5). */
  low: number;
  /** Smoothed mid-band average (mids, bands 6–17). */
  mid: number;
  /** Smoothed high-band average (highs, bands 18–31). */
  high: number;
  /** Peak per-band react across all bands this frame. */
  reactPeak: number;
  /** Peak react across the low bands (kick / bass excitement). */
  reactLow: number;
  /** Spectral flux onset signal (positive only). */
  onset: number;
  /** Beat phase 0..1 from the phase-locked oscillator. */
  beatPhase: number;
  /** Bar phase 0..1 (4 beats). */
  barPhase: number;
  /** Estimated BPM. */
  bpm: number;
  /** Running time in seconds. */
  t: number;
  /** Beat pump envelope 0..1 — whole-field bloom driven by kick + sustained bass level. */
  pump: number;
  /** Shed impulse strength this frame (0 except on a big-beat trigger frame). */
  shedImpulse: number;
  /** Shed event seed (changes each event to select a different particle subset). */
  shedSeedJS: number;
  /** Integrated global rotation angle (radians). Increases monotonically. */
  fieldAngle: number;
  /** Slowly smoothed energy (drives form/geometry). */
  smEnergy: number;
  /** Slowly smoothed low (drives form). */
  smLow: number;
  /** Slowly smoothed mid (drives form). */
  smMid: number;
  /** Slowly smoothed high (drives form). */
  smHigh: number;
}

/** Hardcoded defaults matching the baked values in the concept (no tweak panel in app). */
const DEFAULTS = {
  bpmOverride: 0,
  tempoLock: 1.0,
  motionTempo: 2.0,
  breatheAmt: 1.5,
  waveAmt: 1.5,
  spinAmt: 1.5,
  flockAmt: 1.5,
  shedThresh: 0.55,
  calm: 1.0,
  rotRate: 0.12,
  damping: 3.3,
};

export class AudioFeatureTracker {
  // ── output (mutated in-place each update) ────────────────────────────────
  readonly features: AudioFeatures;

  // ── per-band state ────────────────────────────────────────────────────────
  private readonly _raw = new Float32Array(NB);
  private readonly _prevRaw = new Float32Array(NB);
  private readonly _disp = new Float32Array(NB); // envelope-smoothed display value
  private readonly _bandAvg = new Float32Array(NB);
  private readonly _react = new Float32Array(NB);
  private readonly _bandPk = new Float32Array(NB);
  private readonly _bandFl = new Float32Array(NB);

  // ── global state ──────────────────────────────────────────────────────────
  private _energyEnv = 0;
  private _smEnergy = 0;
  private _smLow = 0;
  private _smMid = 0;
  private _smHigh = 0;
  private _reactPeak = 0;
  private _reactLow = 0;
  private _pump = 0;
  private _shedImpulse = 0;
  private _shedSeedJS = 0;
  private _lastShed = -1;
  private _fieldAngle = 0;
  private _rotRateSm = 0;

  // ── tempo clock state ─────────────────────────────────────────────────────
  private _bpm = 120;
  private _beatPhase = 0;
  private _barPhase = 0;
  private _clockPhase = 0;
  private _beatCount = 0;
  private readonly _onsetBuf = new Float32Array(320); // ~5s ring buffer at 64 Hz
  private _oi = 0;
  private _filled = false;
  private _dtAccum = 0;
  private _sinceCorr = 0;
  private _onsetMean = 0;

  private _t = 0;

  constructor() {
    // initialise per-band peaks so the auto-ranger doesn't start at zero
    for (let i = 0; i < NB; i++) {
      this._bandPk[i] = 0.25;
      this._bandFl[i] = 0.0;
    }

    this.features = {
      bands: this._disp,
      react: this._react,
      energy: 0,
      low: 0,
      mid: 0,
      high: 0,
      reactPeak: 0,
      reactLow: 0,
      onset: 0,
      beatPhase: 0,
      barPhase: 0,
      bpm: 120,
      t: 0,
      pump: 0,
      shedImpulse: 0,
      shedSeedJS: 0,
      fieldAngle: 0,
      smEnergy: 0,
      smLow: 0,
      smMid: 0,
      smHigh: 0,
    };
  }

  /**
   * Call once per animation frame.
   * @param raw  32-band spectrum from the engine (0..1), or null when idle.
   * @param dtMs  Frame delta in milliseconds.
   */
  update(raw: number[] | null, dtMs: number): void {
    const dt = Math.min(dtMs / 1000, 0.05); // cap at 50 ms
    this._t += dt;

    // ── fill _raw from the live engine spectrum ──────────────────────────
    if (raw !== null && raw.length >= NB) {
      this._pullLiveSpectrum(raw, dt);
    } else {
      // no track: let display decay toward calm idle
      for (let b = 0; b < NB; b++) this._raw[b] *= 0.9;
    }

    // ── envelope-smooth raw[] → _disp[] ─────────────────────────────────
    const aUp = 1 - Math.exp(-dt / 0.03);
    const aDn = 1 - Math.exp(-dt / 0.14);
    let sumSq = 0;
    let lo = 0;
    let mi = 0;
    let hi = 0;
    for (let b = 0; b < NB; b++) {
      const tgt = this._raw[b];
      const k = tgt > this._disp[b] ? aUp : aDn;
      this._disp[b] += (tgt - this._disp[b]) * k;
      sumSq += this._disp[b] * this._disp[b];
      if (b < 6) lo += this._disp[b];
      else if (b < 18) mi += this._disp[b];
      else hi += this._disp[b];
    }
    const instLow = lo / 6;
    const instMid = mi / 12;
    const instHigh = hi / 14;
    const inst = Math.sqrt(sumSq / NB);

    // ── energy envelope (fast attack / slower release) ───────────────────
    const ke = 1 - Math.exp(-dt / 0.04);
    const kr = 1 - Math.exp(-dt / 0.25);
    this._energyEnv +=
      (inst - this._energyEnv) * (inst > this._energyEnv ? ke : kr);

    // ── per-band relative excitement ──────────────────────────────────────
    const avgK = 1 - Math.exp(-dt / 1.3);
    const rUp = 1 - Math.exp(-dt / 0.045);
    const rDn = 1 - Math.exp(-dt / 0.22);
    this._reactPeak = 0;
    this._reactLow = 0;
    for (let b = 0; b < NB; b++) {
      this._bandAvg[b] += (this._disp[b] - this._bandAvg[b]) * avgK;
      const rise = Math.max(0, this._disp[b] - this._bandAvg[b]);
      const tgt = Math.min(1, rise * (b < 6 ? 9.5 : 5.5));
      this._react[b] +=
        (tgt - this._react[b]) * (tgt > this._react[b] ? rUp : rDn);
      if (this._react[b] > this._reactPeak) this._reactPeak = this._react[b];
      if (b < 6 && this._react[b] > this._reactLow)
        this._reactLow = this._react[b];
    }

    // ── slowly smoothed spectrum terms → FORM (not brightness) ───────────
    const sE = 1 - Math.exp(-dt / 0.2);
    const sL = 1 - Math.exp(-dt / 0.3);
    const sM = 1 - Math.exp(-dt / 0.34);
    const sH = 1 - Math.exp(-dt / 0.26);
    this._smEnergy += (this._energyEnv - this._smEnergy) * sE;
    this._smLow += (instLow - this._smLow) * sL;
    this._smMid += (instMid - this._smMid) * sM;
    this._smHigh += (instHigh - this._smHigh) * sH;

    // ── tempo clock ───────────────────────────────────────────────────────
    const onset = this._updateClock(dt, raw !== null);

    // ── global rotation angle (integrated, smooth) ────────────────────────
    const cadence =
      1.0 +
      (Math.max(0.5, Math.min(2.2, this._bpm / 120)) - 1.0) *
        DEFAULTS.motionTempo;
    const rotTarget = DEFAULTS.rotRate * (0.6 + DEFAULTS.calm) * cadence;
    this._rotRateSm +=
      (rotTarget - this._rotRateSm) * (1 - Math.exp(-dt / 0.6));
    this._fieldAngle += this._rotRateSm * dt;

    // ── beat pump envelope ─────────────────────────────────────────────────
    let bassLvl = 0;
    for (let b = 0; b < 8; b++) bassLvl += this._disp[b];
    bassLvl /= 8;
    const levelDrive = Math.max(0, bassLvl - 0.2) / 0.7;
    const pumpTarget = Math.max(
      this._reactLow,
      this._reactPeak * 0.45,
      levelDrive * 0.95,
    );
    const pUp = 1 - Math.exp(-dt / 0.13);
    const pDn = 1 - Math.exp(-dt / 0.55);
    this._pump +=
      (pumpTarget - this._pump) *
      (pumpTarget > this._pump ? pUp : pDn);

    // ── shed trigger ──────────────────────────────────────────────────────
    this._shedImpulse = 0;
    if (
      this._reactLow > DEFAULTS.shedThresh &&
      this._t - this._lastShed > 0.16
    ) {
      this._shedImpulse = Math.min(
        1,
        (this._reactLow - DEFAULTS.shedThresh) /
          Math.max(0.05, 1.0 - DEFAULTS.shedThresh),
      );
      this._shedSeedJS = (this._shedSeedJS + 1) % 9973;
      this._lastShed = this._t;
    }

    // ── commit prevRaw ───────────────────────────────────────────────────
    for (let b = 0; b < NB; b++) this._prevRaw[b] = this._raw[b];

    // ── publish to features ───────────────────────────────────────────────
    const f = this.features;
    // bands and react are Float32Arrays shared by reference — already updated
    f.energy = this._energyEnv;
    f.low = instLow;
    f.mid = instMid;
    f.high = instHigh;
    f.reactPeak = this._reactPeak;
    f.reactLow = this._reactLow;
    f.onset = onset;
    f.beatPhase = this._beatPhase;
    f.barPhase = this._barPhase;
    f.bpm = this._bpm;
    f.t = this._t;
    f.pump = this._pump;
    f.shedImpulse = this._shedImpulse;
    f.shedSeedJS = this._shedSeedJS;
    f.fieldAngle = this._fieldAngle;
    f.smEnergy = this._smEnergy;
    f.smLow = this._smLow;
    f.smMid = this._smMid;
    f.smHigh = this._smHigh;
  }

  // ── private helpers ────────────────────────────────────────────────────

  /** Pull live engine spectrum into _raw[], with per-band auto-ranging. */
  private _pullLiveSpectrum(raw: number[], dt: number): void {
    const pkDecay = Math.exp(-dt / 2.0);
    const flRise = 1 - Math.exp(-dt / 3.0);
    for (let b = 0; b < NB; b++) {
      let v = raw[b] ?? 0;
      v = v < 0 ? 0 : v > 1 ? 1 : v;
      // per-band peak/floor auto-ranging
      this._bandPk[b] =
        v > this._bandPk[b]
          ? v
          : Math.max(this._bandFl[b] + 0.04, this._bandPk[b] * pkDecay);
      const flK =
        v < this._bandFl[b] ? 1 - Math.exp(-dt / 0.5) : flRise;
      this._bandFl[b] +=
        (Math.min(v, this._bandFl[b] + 0.5) - this._bandFl[b]) * flK;
      let nv =
        (v - this._bandFl[b]) /
        Math.max(0.03, this._bandPk[b] - this._bandFl[b]);
      nv = nv < 0 ? 0 : nv > 1 ? 1 : nv;
      this._raw[b] = nv;
    }
  }

  /** Autocorrelation BPM estimator — mirrors the concept's autocorrBPM(). */
  private _autocorrBPM(): number {
    const buf = this._onsetBuf;
    const N = buf.length;
    let mean = 0;
    for (let i = 0; i < N; i++) mean += buf[i];
    mean /= N;
    const dtS = 5.0 / N;
    const minLag = Math.max(2, Math.floor(60 / 180 / dtS));
    const maxLag = Math.min(N - 2, Math.floor(60 / 70 / dtS));
    let bestLag = -1;
    let bestVal = -1e9;
    for (let lag = minLag; lag <= maxLag; lag++) {
      let s = 0;
      for (let i = 0; i < N - lag; i++)
        s += (buf[i] - mean) * (buf[i + lag] - mean);
      const lagS = lag * dtS;
      const bpm = 60 / lagS;
      const oct = 1.0 - 0.0018 * Math.abs(bpm - 120);
      s *= Math.max(0.4, oct);
      if (s > bestVal) {
        bestVal = s;
        bestLag = lag;
      }
    }
    if (bestLag < 0) return this._bpm;
    let bpm = 60 / (bestLag * dtS);
    while (bpm < 80) bpm *= 2;
    while (bpm > 165) bpm /= 2;
    return bpm;
  }

  /**
   * Update tempo clock and return the current onset signal.
   * When raw is null (no track), falls back to 120 BPM synthetic clock.
   */
  private _updateClock(dt: number, hasTrack: boolean): number {
    if (!hasTrack) {
      // synthetic fallback: clean 120 BPM
      this._bpm = 120;
      this._clockPhase += (120 / 60) * dt;
      if (this._clockPhase >= 1) {
        this._beatCount += Math.floor(this._clockPhase);
        this._clockPhase -= Math.floor(this._clockPhase);
      }
      this._beatPhase = this._clockPhase;
      this._barPhase =
        ((this._beatCount % 4) + this._clockPhase) / 4;
      return 0;
    }

    // ── onset envelope: kick-weighted positive spectral flux ─────────────
    let flux = 0;
    for (let b = 0; b < NB; b++) {
      const w = b < 5 ? 3.0 : b < 12 ? 1.0 : 0.35;
      flux += w * Math.max(0, this._raw[b] - this._prevRaw[b]);
    }
    this._onsetMean +=
      (flux - this._onsetMean) * (1 - Math.exp(-dt / 0.6));
    const onset = Math.max(0, flux - this._onsetMean);

    // push into ring buffer at a steady 64 Hz cadence
    this._dtAccum += dt;
    const step = 1 / 64;
    while (this._dtAccum >= step) {
      this._dtAccum -= step;
      this._onsetBuf[this._oi] = onset;
      this._oi = (this._oi + 1) % this._onsetBuf.length;
      if (this._oi === 0) this._filled = true;
    }

    // re-estimate BPM a few times a second
    this._sinceCorr += dt;
    if (this._filled && this._sinceCorr > 0.25) {
      this._sinceCorr = 0;
      const est = this._autocorrBPM();
      this._bpm += (est - this._bpm) * 0.18;
    }

    let bpm =
      DEFAULTS.bpmOverride > 0 ? DEFAULTS.bpmOverride : this._bpm;
    bpm = Math.max(50, Math.min(220, bpm));

    // ── phase-locked oscillator ───────────────────────────────────────────
    const beatsPerSec = bpm / 60;
    this._clockPhase += beatsPerSec * dt;
    // nudge toward onset peaks (PLL correction)
    if (onset > this._onsetMean * 1.4 + 0.002) {
      const frac = this._clockPhase - Math.floor(this._clockPhase);
      const err = frac > 0.5 ? frac - 1.0 : frac;
      const lock = Math.max(0, Math.min(1, DEFAULTS.tempoLock));
      this._clockPhase -= err * lock * 0.5;
    }
    if (this._clockPhase >= 1) {
      this._beatCount += Math.floor(this._clockPhase);
      this._clockPhase -= Math.floor(this._clockPhase);
    }
    this._bpm = bpm;
    this._beatPhase = this._clockPhase;
    this._barPhase =
      ((this._beatCount % 4) + this._clockPhase) / 4;

    return onset;
  }
}
