#!/usr/bin/env node
/**
 * Tote native check: the real app, real files, driven through its own window.
 *
 * Builds a throwaway folder set, launches the release tote.exe in test mode
 * (TOTE_ROOTS and TOTE_DATA_DIR, so nothing real is touched and autostart is
 * left alone) with WebView2 remote debugging on, then drives the actual
 * window over CDP: drops downloads with Zone.Identifier streams and waits for
 * them to be filed, searches the real index, moves, renames, pins, recycles
 * and undoes through the real commands. Evidence lands in
 * reports/native-check/<stamp>/.
 *
 * Usage: node scripts/verify-native.mjs   (needs src-tauri/target/release/tote.exe)
 */
import { spawn, execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readdirSync, readFileSync, rmSync, utimesSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "playwright";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
// The fresh build when there is one; otherwise the copy kept in installers/
// (cargo clean removes target/ to save space).
const BUILT_EXE = join(ROOT, "src-tauri", "target", "release", "tote.exe");
const EXE = existsSync(BUILT_EXE) ? BUILT_EXE : join(ROOT, "installers", "tote-release.exe");
const LAB = resolve(process.env.LOCALAPPDATA ?? ROOT, "tote-native-check");
const PORT = 9333;
const STEP_TIMEOUT_MS = 10_000;
const stamp = new Date().toISOString().replace(/[:.]/g, "-").slice(0, 19);
const EVIDENCE_DIR = join(ROOT, "reports", "native-check", stamp);
const results = [];
const pageErrors = [];

function record(step, passed, detail) {
  results.push({ step, passed, detail });
  process.stdout.write(`[${passed ? "PASS" : "FAIL"}] ${step}${detail ? ` :: ${detail}` : ""}\n`);
}
const sleep = (ms) => new Promise((done) => setTimeout(done, ms));

async function until(check, timeout = STEP_TIMEOUT_MS) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    if (await check()) return true;
    await sleep(500);
  }
  return false;
}

function seedLab() {
  rmSync(LAB, { recursive: true, force: true });
  for (const d of ["Desktop", "Documents\\Forms", "Downloads", "data"]) mkdirSync(join(LAB, d), { recursive: true });
  writeFileSync(join(LAB, "Desktop", "Lesson Plan.docx"), "x");
  writeFileSync(join(LAB, "Desktop", "Lesson Plan (2).docx"), "xy");
  writeFileSync(join(LAB, "Desktop", "notes.txt"), "Santos Reyes attendance Grade 7");
  // For Organize: names that carry a grade and a subject, and a shortcut that must stay.
  writeFileSync(join(LAB, "Desktop", "Grade 7 Science Quiz.docx"), "quiz");
  writeFileSync(join(LAB, "Desktop", "AP 8 Reviewer.docx"), "reviewer");
  writeFileSync(join(LAB, "Desktop", "Zoom.lnk"), "shortcut");
  writeFileSync(join(LAB, "Documents", "Forms", "SF9 Template.docx"), "master");
  // A code project in Documents: its build output must stay out of the index.
  mkdirSync(join(LAB, "Documents", "app", "target", "release"), { recursive: true });
  writeFileSync(join(LAB, "Documents", "app", "Cargo.toml"), "[package]");
  writeFileSync(join(LAB, "Documents", "app", "target", "release", "zebrajunk.lib"), "x");
  mkdirSync(join(LAB, "Desktop", "Saved Here"), { recursive: true });
  const old = join(LAB, "Downloads", "old attendance.pdf");
  writeFileSync(old, "old");
  const past = new Date(Date.now() - 90 * 86_400_000);
  utimesSync(old, past, past);
}

/** Writes a file plus the Zone.Identifier stream a browser would leave. */
function fakeDownload(name, body, hostUrl, referrer, folder = join(LAB, "Downloads")) {
  const path = join(folder, name);
  writeFileSync(path, body);
  const zone = `[ZoneTransfer]\r\nZoneId=3\r\n${referrer ? `ReferrerUrl=${referrer}\r\n` : ""}HostUrl=${hostUrl}\r\n`;
  execFileSync("powershell", ["-NoProfile", "-Command", `Set-Content -LiteralPath '${path.replace(/'/g, "''")}' -Stream Zone.Identifier -Value '${zone.replace(/'/g, "''")}'`]);
  return path;
}

const filed = (kind, name) => existsSync(join(LAB, "Documents", "School Files", kind, new Date().toISOString().slice(0, 7), name));

async function main() {
  if (!existsSync(EXE)) {
    process.stderr.write(`[NATIVE CHECK ERROR] ${EXE} not found. Run npm run build:release first.\n`);
    process.exit(1);
  }
  mkdirSync(EVIDENCE_DIR, { recursive: true });
  seedLab();
  const env = {
    ...process.env,
    TOTE_ROOTS: `${join(LAB, "Desktop")};${join(LAB, "Documents")};${join(LAB, "Downloads")}`,
    TOTE_DATA_DIR: join(LAB, "data"),
    // Only honoured together with TOTE_ROOTS (src-tauri/src/lib.rs), so a normal start never opens a port.
    TOTE_DEBUG_PORT: String(PORT),
    // Opt-in (TOTE_CHECK_UPDATE=1): the app treats the published release as
    // new, downloads it and verifies its signature, and stops before installing.
    ...(process.env.TOTE_CHECK_UPDATE ? { TOTE_UPDATE_VERIFY: "1" } : {}),
  };
  const app = spawn(EXE, [], { env, stdio: "ignore", detached: false });
  let browser = null;
  let exitCode = 0;
  try {
    const attached = await until(async () => {
      try {
        browser = await chromium.connectOverCDP(`http://127.0.0.1:${PORT}`);
        return true;
      } catch {
        return false;
      }
    }, 30_000);
    record("The release app starts in test mode and exposes its window over CDP", attached && !app.exitCode, attached ? "" : "no CDP endpoint");
    if (!attached) throw new Error("could not attach");
    const context = browser.contexts()[0];
    const page = context.pages().find((p) => p.url().startsWith("tauri://") || p.url().includes("localhost")) ?? context.pages()[0];
    page.on("pageerror", (error) => pageErrors.push(error.message));
    await page.waitForSelector('[data-testid="find-view"]', { timeout: STEP_TIMEOUT_MS });
    const capture = (name) => page.screenshot({ path: join(EVIDENCE_DIR, `${name}.png`) });

    // 1. First run shows the walkthrough; Escape skips it and it stays skipped (real settings table).
    const tour = await page.locator('[data-testid="tour"]').isVisible();
    await page.keyboard.press("Escape");
    const tourGone = await until(async () => !(await page.locator('[data-testid="tour"]').isVisible()));
    await capture("01-find");
    record("First run shows the walkthrough; Escape skips it", tour && tourGone, JSON.stringify({ tour, tourGone }));

    // 2. The real index: a student's name finds notes.txt by its contents.
    await page.locator('[data-testid="search-input"]').fill("santos");
    const hit = await until(async () => (await page.locator('[data-testid="results"] .row-name-text').first().textContent().catch(() => "")) === "notes.txt");
    const marked = await page.locator('[data-testid="results"] .row-snippet mark').first().textContent().catch(() => null);
    await capture("02-search");
    record("Real index: 'santos' finds notes.txt by its contents with a marked snippet", hit && marked?.toLowerCase() === "santos", JSON.stringify({ hit, marked }));
    // 2b. Build output beside a project file is not indexed.
    await page.locator('[data-testid="search-input"]').fill("zebrajunk");
    await sleep(1500);
    const junkHits = await page.locator('[data-testid="results"] [data-testid="file-row"]').count();
    record("Build output beside Cargo.toml stays out of search", junkHits === 0, `hits=${junkHits}`);
    await page.locator('[data-testid="search-input"]').fill("");

    // 2c. A browser that saved to another folder: the stamped file joins Recent downloads and stays put.
    const elsewhere = fakeDownload("Me at the zoo.mp3", "ID3 fake", "https://files.example.org/zoo.mp3", "https://www.example.org/", join(LAB, "Desktop", "Saved Here"));
    const noticed = await until(async () => (await page.locator('[data-testid="recent"] .row-name-text', { hasText: "Me at the zoo.mp3" }).count()) === 1, 20_000);
    const where = noticed ? await page.locator('[data-testid="recent"] [data-testid="file-row"]', { hasText: "Me at the zoo.mp3" }).locator(".row-where").textContent() : null;
    record("A download saved outside Downloads appears in Recent downloads and is not moved", noticed && existsSync(elsewhere) && String(where).includes("Saved Here"), JSON.stringify({ noticed, stayed: existsSync(elsewhere), where }));

    // 3. Two downloads land (Messenger and DepEd streams) and are filed with their sources; an installer stays.
    fakeDownload("Grade 7 Quiz 2.docx", "Grade 7 Science Quiz Photosynthesis", "https://cdn.fbsbx.com/v/t59/quiz.docx?x=1", "https://www.facebook.com/messages/t/123");
    fakeDownload("DepEd Order 014.pdf", "%PDF-1.4 fake", "https://www.deped.gov.ph/wp-content/uploads/DO_014.pdf");
    writeFileSync(join(LAB, "Downloads", "printer-setup.exe"), "MZ");
    const quizFiled = await until(() => filed("Word", "Grade 7 Quiz 2.docx"), 40_000);
    const orderFiled = await until(() => filed("PDF", "DepEd Order 014.pdf"), 20_000);
    const exeStayed = existsSync(join(LAB, "Downloads", "printer-setup.exe"));
    // The rows first show the file where it landed (Downloads), then where it was filed.
    const rowWhere = async (name) => page.locator('[data-testid="recent"] [data-testid="file-row"]', { hasText: name }).locator(".row-where").first().textContent().catch(() => "");
    const recentOk = await until(async () => {
      const metas = await page.locator('[data-testid="recent"] .row-meta').allTextContents();
      const sources = metas.some((m) => m.startsWith("from Messenger")) && metas.some((m) => m.startsWith("from DepEd"));
      return sources && (await rowWhere("Grade 7 Quiz 2.docx")).includes("School Files") && (await rowWhere("DepEd Order 014.pdf")).includes("School Files");
    }, 20_000);
    await capture("03-filed");
    record("Two downloads are filed by type and month with Messenger and DepEd sources and the rows show their new place; the installer stays in Downloads", quizFiled && orderFiled && exeStayed && recentOk, JSON.stringify({ quizFiled, orderFiled, exeStayed, recentOk }));

    // 4. Move the quiz to Documents > Forms through the real folder picker and command.
    const quizRow = page.locator('[data-testid="recent"] [data-testid="file-row"]', { hasText: "Grade 7 Quiz 2.docx" });
    await quizRow.locator("input[type=checkbox]").check();
    const barShown = await until(async () => (await page.locator('[data-testid="action-bar"]').count()) === 1);
    if (!barShown) {
      const state = await page.evaluate(() => ({
        checked: [...document.querySelectorAll('[data-testid="file-row"] input[type=checkbox]')].filter((c) => c.checked).length,
        rows: document.querySelectorAll('[data-testid="recent"] [data-testid="file-row"]').length,
        toasts: [...document.querySelectorAll(".toast")].map((t) => t.textContent),
      }));
      await capture("03b-no-action-bar");
      record("Ticking a row shows the action bar", false, JSON.stringify(state));
    }
    await capture("03c-before-move");
    const barBox = await page.evaluate(() => {
      const bar = document.querySelector('[data-testid="action-bar"]');
      const button = [...document.querySelectorAll('[data-testid="action-bar"] button')].find((b) => b.textContent?.includes("Move to"));
      const r = button?.getBoundingClientRect();
      const hit = r ? document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2) : null;
      return { bar: Boolean(bar), button: r ? { x: Math.round(r.left), y: Math.round(r.top), w: Math.round(r.width), h: Math.round(r.height) } : null, hitTag: hit?.tagName, hitClass: hit?.className, inner: window.innerHeight };
    });
    if (!barBox.bar || barBox.hitTag !== "BUTTON") record("The Move to... button is reachable and uncovered", false, JSON.stringify(barBox));
    await page.getByRole("button", { name: "Move to..." }).click({ timeout: 10_000 });
    await page.locator(".folder-option", { hasText: "Documents > Forms" }).click();
    await page.locator('[data-testid="move-confirm"]').click();
    const movedOnDisk = await until(() => existsSync(join(LAB, "Documents", "Forms", "Grade 7 Quiz 2.docx")));
    record("Move to... moves the file on disk", movedOnDisk, String(movedOnDisk));

    // 5. Rename it inline; the extension stays.
    await until(async () => (await page.locator('[data-testid="action-bar"]').count()) === 0);
    const row = page.locator('[data-testid="recent"] [data-testid="file-row"]', { hasText: "Grade 7 Quiz 2.docx" });
    await row.locator("input[type=checkbox]").check();
    await page.getByRole("button", { name: "Rename" }).click();
    await page.locator('[data-testid="rename-input"]').fill("Science Quiz 2");
    await page.keyboard.press("Enter");
    const renamedOnDisk = await until(() => existsSync(join(LAB, "Documents", "Forms", "Science Quiz 2.docx")));
    record("Rename changes the name on disk and keeps .docx", renamedOnDisk, String(renamedOnDisk));

    // 6. Pin it; it shows under Pinned.
    await until(async () => (await page.locator('[data-testid="action-bar"]').count()) === 0);
    await page.locator('[data-testid="recent"] [data-testid="file-row"]', { hasText: "Science Quiz 2.docx" }).getByRole("button", { name: /^Pin / }).click();
    const pinned = await until(async () => (await page.locator('[data-testid="pinned"] [data-testid="file-row"]', { hasText: "Science Quiz 2.docx" }).count()) === 1);
    record("Pin puts the file under Pinned", pinned, String(pinned));

    // 7. Recycle the DepEd order, then Undo from Activity restores it from the Recycle Bin.
    const orderRow = page.locator('[data-testid="recent"] [data-testid="file-row"]', { hasText: "DepEd Order 014.pdf" });
    await orderRow.locator("input[type=checkbox]").check();
    await page.getByRole("button", { name: "Recycle Bin" }).click();
    await page.getByRole("button", { name: "Move to Recycle Bin" }).click();
    const gone = await until(() => !filed("PDF", "DepEd Order 014.pdf"));
    await page.locator('[data-testid="nav-activity"]').click();
    await page.waitForSelector('[data-testid="activity-row"]', { timeout: STEP_TIMEOUT_MS });
    const firstTitle = await page.locator('[data-testid="activity-row"] strong').first().textContent();
    await page.locator('[data-testid="activity-row"]').first().locator('[data-testid="undo"]').click();
    const back = await until(() => filed("PDF", "DepEd Order 014.pdf"), 15_000);
    await capture("04-activity");
    record("Recycle Bin removes the file and Undo restores it from the Recycle Bin", gone && firstTitle?.includes("Recycle Bin") && back, JSON.stringify({ gone, firstTitle, back }));

    // 8. Clean up sees the duplicate lesson plans and the old download.
    await page.locator('[data-testid="nav-cleanup"]').click();
    await page.waitForSelector('[data-testid="cleanup-view"] .card', { timeout: STEP_TIMEOUT_MS });
    const dupes = await page.locator('[data-testid="cleanup-view"] .latest').count();
    const old = await page.locator('[data-testid="old-downloads-list"] .suggestion').count();
    await capture("05-cleanup");
    record("Clean up lists the duplicate lesson plans and the 90-day-old download", dupes >= 1 && old === 1, JSON.stringify({ dupes, old }));

    // 8b. Organize now on real files: grade and subject folders are made, the shortcut stays, Undo all reverses it.
    const school = join(LAB, "Documents", "School Files");
    await page.locator('[data-testid="organize-open"]').click();
    await page.waitForSelector('[data-testid="organize-dialog"]', { timeout: STEP_TIMEOUT_MS });
    const planTitle = await page.locator('[data-testid="organize-dialog"] #dialog-title').textContent();
    await page.locator('[data-testid="organize-confirm"]').click();
    const quizHome = join(school, "Grade 7", "Science", "Grade 7 Science Quiz.docx");
    const apHome = join(school, "Grade 8", "Araling Panlipunan", "AP 8 Reviewer.docx");
    const organized = await until(() => existsSync(quizHome) && existsSync(apHome), 15_000);
    const shortcutStayed = existsSync(join(LAB, "Desktop", "Zoom.lnk"));
    const desktopLeft = readdirSync(join(LAB, "Desktop")).filter((n) => !["Zoom.lnk", "Saved Here"].includes(n));
    await capture("05b-organized");
    await page.locator('[data-testid="nav-activity"]').click();
    await page.waitForSelector('[data-testid="activity-batch"]', { timeout: STEP_TIMEOUT_MS });
    await page.locator('[data-testid="undo-batch"]').first().click();
    const restored = await until(() => existsSync(join(LAB, "Desktop", "Grade 7 Science Quiz.docx")) && existsSync(join(LAB, "Desktop", "AP 8 Reviewer.docx")), 15_000);
    const emptyRemoved = await until(() => !existsSync(join(school, "Grade 7")) && !existsSync(join(school, "Grade 8")), 5_000);
    await capture("05c-organize-undone");
    record("Organize files real grade and subject names into new folders, leaves the shortcut, and Undo all puts every file back and removes the empty folders",
      organized && shortcutStayed && desktopLeft.length === 0 && restored && emptyRemoved,
      JSON.stringify({ planTitle, organized, shortcutStayed, desktopLeft, restored, emptyRemoved }));

    // 9. Settings persist to the real database: notifications off survives a reload of app info.
    await page.locator('[data-testid="nav-settings"]').click();
    await page.waitForSelector('[data-testid="settings-view"]', { timeout: STEP_TIMEOUT_MS });
    const toggle = page.locator("label.toggle", { hasText: "Tell me when a download is filed" }).locator("input");
    await toggle.uncheck();
    await sleep(500);
    const dbText = execFileSync("python", ["-I", "-c", `import sqlite3;print(sqlite3.connect(r'${join(LAB, "data", "tote.sqlite")}').execute("select value from settings where key='settings'").fetchone()[0])`], { encoding: "utf8" });
    record("Settings are written to the real database", dbText.includes('"notifications":false') && dbText.includes('"tourDone":true'), dbText.trim().slice(0, 120));

    // The window has no Windows frame: the green title bar carries the controls.
    const frame = await page.evaluate(() => ({ frameless: outerWidth === innerWidth && outerHeight === innerHeight, controls: [...document.querySelectorAll(".window-button")].map((b) => b.getAttribute("aria-label")) }));
    const startWidth = await page.evaluate(() => outerWidth);
    await page.getByRole("button", { name: "Maximize" }).click();
    const maximized = await until(() => page.evaluate(() => outerWidth === screen.availWidth));
    // The label follows Windows' resize event a moment after the width changes.
    await until(async () => (await page.locator(".window-button").nth(1).getAttribute("aria-label")) === "Restore");
    const restoreLabel = await page.locator(".window-button").nth(1).getAttribute("aria-label");
    await page.getByRole("button", { name: "Restore" }).click();
    const unmaximized = await until(() => page.evaluate((w) => outerWidth === w, startWidth));
    await capture("12-title-bar");
    record("The green title bar replaces the Windows frame; Maximize and Restore work from its buttons",
      frame.frameless && frame.controls.join(",") === "Minimize,Maximize,Close" && maximized && restoreLabel === "Restore" && unmaximized,
      JSON.stringify({ ...frame, maximized, restoreLabel, unmaximized }));

    if (process.env.TOTE_CHECK_UPDATE) {
      // The live manifest, the signed installer on the site and the public key
      // in this build, end to end. The renderer's own IPC bridge is used, as the
      // banner would.
      const offered = await page.evaluate(() => window.__TAURI_INTERNALS__.invoke("check_update"));
      const installed = await page.evaluate(() => window.__TAURI_INTERNALS__.invoke("install_update").then(() => "ok", (e) => JSON.stringify(e)));
      const logPath = join(process.env.LOCALAPPDATA ?? "", "com.focalstack.tote", "logs", "tote-test.log");
      const log = existsSync(logPath) ? readFileSync(logPath, "utf8") : "";
      const verified = /update: \S+ downloaded \(\d+ bytes\) and its signature verified/.test(log) && log.includes("dry run, so");
      record("Update: the live manifest offers a release, its installer downloads from codewithlex.site and its signature verifies; nothing is installed in a dry run",
        Boolean(offered?.version) && installed === "ok" && verified, JSON.stringify({ offered, installed, verified }));
    }

    record("No uncaught page errors in the real window", pageErrors.length === 0, pageErrors.join(" | "));
  } catch (error) {
    process.stderr.write(`[NATIVE CHECK ERROR] ${error.stack || error.message}\n`);
    exitCode = 1;
  } finally {
    if (browser) await browser.close().catch(() => undefined);
    spawn("taskkill", ["/pid", String(app.pid), "/T", "/F"], { stdio: "ignore" });
    await sleep(800);
    const logPath = join(process.env.LOCALAPPDATA ?? "", "com.focalstack.tote", "logs", "tote-test.log");
    if (existsSync(logPath)) writeFileSync(join(EVIDENCE_DIR, "tote.log"), readFileSync(logPath, "utf8").split("\n").slice(-200).join("\n"));
    writeFileSync(join(EVIDENCE_DIR, "summary.json"), `${JSON.stringify({ stamp, lab: LAB, steps: results, pageErrors, downloadsLeft: readdirSync(join(LAB, "Downloads")) }, null, 2)}\n`);
  }
  const failed = results.filter((r) => !r.passed).length;
  if (failed > 0 || pageErrors.length > 0) exitCode = 1;
  process.stdout.write(`\nEvidence written to ${EVIDENCE_DIR}\n${exitCode === 0 ? "[NATIVE CHECK PASS] The real app files, finds, moves, renames, pins, recycles and undoes." : `[NATIVE CHECK FAIL] ${failed} step(s) failed.`}\n`);
  process.exit(exitCode);
}

await main();
