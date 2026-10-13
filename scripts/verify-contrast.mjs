#!/usr/bin/env node
// Contrast gate over the design tokens in src/styles.css (spec 7): every text
// token on every surface it can sit on. Body text needs 4.5:1; --text-faint
// is reserved for 13 px and larger labels and needs 3:1; the accent is used
// as a focus and selection mark and needs 3:1 on light surfaces.
import { readFileSync } from "node:fs";

const css = readFileSync(new URL("../src/styles.css", import.meta.url), "utf8");
const start = css.indexOf(":root {");
const close = css.indexOf("}", start);
const tokens = {};
for (const match of css.slice(start, close).matchAll(/--([a-z-]+):\s*(#[0-9a-f]{6})\s*;/gi)) tokens[match[1]] = match[2];

function luminance(hex) {
  const channel = (offset) => {
    const value = parseInt(hex.slice(offset, offset + 2), 16) / 255;
    return value <= 0.03928 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * channel(1) + 0.7152 * channel(3) + 0.0722 * channel(5);
}

function ratio(a, b) {
  const [high, low] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (high + 0.05) / (low + 0.05);
}

const PAIRS = [
  ...["bg-canvas", "bg-card", "bg-raised", "accent-soft", "coral-soft"].flatMap((surface) => [
    { text: "text-primary", surface, min: 4.5 },
    { text: "text-muted", surface, min: 4.5 },
    { text: "text-faint", surface, min: 3 },
    { text: "accent", surface, min: 3 },
    { text: "coral", surface, min: 4.5 },
  ]),
  { text: "accent-text", surface: "accent-soft", min: 4.5 },
  { text: "on-accent", surface: "accent", min: 4.5 },
  { text: "on-accent", surface: "accent-hover", min: 4.5 },
  { text: "sidebar-text", surface: "bg-sidebar", min: 4.5 },
  { text: "sidebar-muted", surface: "bg-sidebar", min: 4.5 },
  // Hovered nav buttons switch their text to --sidebar-text (styles.css .nav-button:hover),
  // so muted text never sits on the hover surface.
  { text: "sidebar-text", surface: "bg-sidebar-hover", min: 4.5 },
  { text: "sidebar-active-text", surface: "bg-sidebar-active", min: 4.5 },
  { text: "sidebar-text", surface: "bg-shell-top", min: 4.5 },
  { text: "sidebar-text", surface: "bg-shell-bottom", min: 4.5 },
];

let failures = 0;
for (const { text, surface, min } of PAIRS) {
  const fg = tokens[text];
  const bg = tokens[surface];
  if (!fg || !bg) {
    failures += 1;
    console.error(`[FAIL] missing token --${fg ? surface : text}`);
    continue;
  }
  const value = ratio(fg, bg);
  const ok = value >= min;
  if (!ok) failures += 1;
  console.log(`[${ok ? "PASS" : "FAIL"}] --${text} on --${surface}: ${value.toFixed(2)}:1 (min ${min})`);
}
console.log(`\n${PAIRS.length} pairs checked, ${failures} below threshold.`);
process.exit(failures === 0 ? 0 : 1);
