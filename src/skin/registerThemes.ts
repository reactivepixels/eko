/**
 * Theme registration bootstrap (docs/skin-registry-plan.md §2.2).
 *
 * Explicit composition — NOT scattered import-time side effects — so registration order is
 * deterministic and debuggable. Porcelain (free) always registers first; the Pro themes come
 * from `@pro`'s `proThemes` (an empty array in the free-build stub).
 *
 * Imported once for its side effect by `App.tsx`, before the first render, so the registry is
 * populated before `ThemeHost` resolves a theme.
 */
import { registerTheme } from "./registry";
import { PorcelainShell } from "../player/porcelain/PorcelainShell";
import { proThemes } from "@pro";

// Porcelain (free) is the base case, registered first; Pro themes come from `@pro`
// (an empty array in the free-build stub). Theme objects are built here at the composition
// boundary so the Shell files stay pure components (clean Fast-Refresh).
registerTheme({ id: "porcelain", label: "Porcelain", tier: "free", Shell: PorcelainShell });
proThemes.forEach(registerTheme);
