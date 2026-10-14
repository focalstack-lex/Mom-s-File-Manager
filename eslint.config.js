import js from "@eslint/js";
import tseslint from "typescript-eslint";
import globals from "globals";
import reactHooks from "eslint-plugin-react-hooks";
import tote from "./eslint-rules/tote-invariants.js";

/**
 * Tote ESLint gate.
 *
 * Narrow on purpose: the four directive invariants (zero emoji, zero em or en
 * dash, no silent catch, no committed secret) plus TypeScript and hooks
 * correctness. No style preferences, so it never forces a structural rewrite.
 */
const INVARIANTS = {
  "tote/no-emoji": "error",
  "tote/no-dash-punctuation": "error",
  "tote/no-silent-catch": "error",
  "tote/no-hardcoded-secret": "error",
};

export default tseslint.config(
  {
    ignores: ["dist/**", "node_modules/**", "src-tauri/**", "reports/**", "docs/**", ".scratch/**", ".claude/**"],
  },
  js.configs.recommended,
  ...tseslint.configs.recommended,
  {
    files: ["**/*.{js,mjs,ts,tsx}"],
    plugins: { tote },
    languageOptions: {
      ecmaVersion: 2022,
      sourceType: "module",
      globals: { ...globals.browser, ...globals.node, ...globals.es2021 },
    },
    rules: {
      ...INVARIANTS,
      // TypeScript resolves identifiers; no-undef misfires on DOM and JSX types.
      "no-undef": "off",
    },
  },
  {
    files: ["src/**/*.{ts,tsx}"],
    plugins: { "react-hooks": reactHooks },
    rules: {
      "react-hooks/rules-of-hooks": "error",
      "react-hooks/exhaustive-deps": "warn",
      "@typescript-eslint/no-unused-vars": ["error", { argsIgnorePattern: "^_", varsIgnorePattern: "^_" }],
    },
  },
);
