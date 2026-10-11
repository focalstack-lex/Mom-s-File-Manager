import js from "@eslint/js";
import tseslint from "typescript-eslint";
import globals from "globals";
import reactHooks from "eslint-plugin-react-hooks";
import desk from "./eslint-rules/desk-invariants.js";

/**
 * Desk ESLint gate.
 *
 * Narrow on purpose: the four directive invariants (zero emoji, zero em or en
 * dash, no silent catch, no committed secret) plus TypeScript and hooks
 * correctness. No style preferences, so it never forces a structural rewrite.
 */
const INVARIANTS = {
  "desk/no-emoji": "error",
  "desk/no-dash-punctuation": "error",
  "desk/no-silent-catch": "error",
  "desk/no-hardcoded-secret": "error",
};

export default tseslint.config(
  {
    ignores: ["dist/**", "node_modules/**", "src-tauri/**", "reports/**", "docs/**", ".scratch/**", ".claude/**"],
  },
  js.configs.recommended,
  ...tseslint.configs.recommended,
  {
    files: ["**/*.{js,mjs,ts,tsx}"],
    plugins: { desk },
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
