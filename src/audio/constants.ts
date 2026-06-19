// Classic Winamp 10-band graphic-EQ center frequencies (Hz).
export const EQ_BANDS = [60, 170, 310, 600, 1000, 3000, 6000, 12000, 14000, 16000] as const;

export const EQ_BAND_COUNT = EQ_BANDS.length;

// Per-band gain range, in dB. Matches Winamp's ±12 dB sliders.
export const EQ_GAIN_MIN = -12;
export const EQ_GAIN_MAX = 12;

// Q for each peaking filter. ~1.0 gives the broad, musical curves of the original.
export const EQ_Q = 1.0;

export type EqPreset = {
  name: string;
  preamp: number; // dB
  gains: number[]; // length EQ_BAND_COUNT, dB
};

// Musical presets tuned for these 10 broad (Q≈1) peaking bands. Because the bands
// overlap, boosts add up — so each preset carries a negative `preamp` for headroom,
// keeping the signal under 0 dBFS (no clipping/harshness). gains are low→high.
export const EQ_PRESETS: EqPreset[] = [
  { name: "Flat", preamp: 0, gains: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0] },
  { name: "Rock", preamp: -5, gains: [5, 3, -1, -2, -1, 1, 3, 4, 5, 5] },
  { name: "Pop", preamp: -3, gains: [-1, 1, 3, 4, 3, 1, -1, -1, 0, 1] },
  { name: "Bass Boost", preamp: -5, gains: [6, 5, 4, 2, 0, 0, 0, 0, 0, 0] },
  { name: "Treble Boost", preamp: -4, gains: [0, 0, 0, 0, 0, 1, 3, 4, 5, 5] },
  { name: "Vocal", preamp: -3, gains: [-2, -2, 0, 2, 4, 4, 3, 1, 0, -1] },
  { name: "Jazz", preamp: -3, gains: [3, 2, 0, 1, -1, -1, 0, 1, 2, 3] },
  { name: "Acoustic", preamp: -3, gains: [3, 3, 1, 0, 1, 1, 2, 2, 1, 1] },
  { name: "Classical", preamp: -2, gains: [3, 2, 0, 0, 0, 0, -1, -1, -2, -3] },
  { name: "Loudness", preamp: -5, gains: [6, 4, 0, -1, -2, -1, 0, 3, 5, 5] },
];

export const FLAT_GAINS = (): number[] => new Array(EQ_BAND_COUNT).fill(0);
