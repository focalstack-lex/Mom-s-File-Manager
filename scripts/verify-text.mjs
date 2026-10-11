#!/usr/bin/env node
/**
 * Text invariant gate: zero em dash, zero en dash and zero emoji in every text
 * file the repository tracks, whatever its language. ESLint enforces the same
 * rules inside JS and TS; this covers everything else (.rs, .css, .html, .md,
 * .sql, .toml, .json, .yml). Binaries, lockfiles and vendored third-party
 * license texts are skipped: they are not ours to edit.
 *
 * Usage: npm run verify:text
 */
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

// Built from code points so this file never contains the characters it bans.
const DASH = new RegExp("[" + String.fromCodePoint(0x2013, 0x2014) + "]", "u");
const EMOJI = /\p{Extended_Pictographic}/u;
// Extended_Pictographic also covers a few typographic symbols that are legal in copy.
const ALLOWED = new Set([0xa9, 0xae, 0x2122, 0x2139].map((point) => String.fromCodePoint(point)));
const BINARY = /\.(png|ico|icns|woff2?|ttf|otf|jpe?g|gif|webp|zip|exe|msi)$/i;
const GENERATED = new Set(["package-lock.json", "src-tauri/Cargo.lock"]);
const VENDORED = /^src\/assets\/fonts\/LICENSE-/;

/** True for repository paths whose content this gate owns. */
export function isScannable(path) {
  return !BINARY.test(path) && !GENERATED.has(path) && !VENDORED.test(path);
}

/** Every dash or emoji in the text, with 1-based line and code-point column. */
export function scanText(text) {
  const violations = [];
  text.split(/\r?\n/).forEach((line, index) => {
    let column = 0;
    for (const char of line) {
      column += 1;
      if (DASH.test(char)) {
        violations.push({ line: index + 1, column, kind: "dash", sample: char });
      } else if (EMOJI.test(char) && !ALLOWED.has(char)) {
        violations.push({ line: index + 1, column, kind: "emoji", sample: char });
      }
    }
  });
  return violations;
}

function main() {
  const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
  const tracked = execFileSync("git", ["ls-files", "-z"], { cwd: root, encoding: "utf8" })
    .split("\0")
    .filter(Boolean)
    .filter(isScannable);

  let failures = 0;
  for (const path of tracked) {
    for (const v of scanText(readFileSync(resolve(root, path), "utf8"))) {
      failures += 1;
      const code = v.sample.codePointAt(0).toString(16).toUpperCase().padStart(4, "0");
      console.error(`${path}:${v.line}:${v.column} ${v.kind} U+${code}`);
    }
  }
  console.log(`${tracked.length} tracked text files scanned, ${failures} violation(s).`);
  process.exit(failures === 0 ? 0 : 1);
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) main();
