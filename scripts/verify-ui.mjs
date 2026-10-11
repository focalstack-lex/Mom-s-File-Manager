#!/usr/bin/env node
/**
 * Desk UI verification drive.
 *
 * Starts the Vite dev server, which activates the mocked backend in a plain
 * browser (src/main.tsx), then drives the renderer with Playwright the way
 * she would: real keys and clicks, screenshots and console output as evidence.
 *
 * Exits non-zero when the shell fails to render, a mapped surface does not
 * open, or the page raises an uncaught error. Evidence lands in
 * reports/ui-verification/<stamp>/. Native behaviour (sorting, the real
 * database, the tray) stays with cargo test.
 *
 * Usage: npm run verify:ui        (add --headed to watch it run)
 */
import { spawn } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "playwright";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const VITE_BIN = join(ROOT, "node_modules", "vite", "bin", "vite.js");
const HOST = "127.0.0.1";
const PORT = 1430;
const BASE_URL = `http://${HOST}:${PORT}`;
const READY_TIMEOUT_MS = 60_000;
const STEP_TIMEOUT_MS = 8_000;
const VIEWPORT = { width: 1280, height: 820 };
const HEADED = process.argv.includes("--headed");

const stamp = new Date().toISOString().replace(/[:.]/g, "-").slice(0, 19);
const EVIDENCE_DIR = join(ROOT, "reports", "ui-verification", stamp);

const results = [];
const consoleErrors = [];
const pageErrors = [];

function record(step, passed, detail) {
  results.push({ step, passed, detail });
  process.stdout.write(`[${passed ? "PASS" : "FAIL"}] ${step}${detail ? ` :: ${detail}` : ""}\n`);
}

const sleep = (ms) => new Promise((done) => setTimeout(done, ms));

async function waitForServer() {
  const deadline = Date.now() + READY_TIMEOUT_MS;
  while (Date.now() < deadline) {
    try {
      const response = await fetch(BASE_URL, { signal: AbortSignal.timeout(3000) });
      if (response.ok) return (await response.text()).includes('id="root"');
    } catch {
      // Not listening yet; the dev server is still starting.
    }
    await sleep(500);
  }
  return false;
}

function startDevServer() {
  const child = spawn(process.execPath, [VITE_BIN, "--host", HOST, "--port", String(PORT), "--strictPort"], {
    cwd: ROOT,
    env: { ...process.env, BROWSER: "none", CI: "1" },
    stdio: ["ignore", "pipe", "pipe"],
  });
  let log = "";
  child.stdout.on("data", (chunk) => { log += chunk.toString(); });
  child.stderr.on("data", (chunk) => { log += chunk.toString(); });
  return { child, readLog: () => log };
}

function stopDevServer(child) {
  if (!child || child.exitCode !== null) return;
  if (process.platform === "win32") {
    spawn("taskkill", ["/pid", String(child.pid), "/T", "/F"], { stdio: "ignore" });
  } else {
    child.kill("SIGTERM");
  }
}

async function visible(page, selector) {
  try {
    await page.waitForSelector(selector, { state: "visible", timeout: STEP_TIMEOUT_MS });
    return true;
  } catch {
    return false;
  }
}

async function hidden(page, selector) {
  try {
    await page.waitForSelector(selector, { state: "hidden", timeout: STEP_TIMEOUT_MS });
    return true;
  } catch {
    return false;
  }
}

const capture = (page, name) => page.screenshot({ path: join(EVIDENCE_DIR, `${name}.png`) });
const errorToasts = (page) => page.locator(".toast-error").count();

async function newPage(browser, viewport = VIEWPORT) {
  const context = await browser.newContext({ viewport });
  const page = await context.newPage();
  page.on("crash", () => pageErrors.push("renderer crashed"));
  page.on("console", (message) => { if (message.type() === "error") consoleErrors.push(message.text()); });
  page.on("pageerror", (error) => pageErrors.push(error.message));
  await page.goto(BASE_URL, { waitUntil: "domcontentloaded" });
  await page.waitForSelector('[data-testid="app"]', { timeout: READY_TIMEOUT_MS });
  await page.waitForSelector('[data-testid="find-view"]', { timeout: STEP_TIMEOUT_MS });
  return { context, page };
}

/** No region may scroll sideways and nothing may stick out of the window. */
async function layoutReport(page) {
  return page.evaluate(() => {
    const regions = {};
    let ok = true;
    for (const selector of ['[data-testid="sidebar"]', '[data-testid="main"]', ".search", ".row"]) {
      const el = document.querySelector(selector);
      if (!el) continue;
      const rect = el.getBoundingClientRect();
      const overflow = el.scrollWidth > el.clientWidth + 1 || rect.right > window.innerWidth + 0.5;
      if (overflow) ok = false;
      regions[selector] = { scrollWidth: el.scrollWidth, clientWidth: el.clientWidth, right: Math.round(rect.right), overflow };
    }
    const sidebar = document.querySelector('[data-testid="sidebar"]')?.getBoundingClientRect().width ?? -1;
    return { ok, sidebar, width: window.innerWidth, docOverflow: document.documentElement.scrollWidth > window.innerWidth, regions };
  });
}

async function drive(browser) {
  const { context, page } = await newPage(browser);

  // 1. First run: the walkthrough opens on top of the Find screen with an arrow.
  const tourOpen = await visible(page, '[data-testid="tour"]');
  const arrowDrawn = tourOpen && (await page.locator(".tour-arrow path").count()) === 1;
  const ringOnSearch = await page.evaluate(() => {
    const ring = document.querySelector(".tour-ring")?.getBoundingClientRect();
    const search = document.querySelector('[data-tour="search"]')?.getBoundingClientRect();
    return Boolean(ring && search && ring.top <= search.top && ring.bottom >= search.bottom);
  });
  await capture(page, "01-tour-step-1");
  record("First run: the walkthrough opens with an arrow and a ring around the search box", tourOpen && arrowDrawn && ringOnSearch, JSON.stringify({ tourOpen, arrowDrawn, ringOnSearch }));

  // 2. Enter advances through all five steps and the last one finishes it; it is saved as done.
  const titles = [];
  for (let i = 0; i < 5; i += 1) {
    titles.push(await page.locator("#tour-title").textContent());
    if (i === 2) await capture(page, "02-tour-step-3-open");
    await page.keyboard.press("Enter");
    await sleep(120);
  }
  const tourClosed = await hidden(page, '[data-testid="tour"]');
  const saved = await page.evaluate(() => window.__deskMock.calls.some((c) => c.command === "set_settings" && c.args.settings?.tourDone === true));
  record("Enter walks through five steps, then the walkthrough closes and is remembered", titles.length === 5 && new Set(titles).size === 5 && tourClosed && saved, JSON.stringify({ titles, tourClosed, saved }));

  // 3. The Find screen shows recent downloads with source and location, and the status line.
  const rows = await page.locator('[data-testid="recent"] [data-testid="file-row"]').count();
  const firstMeta = await page.locator('[data-testid="recent"] .row-meta').first().textContent();
  const firstWhere = await page.locator('[data-testid="recent"] .row-where').first().textContent();
  const status = await page.locator('[data-testid="status-line"]').textContent();
  const pinned = await page.locator('[data-testid="pinned"] [data-testid="file-row"]').count();
  await capture(page, "03-find-home");
  record("Find shows pinned files, recent downloads with source and location, and the quiet status line",
    rows === 5 && firstMeta?.startsWith("from Messenger") && firstWhere?.includes("School Files > Word") && status?.includes("filed for you") && pinned === 1,
    JSON.stringify({ rows, firstMeta, firstWhere, status, pinned }));

  // 4. Typing a student's name finds a spreadsheet by its contents, with a snippet.
  await page.locator('[data-testid="search-input"]').fill("santos");
  await visible(page, '[data-testid="results"]');
  await page.waitForFunction(() => document.querySelectorAll('[data-testid="results"] [data-testid="file-row"]').length > 0, null, { timeout: STEP_TIMEOUT_MS });
  const hitName = await page.locator('[data-testid="results"] .row-name-text').first().textContent();
  const snippet = await page.locator('[data-testid="results"] .row-snippet mark').first().textContent();
  await capture(page, "04-search-content");
  record("Searching a student's name finds Book1.xlsx by its contents and marks the match", hitName === "Book1.xlsx" && snippet === "Santos", JSON.stringify({ hitName, snippet }));

  // 5. Copies are grouped with Latest marked and the others behind one button.
  await page.locator('[data-testid="search-input"]').fill("lesson plan");
  await page.waitForFunction(() => document.querySelector('[data-testid="results"] .latest'), null, { timeout: STEP_TIMEOUT_MS });
  const latestName = await page.locator('[data-testid="results"] .row-name-text').first().textContent();
  const copiesButton = await page.locator(".copies-toggle").first().textContent();
  await page.locator(".copies-toggle").first().click();
  const expanded = await page.locator(".copies [data-testid='file-row']").count();
  await capture(page, "05-duplicates");
  record("Copies group under the newest one with Latest and a '2 other copies' button", latestName === "Lesson Plan final_final.docx" && copiesButton === "2 other copies" && expanded === 2, JSON.stringify({ latestName, copiesButton, expanded }));

  // 6. The More filters toggle reveals type and source chips; a chip narrows results.
  await page.locator('[data-testid="search-input"]').fill("");
  await page.locator('[data-testid="more-filters"]').click();
  const sourceChips = await page.locator('[aria-label="Where it came from"] .chip').count();
  await page.locator('[aria-label="Where it came from"] .chip', { hasText: "DepEd" }).click();
  await visible(page, '[data-testid="results"]');
  await page.waitForFunction(() => {
    const names = [...document.querySelectorAll('[data-testid="results"] .row-name-text')].map((el) => el.textContent);
    return names.length === 1 && names[0] === "DepEd Order 2026-014.pdf";
  }, null, { timeout: STEP_TIMEOUT_MS });
  const depedOnly = await page.locator('[data-testid="results"] .row-name-text').allTextContents();
  await capture(page, "06-filters");
  record("More filters reveals the chips and 'DepEd' narrows to one file", sourceChips === 5 && depedOnly.join() === "DepEd Order 2026-014.pdf", JSON.stringify({ sourceChips, depedOnly }));
  await page.locator('[aria-label="Where it came from"] .chip', { hasText: "DepEd" }).click();
  await page.locator('[data-testid="more-filters"]').click();

  // 7. Clicking a row opens the preview with the words inside; Open calls the backend.
  await hidden(page, '[data-testid="results"]');
  await page.locator('[data-testid="recent"] .row-body').first().click();
  const previewOk = await visible(page, '[data-testid="preview"]');
  const previewText = await page.locator('[data-testid="preview"] .preview-text').textContent();
  await page.locator('[data-testid="recent"] [data-testid="file-row"]').first().getByRole("button", { name: "Open", exact: true }).click();
  const opened = await page.evaluate(() => window.__deskMock.calls.filter((c) => c.command === "open_path").at(-1)?.args.path);
  await capture(page, "07-preview");
  record("A row opens the preview with its text and Open reaches the backend", previewOk && previewText?.includes("Photosynthesis") && String(opened).endsWith("Grade 7 Quiz 2.docx"), JSON.stringify({ previewOk, opened }));
  await page.locator('[aria-label="Close preview"]').click();

  // 8. Ticking a file shows the action bar; Move to picks a folder and moves it.
  await page.locator('[data-testid="recent"] [data-testid="file-row"] input[type=checkbox]').nth(1).check();
  const barOk = await visible(page, '[data-testid="action-bar"]');
  await page.getByRole("button", { name: "Move to..." }).click();
  const pickerOk = await visible(page, '[data-testid="folder-picker"]');
  await page.locator(".folder-option", { hasText: "Documents > Forms" }).click();
  await page.locator('[data-testid="move-confirm"]').click();
  const moved = await page.evaluate(() => window.__deskMock.calls.filter((c) => c.command === "move_files").at(-1)?.args);
  const barGone = await hidden(page, '[data-testid="action-bar"]');
  await capture(page, "08-moved");
  record("Ticking a file shows the bar; Move to... picks a folder and moves the file", barOk && pickerOk && moved?.destination?.endsWith("Forms") && barGone, JSON.stringify({ barOk, pickerOk, moved, barGone }));

  // 9. Rename happens inline with the extension locked; a bad name is refused in words.
  await page.locator('[data-testid="recent"] [data-testid="file-row"] input[type=checkbox]').first().check();
  await page.getByRole("button", { name: "Rename" }).click();
  const renameOk = await visible(page, '[data-testid="rename-input"]');
  const extLocked = await page.locator(".rename-inline .ext").textContent();
  await page.locator('[data-testid="rename-input"]').fill("bad:name");
  await page.keyboard.press("Enter");
  const refused = await visible(page, ".rename-inline .field-error");
  await page.locator('[data-testid="rename-input"]').fill("Grade 7 Science Quiz 2");
  await page.keyboard.press("Enter");
  await hidden(page, '[data-testid="rename-input"]');
  const renamed = await page.locator('[data-testid="recent"] .row-name-text').first().textContent();
  await capture(page, "09-renamed");
  record("Rename is inline with the extension locked, refuses ':' in words, then renames", renameOk && extLocked === ".docx" && refused && renamed === "Grade 7 Science Quiz 2.docx", JSON.stringify({ renameOk, extLocked, refused, renamed }));
  await page.locator('[aria-label="Clear selection"]').click().catch(() => undefined);

  // 10. Recycle Bin asks first, then confirms with a toast and Undo is in Activity.
  await page.locator('[data-testid="recent"] [data-testid="file-row"] input[type=checkbox]').last().check();
  await page.getByRole("button", { name: "Recycle Bin" }).click();
  const confirmOk = await visible(page, '[data-testid="recycle-confirm"]');
  await page.getByRole("button", { name: "Move to Recycle Bin" }).click();
  const recycled = await page.evaluate(() => window.__deskMock.calls.some((c) => c.command === "recycle_files"));
  const toastText = await page.locator(".toast").last().textContent();
  record("Recycle Bin asks first and confirms in a toast", confirmOk && recycled && toastText?.includes("Recycle Bin"), JSON.stringify({ confirmOk, recycled, toastText }));

  // 11. Activity lists changes in words, shows the failure in coral, and Undo works.
  await page.locator('[data-testid="nav-activity"]').click();
  const activityOk = (await visible(page, '[data-testid="activity-view"]')) && (await visible(page, '[data-testid="activity-row"]'));
  const activityRows = await page.locator('[data-testid="activity-row"]').count();
  const failedRow = await page.locator(".activity-row.is-failed").count();
  const firstTitle = await page.locator('[data-testid="activity-row"] strong').first().textContent();
  await page.locator('[data-testid="undo"]').first().click();
  await page.waitForFunction(() => document.querySelector(".activity-state")?.textContent?.includes("Undone"), null, { timeout: STEP_TIMEOUT_MS });
  const undone = await page.locator(".activity-state", { hasText: "Undone" }).count();
  await capture(page, "10-activity");
  record("Activity lists changes in plain words, marks the failed sort, and Undo marks the row undone", activityOk && activityRows >= 5 && failedRow === 1 && firstTitle?.startsWith("Moved") && undone >= 1, JSON.stringify({ activityRows, failedRow, firstTitle, undone }));

  // 12. Clean up shows the three cards and nothing is ticked by default.
  await page.locator('[data-testid="nav-cleanup"]').click();
  const cleanupOk = (await visible(page, '[data-testid="cleanup-view"]')) && (await visible(page, '[data-testid="cleanup-view"] section.card'));
  const cards = await page.locator('[data-testid="cleanup-view"] section.card').count();
  const tickedByDefault = await page.locator('[data-testid="cleanup-view"] input[type=checkbox]:checked').count();
  const recycleDisabled = await page.locator('[data-testid="recycle-ticked"]').isDisabled();
  await page.locator('[data-testid="desktop-list"] .button-primary').first().click();
  const suggestionMoved = await page.evaluate(() => window.__deskMock.calls.filter((c) => c.command === "move_files").at(-1)?.args.destination);
  await capture(page, "11-cleanup");
  record("Clean up shows duplicates, Desktop clutter and old downloads with nothing ticked, and a suggestion moves on click", cleanupOk && cards === 3 && tickedByDefault === 0 && recycleDisabled && String(suggestionMoved).includes("School Files"), JSON.stringify({ cards, tickedByDefault, recycleDisabled, suggestionMoved }));

  // 13. Settings can replay the walkthrough.
  await page.locator('[data-testid="nav-settings"]').click();
  const settingsOk = await visible(page, '[data-testid="settings-view"]');
  await page.locator('[data-testid="replay-tour"]').click();
  const replayed = await visible(page, '[data-testid="tour"]');
  await page.keyboard.press("Escape");
  const skipped = await hidden(page, '[data-testid="tour"]');
  await capture(page, "12-settings");
  record("Settings opens and 'Show me again' replays the walkthrough; Escape skips it", settingsOk && replayed && skipped, JSON.stringify({ settingsOk, replayed, skipped }));

  // 13a. Browsers: Find shows one notice for the mock's Edge (asks where to save); Settings lists both browsers.
  await page.locator('[data-testid="nav-find"]').click();
  const noticeText = (await visible(page, '[data-testid="browser-notice"]')) ? await page.locator('[data-testid="browser-notice"] span').textContent() : null;
  await page.locator('[data-testid="browser-notice"]').getByRole("button", { name: "How to fix" }).click();
  const browsersCard = await visible(page, '[data-testid="browsers-card"]');
  const browserRows = await page.locator('[data-testid="browser-row"]').count();
  const flagged = await page.locator('[data-testid="browser-row"].needs-change').count();
  await page.locator('[data-testid="browsers-card"]').scrollIntoViewIfNeeded();
  await capture(page, "12a-settings-browsers");
  record("Find warns about the browser that asks where to save; How to fix opens Settings with each browser's status",
    noticeText?.startsWith("Edge saves some downloads outside Downloads") && browsersCard && browserRows === 2 && flagged === 1,
    JSON.stringify({ noticeText, browsersCard, browserRows, flagged }));

  // 13b. Folders: Add a folder (native picker, mocked) lists it and asks for a restart; Remove works; Documents cannot be removed.
  await page.locator('[data-testid="nav-settings"]').click();
  await visible(page, '[data-testid="settings-view"]');
  const rootsBefore = await page.locator('[data-testid="root-row"]').count();
  await page.locator('[data-testid="add-folder"]').click();
  await page.waitForFunction((n) => document.querySelectorAll('[data-testid="root-row"]').length === n + 1, rootsBefore, { timeout: STEP_TIMEOUT_MS });
  const restartNote = await visible(page, '[data-testid="restart-note"]');
  const added = await page.locator('[data-testid="root-row"]', { hasText: "Lesson Plans" }).count();
  const alwaysKept = await page.locator('[data-testid="root-row"]', { hasText: "Documents" }).first().locator("text=always").count();
  await page.locator('[data-testid="root-row"]', { hasText: "Lesson Plans" }).getByRole("button", { name: "Remove" }).click();
  await page.waitForFunction((n) => document.querySelectorAll('[data-testid="root-row"]').length === n, rootsBefore, { timeout: STEP_TIMEOUT_MS });
  await capture(page, "12b-settings-folders");
  record("Settings: Add a folder lists it with a restart note, Documents stays, Remove takes it out", restartNote && added === 1 && alwaysKept === 1, JSON.stringify({ rootsBefore, restartNote, added, alwaysKept }));

  // 13c. Templates: the file picker (mocked) adds a template to the shelf.
  const templatesBefore = await page.locator('[data-testid="template-row"]').count();
  await page.locator('[data-testid="add-template"]').click();
  await page.waitForFunction((n) => document.querySelectorAll('[data-testid="template-row"]').length === n + 1, templatesBefore, { timeout: STEP_TIMEOUT_MS });
  const templateAdded = await page.locator('[data-testid="template-row"]', { hasText: "DLL Template" }).count();
  record("Settings: choosing a file adds it to the template shelf", templateAdded === 1, JSON.stringify({ templatesBefore, templateAdded }));

  // 14. A sorted download arriving in the background shows a toast and the new row.
  await page.evaluate(() => window.__deskMock.simulateDownload());
  await page.locator('[data-testid="nav-find"]').click();
  await visible(page, '[data-testid="find-view"]');
  const arrivedToast = await page.locator(".toast", { hasText: "Reading Assessment Q2.pdf" }).count();
  await page.waitForFunction(() => document.querySelector('[data-testid="recent"] .row-name-text')?.textContent === "Reading Assessment Q2.pdf", null, { timeout: STEP_TIMEOUT_MS });
  const newFirst = await page.locator('[data-testid="recent"] .row-name-text').first().textContent();
  record("A download filed in the background shows a toast and appears at the top of Recent", arrivedToast === 1 && newFirst === "Reading Assessment Q2.pdf", JSON.stringify({ arrivedToast, newFirst }));

  record("No error toasts during the drive", (await errorToasts(page)) === 0);
  await context.close();

  // 15. Narrow windows: the sidebar becomes a rail at 800 px and nothing overflows at 640 px.
  for (const size of [{ width: 800, height: 640 }, { width: 640, height: 600 }]) {
    const narrow = await newPage(browser, size);
    await narrow.page.keyboard.press("Escape");
    await sleep(200);
    const layout = await layoutReport(narrow.page);
    await capture(narrow.page, `13-layout-${size.width}`);
    record(`${size.width} px: the sidebar is a 64 px rail and nothing overflows`, layout.ok && !layout.docOverflow && Math.round(layout.sidebar) === 64, JSON.stringify(layout));
    await narrow.context.close();
  }
}

async function main() {
  mkdirSync(EVIDENCE_DIR, { recursive: true });
  let dev = null;
  let browser = null;
  let exitCode = 0;
  try {
    dev = startDevServer();
    const ready = await waitForServer();
    record("Doctor: dev server serves the app shell", ready, ready ? BASE_URL : `nothing serving #root on ${BASE_URL} after ${READY_TIMEOUT_MS} ms`);
    if (!ready) {
      process.stderr.write(dev.readLog());
      exitCode = 1;
    } else {
      browser = await chromium.launch({ headless: !HEADED });
      await drive(browser);
    }
    const failed = results.filter((entry) => !entry.passed);
    const summary = { stamp, baseUrl: BASE_URL, viewport: VIEWPORT, headless: !HEADED, steps: results, failedSteps: failed.length, consoleErrors, pageErrors };
    writeFileSync(join(EVIDENCE_DIR, "summary.json"), `${JSON.stringify(summary, null, 2)}\n`);
    writeFileSync(
      join(EVIDENCE_DIR, "console.log"),
      [...consoleErrors.map((line) => `[console.error] ${line}`), ...pageErrors.map((line) => `[pageerror] ${line}`)].join("\n") || "no console errors and no uncaught page errors\n",
    );
    process.stdout.write(`\nEvidence written to ${EVIDENCE_DIR}\n`);
    if (consoleErrors.length > 0) process.stderr.write(`[FAIL] ${consoleErrors.length} console error(s):\n${consoleErrors.join("\n")}\n`);
    if (pageErrors.length > 0) process.stderr.write(`[FAIL] ${pageErrors.length} uncaught page error(s):\n${pageErrors.join("\n")}\n`);
    if (failed.length > 0) process.stderr.write(`[FAIL] ${failed.length} surface check(s) failed.\n`);
    if (failed.length > 0 || pageErrors.length > 0 || consoleErrors.length > 0) exitCode = 1;
    process.stdout.write(exitCode === 0 ? "\n[UI VERIFY PASS] The interface renders and every mapped surface works.\n" : "\n[UI VERIFY FAIL] See the failures above.\n");
  } catch (error) {
    process.stderr.write(`[UI VERIFY ERROR] ${error.stack || error.message}\n`);
    exitCode = 1;
  } finally {
    if (browser) await browser.close().catch((error) => process.stderr.write(`browser close failed: ${error.message}\n`));
    if (dev) stopDevServer(dev.child);
    await sleep(500);
  }
  process.exit(exitCode);
}

await main();
