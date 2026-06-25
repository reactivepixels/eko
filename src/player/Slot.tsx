/**
 * Slot — renders the variant the active skin's preset assigns to a slot.
 *
 * Resolution: the active preset's slot default ?? the free default. A variant Component consumes
 * its slot's headless hook itself, so Slot renders it with no props. A skin only ever renders its
 * OWN variants — there is no cross-skin override (that "Customize" picker was removed because
 * forcing one skin's control into another skin left its material tokens, e.g. --cap-a/--ring-grad,
 * undefined → broken controls). Unknown ids fall through to the free default so a slot always
 * renders something valid.
 */
import { useUiStore } from "../store/useUiStore";
import { getVariant } from "../skin/variants";
import { presetSlots, DEFAULT_SLOT_VARIANT } from "../skin/presets";
import type { Role } from "../skin/vocabulary";

export function Slot({ slot }: { slot: Role }) {
  const skin = useUiStore((s) => s.skin);
  const presetId = presetSlots(skin)[slot] ?? DEFAULT_SLOT_VARIANT[slot];
  const def = getVariant(presetId) ?? getVariant(DEFAULT_SLOT_VARIANT[slot]);
  if (!def) return null;
  const Component = def.Component;
  return <Component />;
}
