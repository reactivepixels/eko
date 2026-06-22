import js from "@eslint/js";
import tseslint from "typescript-eslint";
import reactHooks from "eslint-plugin-react-hooks";
import reactRefresh from "eslint-plugin-react-refresh";
import prettier from "eslint-config-prettier";

export default tseslint.config(
  // `site/` is a separate Astro sub-project with its own toolchain (astro check).
  { ignores: ["dist", "src-tauri", "concepts", "node_modules", "site"] },
  js.configs.recommended,
  ...tseslint.configs.recommended,
  {
    files: ["**/*.{ts,tsx}"],
    plugins: {
      "react-hooks": reactHooks,
      "react-refresh": reactRefresh,
    },
    rules: {
      ...reactHooks.configs.recommended.rules,
      // Allow intentionally-unused params/vars when prefixed with `_` (e.g. no-op stub
      // signatures in src/pro-stub that must match the @pro interface).
      "@typescript-eslint/no-unused-vars": [
        "error",
        { argsIgnorePattern: "^_", varsIgnorePattern: "^_", caughtErrorsIgnorePattern: "^_" },
      ],
      "react-refresh/only-export-components": ["warn", { allowConstantExport: true }],
      // Brand-new compiler-era rule; flags legitimate prop-reset / async-cache effects
      // (e.g. LocalCover resetting on a new path). Off until it's less false-positive-prone.
      "react-hooks/set-state-in-effect": "off",
    },
  },
  prettier,
);
