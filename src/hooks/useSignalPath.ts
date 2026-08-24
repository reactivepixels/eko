import { useState } from "react";
import { usePlayerStore } from "../store/usePlayerStore";
import { nativeEngine } from "../audio/nativeEngine";
import type { SignalPathReport } from "../audio/nativeEngine";
import type { ReplayGainMode } from "../types";

/**
 * The single source of bit-perfect truth — read, not derived.
 *
 * The derivation lives in Rust (`eko_core::signal_path::derive`, tested in
 * `crates/eko-core/src/signal_path.rs`) so the desktop app and the terminal client
 * report the same seal for the same playback. This hook does NOT compute the seal, and
 * must not start to: any rule added here would be a rule the CLI does not apply, and
 * the product's central claim would then depend on which front end you looked at.
 *
 * `usePlayerStore`'s `refreshSignalPath()` calls Rust once per change to one of the
 * seal's inputs and caches the result, so this stays a synchronous store read — no
 * per-render IPC, and every consumer sees the same object at the same time.
 *
 * When no seal has been derived yet, `active` is false and every field is empty. That
 * is deliberate: consumers render NOTHING rather than a default. A seal that flickered
 * to BIT-PERFECT while the signal was being resampled would be a lying seal.
 *
 * There is a second, narrower "not yet" state, and consumers must handle it too. Because
 * the derivation is asynchronous, an input can change while the reply that would confirm
 * the verdict is still in flight. The store withdraws the cached bit-perfect claim the
 * instant that happens (`unconfirmSeal` in `usePlayerStore.ts`), so what arrives here is
 * `pure: false`, a `sealLabel` naming no modifier, and an EMPTY `engineLabel`. The rest
 * of the seal — `active`, SOURCE, OUTPUT, RG — is untouched and still true, so nothing
 * blanks and nothing blinks. Read an empty `engineLabel` on a non-pure seal as "no claim
 * yet", never as "no processing".
 *
 * A `StatusLamp`/seal binds `pure`; it is a lit status, NEVER a toggle.
 */
export const NO_SEAL: SignalPathReport = Object.freeze({
  active: false,
  pure: false,
  eqActive: false,
  attenuated: false,
  rgActive: false,
  resampled: false,
  osResampled: false,
  codec: "",
  src: "",
  output: "",
  engineLabel: "",
  sealLabel: "",
  rgLabel: "",
});

export function useSignalPath() {
  const info = usePlayerStore((s) => s.engineInfo);
  const signalPath = usePlayerStore((s) => s.signalPath);
  const outputDevice = usePlayerStore((s) => s.outputDevice);
  const replayGainMode = usePlayerStore((s) => s.replayGainMode);
  const rgAppliedDb = usePlayerStore((s) => s.rgAppliedDb);

  const [devices, setDevices] = useState<string[]>([]);

  const sp = signalPath ?? NO_SEAL;

  return {
    // ── The seal and its breakdown, derived in Rust ──
    ...sp,
    info,
    // ── Raw store state the pickers bind to ──
    replayGainMode,
    rgAppliedDb,
    setReplayGainMode: (m: ReplayGainMode) => usePlayerStore.getState().setReplayGainMode(m),
    outputDevice,
    setOutputDevice: (name: string | null) => usePlayerStore.getState().setOutputDevice(name),
    devices,
    loadDevices: async () => setDevices(await nativeEngine.listDevices().catch(() => [])),
  };
}
