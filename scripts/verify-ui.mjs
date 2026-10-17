#!/usr/bin/env node
/**
 * Tote UI verification drive.
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

/** Program text that must never reach a toast (T-17): OS error numbers, library prefixes, debug dumps. */
const PROGRAM_TEXT = /os error|Error during|[{}`]/;

/**
 * Every toast the page ever showed, caught by an observer installed before
 * the app starts, so a toast that came and went is still on record.
 */
function watchToasts(page) {
  return page.addInitScript(() => {
    window.__toteToasts = [];
    // An init script runs before <html> exists; the document itself is a node already.
    new MutationObserver(() => {
      for (const el of document.querySelectorAll(".toast-text")) {
        if (el.dataset.logged) continue;
        el.dataset.logged = "1";
        window.__toteToasts.push(el.textContent ?? "");
      }
    }).observe(document, { childList: true, subtree: true });
  });
}

/** How many toasts the page showed, and the ones that carried program text; a page that showed none proves nothing. */
const toastAudit = (page) => page.evaluate((pattern) => ({ seen: window.__toteToasts.length, raw: window.__toteToasts.filter((text) => new RegExp(pattern).test(text)) }), PROGRAM_TEXT.source);

/** A bare page with the error hooks and the toast watcher, before any navigation. */
async function openPage(browser, viewport = VIEWPORT) {
  const context = await browser.newContext({ viewport });
  const page = await context.newPage();
  page.on("crash", () => pageErrors.push("renderer crashed"));
  page.on("console", (message) => { if (message.type() === "error") consoleErrors.push(message.text()); });
  page.on("pageerror", (error) => pageErrors.push(error.message));
  await watchToasts(page);
  return { context, page };
}

/** A fresh page on the app; `path` carries the query string the mock reads for edge states. */
async function newPage(browser, viewport = VIEWPORT, path = "/") {
  const { context, page } = await openPage(browser, viewport);
  await page.goto(`${BASE_URL}${path}`, { waitUntil: "domcontentloaded" });
  await page.waitForSelector('[data-testid="app"]', { timeout: READY_TIMEOUT_MS });
  await page.waitForSelector('[data-testid="find-view"]', { timeout: STEP_TIMEOUT_MS });
  return { context, page };
}

/**
 * The mock rejects the next `times` calls of `command` with `error`
 * (src/dev/mockBackend.ts). A screen's mount load runs twice under React's
 * StrictMode in the dev harness, so a load failure is planned twice; the
 * drive clears what is left before it clicks Try again.
 */
const planFail = (page, command, error, times = 1) => page.evaluate(([c, e, n]) => { window.__toteMockFail = { command: c, error: e, times: n }; }, [command, error, times]);
const clearFail = (page) => page.evaluate(() => { delete window.__toteMockFail; });

/** No region may scroll sideways and nothing may stick out of the window. */
async function layoutReport(page) {
  return page.evaluate(() => {
    const regions = {};
    let ok = true;
    for (const selector of ['[data-testid="sidebar"]', '[data-testid="main"]', ".search", ".row", ".todo-add", ".task"]) {
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

  // 2. Enter advances through all six steps and the last one finishes it; it is saved as done.
  const titles = [];
  for (let i = 0; i < 6; i += 1) {
    titles.push(await page.locator("#tour-title").textContent());
    if (i === 2) await capture(page, "02-tour-step-3-open");
    await page.keyboard.press("Enter");
    await sleep(120);
  }
  const tourClosed = await hidden(page, '[data-testid="tour"]');
  const saved = await page.evaluate(() => window.__toteMock.calls.some((c) => c.command === "set_settings" && c.args.settings?.tourDone === true));
  record("Enter walks through six steps, then the walkthrough closes and is remembered", titles.length === 6 && new Set(titles).size === 6 && tourClosed && saved, JSON.stringify({ titles, tourClosed, saved }));

  // 3. The Find screen shows recent downloads with source and location, and the status line.
  const rows = await page.locator('[data-testid="recent"] [data-testid="file-row"]').count();
  const firstMeta = await page.locator('[data-testid="recent"] .row-meta').first().textContent();
  const firstWhere = await page.locator('[data-testid="recent"] .row-where').first().textContent();
  const status = await page.locator('[data-testid="status-line"]').textContent();
  const pinned = await page.locator('[data-testid="pinned"] [data-testid="file-row"]').count();
  await capture(page, "03-find-home");
  record("Find shows pinned files, recent downloads with source and location, and the quiet status line",
    rows === 10 && firstMeta?.startsWith("from Messenger") && firstWhere?.includes("School Files > Word") && status?.includes("filed for you") && pinned === 1,
    JSON.stringify({ rows, firstMeta, firstWhere, status, pinned }));

  // 3b. Show a little, offer more: 10 of 14 downloads under day headings, then the rest on request.
  const days = await page.locator('[data-testid="recent-day"] .day-title').allTextContents();
  const moreText = await page.locator('[data-testid="recent-more"]').textContent();
  await page.locator('[data-testid="recent-more"]').getByRole("button").click();
  const afterMore = await page.locator('[data-testid="recent"] [data-testid="file-row"]').count();
  const moreGone = (await page.locator('[data-testid="recent-more"]').count()) === 0;
  await capture(page, "03b-recent-all");
  record("Recent downloads opens on 10 rows under day headings; Show 4 more reveals the rest and the button goes away",
    days[0] === "Today" && days.includes("Earlier") && moreText?.includes("Showing 10 of 14") && moreText?.includes("Show 4 more") && afterMore === 14 && moreGone,
    JSON.stringify({ days, moreText, afterMore, moreGone }));

  // 4. Typing a student's name finds a spreadsheet by its contents, with a snippet.
  await page.locator('[data-testid="search-input"]').fill("santos");
  await visible(page, '[data-testid="results"]');
  await page.waitForFunction(() => document.querySelectorAll('[data-testid="results"] [data-testid="file-row"]').length > 0, null, { timeout: STEP_TIMEOUT_MS });
  const hitName = await page.locator('[data-testid="results"] .row-name-text').first().textContent();
  const snippet = await page.locator('[data-testid="results"] .row-snippet mark').first().textContent();
  await capture(page, "04-search-content");
  record("Searching a student's name finds Book1.xlsx by its contents and marks the match", hitName === "Book1.xlsx" && snippet === "Santos", JSON.stringify({ hitName, snippet }));

  // 4b. Punctuation alone is no search: the pinned and recent lists stay instead of "Nothing matched yet".
  await page.locator('[data-testid="search-input"]').fill("---");
  const resultsGone = await hidden(page, '[data-testid="results"]');
  const recentBack = await visible(page, '[data-testid="recent"] [data-testid="file-row"]');
  const nothingMatched = await page.getByText("Nothing matched yet").count();
  await capture(page, "04b-search-punctuation");
  record("Typing punctuation alone keeps the pinned and recent lists instead of saying nothing matched", resultsGone && recentBack && nothingMatched === 0, JSON.stringify({ resultsGone, recentBack, nothingMatched }));

  // 4c. Punctuation beside a type chip browses the chip's files, the same as an empty box: no dead end.
  await page.locator('[data-testid="more-filters"]').click();
  await page.locator('[aria-label="File type"] .chip', { hasText: "Excel" }).click();
  const chipRows = await visible(page, '[data-testid="results"] [data-testid="file-row"]');
  const chipNames = await page.locator('[data-testid="results"] .row-name-text').allTextContents();
  const chipDeadEnd = await page.getByText("Nothing matched yet").count();
  await capture(page, "04c-punctuation-with-chip");
  record("Punctuation beside the Excel chip lists the Excel files instead of saying nothing matched", chipRows && chipNames.length > 0 && chipNames.every((name) => name.endsWith(".xlsx")) && chipDeadEnd === 0, JSON.stringify({ chipNames, chipDeadEnd }));
  await page.locator('[aria-label="File type"] .chip', { hasText: "Excel" }).click();
  await page.locator('[data-testid="more-filters"]').click();

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
  const opened = await page.evaluate(() => window.__toteMock.calls.filter((c) => c.command === "open_path").at(-1)?.args.path);
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
  const moved = await page.evaluate(() => window.__toteMock.calls.filter((c) => c.command === "move_files").at(-1)?.args);
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
  const recycled = await page.evaluate(() => window.__toteMock.calls.some((c) => c.command === "recycle_files"));
  const toastText = await page.locator(".toast").last().textContent();
  record("Recycle Bin asks first and confirms in a toast", confirmOk && recycled && toastText?.includes("Recycle Bin"), JSON.stringify({ confirmOk, recycled, toastText }));

  // 11. Activity lists changes in words, shows the failure in coral, and Undo works.
  await page.locator('[data-testid="nav-activity"]').click();
  const activityOk = (await visible(page, '[data-testid="activity-view"]')) && (await visible(page, '[data-testid="activity-row"]'));
  const activityRows = await page.locator('[data-testid="activity-row"]').count();
  const failedRow = await page.locator(".activity-row.is-failed").count();
  const firstTitle = await page.locator('[data-testid="activity-row"] strong').first().textContent();
  // A Recycle Bin row that failed says so with the reason; the file Windows removed cannot be shown.
  const refusedRow = page.locator(".activity-row.is-failed", { hasText: "too long for the Recycle Bin" }).first();
  const refusedTitle = await refusedRow.locator("strong").textContent();
  const refusedReason = await refusedRow.locator(".activity-text span").textContent();
  const refusedShow = await refusedRow.locator('[data-testid="failed-reveal"]').count();
  const goneRow = page.locator(".activity-row.is-failed", { hasText: "Windows removed it" }).first();
  const goneTitle = await goneRow.locator("strong").textContent();
  const goneShow = await goneRow.locator('[data-testid="failed-reveal"]').count();
  // A Recycle Bin batch where one file stayed counts what went, keeps Undo all, and lists the file that stayed under it with its reason.
  const partialBatch = page.locator('[data-testid="activity-batch"]', { hasText: "1 of 2" }).first();
  const partialTitle = await partialBatch.locator("strong").first().textContent();
  const partialUndo = await partialBatch.locator('[data-testid="undo-batch"]').count();
  const partialStayed = partialBatch.locator('[data-testid="activity-batch-failed"]');
  const partialStayedTitle = await partialStayed.locator("strong").textContent();
  const partialReason = await partialStayed.locator(".activity-text span").textContent();
  const partialShow = await partialStayed.locator('[data-testid="failed-reveal"]').count();
  await partialBatch.scrollIntoViewIfNeeded();
  await capture(page, "10-activity-partial-batch");
  await page.locator('[data-testid="undo"]').first().scrollIntoViewIfNeeded();
  await page.locator('[data-testid="undo"]').first().click();
  await page.waitForFunction(() => document.querySelector(".activity-state")?.textContent?.includes("Undone"), null, { timeout: STEP_TIMEOUT_MS });
  const undone = await page.locator(".activity-state", { hasText: "Undone" }).count();
  await capture(page, "10-activity");
  record(
    "Activity lists changes in plain words, marks the failed sort and the failed Recycle Bin rows with their reasons (Show the file only while the file is still there), titles a batch with one refused file as 1 of 2 with that file under it, and Undo marks the row undone",
    activityOk && activityRows >= 5 && failedRow === 4 && firstTitle?.startsWith("Moved") && refusedTitle === "Could not send Deep Quiz.docx to the Recycle Bin" && refusedReason?.includes("Move it to a shorter folder first") && refusedShow === 1 && goneTitle === "Could not send Field Trip Photos.zip to the Recycle Bin" && goneShow === 0
      && partialTitle === "Moved 1 of 2 files to the Recycle Bin" && partialUndo === 1 && partialStayedTitle === "Could not send Seat Plan Photos.jpg to the Recycle Bin" && partialReason?.includes("flash drive") && partialShow === 1 && undone >= 1,
    JSON.stringify({ activityRows, failedRow, firstTitle, refusedTitle, refusedReason, refusedShow, goneTitle, goneShow, partialTitle, partialUndo, partialStayedTitle, partialReason, partialShow, undone }),
  );

  // 11b. Two files sent to the Recycle Bin at once are one Activity row with Undo all; a filed file can be shown in its folder.
  await page.locator('[data-testid="nav-find"]').click();
  await visible(page, '[data-testid="recent"]');
  const recentChecks = page.locator('[data-testid="recent"] [data-testid="file-row"] input[type=checkbox]');
  await recentChecks.nth(0).check();
  await recentChecks.nth(1).check();
  await page.getByRole("button", { name: "Recycle Bin" }).click();
  await page.getByRole("button", { name: "Move to Recycle Bin" }).click();
  await hidden(page, '[data-testid="recycle-confirm"]');
  await page.locator('[data-testid="nav-activity"]').click();
  await visible(page, '[data-testid="activity-batch"]');
  const batchRowTitle = await page.locator('[data-testid="activity-batch"] strong').first().textContent();
  const batchUndo = await page.locator('[data-testid="activity-batch"]').first().locator('[data-testid="undo-batch"]').count();
  const filedMark = await page.locator('[data-testid="activity-row"] .op-mark.is-filed').count();
  await page.locator('[data-testid="activity-show"]').first().click();
  const revealed = await page.evaluate(() => window.__toteMock.calls.filter((c) => c.command === "reveal_path").at(-1)?.args.path);
  await capture(page, "10b-activity-batch");
  await page.locator('[data-testid="activity-batch"]').first().locator('[data-testid="undo-batch"]').click();
  await page.waitForFunction(() => document.querySelector('[data-testid="activity-batch"] .activity-state')?.textContent?.includes("Undone"), null, { timeout: STEP_TIMEOUT_MS });
  record("Two files recycled together are one Activity row with Undo all, filed rows are marked green and can be shown in their folder, and Undo all reverses the batch",
    batchRowTitle === "Moved 2 files to the Recycle Bin" && batchUndo === 1 && filedMark > 0 && String(revealed).includes("School Files"),
    JSON.stringify({ batchRowTitle, batchUndo, filedMark, revealed }));

  // 12. Clean up shows the Organize card and one card holding the three sections; nothing is ticked by default.
  await page.locator('[data-testid="nav-cleanup"]').click();
  const cleanupOk = (await visible(page, '[data-testid="cleanup-view"]')) && (await visible(page, '[data-testid="tidy-card"]'));
  const cards = (await page.locator('[data-testid="organize-card"]').count()) + (await page.locator('[data-testid="tidy-card"] section.fold').count());
  const tickedByDefault = await page.locator('[data-testid="cleanup-view"] input[type=checkbox]:checked').count();
  const recycleDisabled = await page.locator('[data-testid="recycle-ticked"]').isDisabled();
  await page.locator('[data-testid="desktop-list"]').getByRole("button", { name: "Move it there" }).first().click();
  const suggestionMoved = await page.evaluate(() => window.__toteMock.calls.filter((c) => c.command === "move_files").at(-1)?.args.destination);
  await capture(page, "11-cleanup");
  record("Clean up shows Organize, duplicates, Desktop clutter and old downloads with nothing ticked, and a suggestion moves on click", cleanupOk && cards === 4 && tickedByDefault === 0 && recycleDisabled && String(suggestionMoved).includes("School Files"), JSON.stringify({ cards, tickedByDefault, recycleDisabled, suggestionMoved }));

  // 12b. A long card starts folded with its count; opening it shows 10 rows and Show 4 more.
  const oldToggle = page.locator('[data-testid="old-card-toggle"]');
  const foldedAtStart = (await oldToggle.getAttribute("aria-expanded")) === "false" && (await page.locator('[data-testid="old-downloads-list"]').count()) === 0;
  const oldCount = await oldToggle.locator(".section-count").textContent();
  await oldToggle.click();
  const oldRows = await page.locator('[data-testid="old-downloads-list"] .suggestion').count();
  const oldMore = await page.locator('[data-testid="old-card-more"]').textContent();
  await page.locator('[data-testid="old-card-more"]').getByRole("button").click();
  const oldAll = await page.locator('[data-testid="old-downloads-list"] .suggestion').count();
  const shortOpen = (await page.locator('[data-testid="desktop-card-toggle"]').getAttribute("aria-expanded")) === "true";
  await capture(page, "11b-cleanup-folded");
  // 12c. Organize now: the plan first, then one run, a toast with Undo, and one Activity row with Undo all.
  await page.locator('[data-testid="organize-open"]').click();
  const planOk = await visible(page, '[data-testid="organize-dialog"]');
  const planTitle = await page.locator('[data-testid="organize-dialog"] #dialog-title').textContent();
  const folderRows = await page.locator('[data-testid="organize-dialog"] .organize-folder').count();
  const gradeFolder = await page.locator('[data-testid="organize-dialog"] .organize-folder-name', { hasText: "Grade 7" }).count();
  await page.locator('[data-testid="organize-every-file"]').click();
  const everyFile = await page.locator('[data-testid="organize-dialog"] .organize-files li').count();
  await capture(page, "11c-organize-plan");
  await page.locator('[data-testid="organize-confirm"]').click();
  const planClosed = await hidden(page, '[data-testid="organize-dialog"]');
  const organizedToast = await page.locator(".toast", { hasText: "Organized" }).count();
  const toastUndo = await page.locator(".toast", { hasText: "Organized" }).locator('[data-testid="toast-undo"]').count();
  const cardGone = await hidden(page, '[data-testid="organize-card"]');
  await page.locator('[data-testid="nav-activity"]').click();
  await page.waitForSelector('[data-testid="activity-batch"]', { timeout: STEP_TIMEOUT_MS });
  const batchTitle = await page.locator('[data-testid="activity-batch"] strong').first().textContent();
  await page.locator('[data-testid="undo-batch"]').first().click();
  await page.waitForFunction(() => document.querySelector('[data-testid="activity-batch"] .activity-state')?.textContent?.includes("Undone"), null, { timeout: STEP_TIMEOUT_MS });
  await capture(page, "11d-organize-activity");
  record("Organize shows the plan with a Grade 7 folder and every file, runs once, offers Undo in the toast, and Activity shows one row whose Undo all reverses it",
    planOk && /^Organize \d+ files into \d+ folders\?$/.test(planTitle ?? "") && folderRows > 0 && gradeFolder === 1 && everyFile > 0 && planClosed && organizedToast === 1 && toastUndo === 1 && cardGone && batchTitle?.startsWith("Organized"),
    JSON.stringify({ planTitle, folderRows, gradeFolder, everyFile, organizedToast, toastUndo, cardGone, batchTitle }));
  await page.locator('[data-testid="nav-cleanup"]').click();
  await visible(page, '[data-testid="tidy-card"]');

  record("A long Clean up card starts folded with its count and opens 10 at a time; short cards start open",
    foldedAtStart && oldCount === "14" && oldRows === 10 && oldMore?.includes("Show 4 more") && oldAll === 14 && shortOpen,
    JSON.stringify({ foldedAtStart, oldCount, oldRows, oldMore, oldAll, shortOpen }));

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

  // 13d. Organize words: the standard list shows; adding a subject is saved when she leaves the field.
  await page.locator('[data-testid="subject-rules"]').scrollIntoViewIfNeeded();
  const ruleRows = await page.locator('[data-testid="subject-rule"]').count();
  await page.locator('[data-testid="add-subject"]').click();
  const last = page.locator('[data-testid="subject-rule"]').last();
  await last.locator("input").first().fill("Robotics");
  await last.locator("input").nth(1).fill("Arduino, robot");
  await last.locator("input").nth(1).blur();
  const savedRules = await page.evaluate(() => window.__toteMock.calls.filter((c) => c.command === "set_settings").at(-1)?.args.settings?.subjectRules ?? []);
  const robotics = savedRules.find((r) => r.folder === "Robotics");
  await capture(page, "12c-settings-subject-words");
  record("Settings: the subject word list shows the standard rules, and a new subject is saved when she leaves the field",
    ruleRows === 4 && robotics?.words?.join(",") === "arduino,robot" && savedRules.length === 5,
    JSON.stringify({ ruleRows, robotics, saved: savedRules.length }));

  // 14. A sorted download arriving in the background shows a toast and the new row.
  await page.evaluate(() => window.__toteMock.simulateDownload());
  await page.locator('[data-testid="nav-find"]').click();
  await visible(page, '[data-testid="find-view"]');
  const arrivedToast = await page.locator(".toast", { hasText: "Reading Assessment Q2.pdf" }).count();
  await page.waitForFunction(() => document.querySelector('[data-testid="recent"] .row-name-text')?.textContent === "Reading Assessment Q2.pdf", null, { timeout: STEP_TIMEOUT_MS });
  const newFirst = await page.locator('[data-testid="recent"] .row-name-text').first().textContent();
  record("A download filed in the background shows a toast and appears at the top of Recent", arrivedToast === 1 && newFirst === "Reading Assessment Q2.pdf", JSON.stringify({ arrivedToast, newFirst }));

  // 14b. The sidebar folds into the icon rail and back; the menu keeps its names for screen readers.
  const sidebarWidth = () => page.evaluate(() => Math.round(document.querySelector('[data-testid="sidebar"]').getBoundingClientRect().width));
  const openWidth = await sidebarWidth();
  // The active look is one sliding canvas tab: once it settles it sits on the active item.
  const canvasTab = () => {
    const tab = document.querySelector('[data-testid="nav-indicator"]');
    const item = document.querySelector('[data-testid="nav-find"]');
    return getComputedStyle(tab).backgroundColor === "rgb(238, 242, 238)" && Math.abs(tab.getBoundingClientRect().top - item.getBoundingClientRect().top) < 1;
  };
  const activeTab = await page.waitForFunction(canvasTab, null, { timeout: STEP_TIMEOUT_MS }).then(() => true, () => false);
  await page.locator('[data-testid="sidebar-toggle"]').click();
  // The column animates; wait for it to settle before measuring.
  await page.waitForFunction(() => Math.round(document.querySelector('[data-testid="sidebar"]').getBoundingClientRect().width) === 64, null, { timeout: STEP_TIMEOUT_MS }).catch(() => undefined);
  const railWidth = await sidebarWidth();
  const railNamed = await page.getByRole("button", { name: "Clean up", exact: true }).count();
  await capture(page, "15-sidebar-rail");
  await page.locator('[data-testid="sidebar-toggle"]').click();
  await page.waitForFunction((width) => Math.round(document.querySelector('[data-testid="sidebar"]').getBoundingClientRect().width) === width, openWidth, { timeout: STEP_TIMEOUT_MS }).catch(() => undefined);
  const reopened = await sidebarWidth();
  record("The sidebar folds to a 64 px rail and opens again; the sliding canvas tab sits on the active item and the rail keeps menu names",
    openWidth > 200 && railWidth === 64 && reopened === openWidth && railNamed === 1 && activeTab,
    JSON.stringify({ openWidth, railWidth, reopened, railNamed, activeTab }));

  // 16. New version: the daily check offers 0.3.0, the banner shows on every
  // screen, Update now calls the native installer, Later hides it, and the
  // Settings switch turns the offer off.
  await page.evaluate(() => window.__toteMock.offerUpdate("0.3.0"));
  await page.evaluate(() => window.dispatchEvent(new Event("focus")));
  const bannerShown = await visible(page, '[data-testid="update-banner"]').then(() => true, () => false);
  const bannerText = bannerShown ? await page.locator('[data-testid="update-banner"] .update-title').textContent() : null;
  await capture(page, "16-update-banner");
  await page.locator('[data-testid="nav-cleanup"]').click();
  const onCleanup = await page.locator('[data-testid="update-banner"]').count();
  const installsBefore = await page.evaluate(() => window.__toteMock.calls.filter((c) => c.command === "install_update").length);
  await page.locator('[data-testid="update-now"]').click();
  await page.waitForFunction((n) => window.__toteMock.calls.filter((c) => c.command === "install_update").length > n, installsBefore, { timeout: STEP_TIMEOUT_MS }).catch(() => undefined);
  const installed = await page.evaluate(() => window.__toteMock.calls.filter((c) => c.command === "install_update").length) - installsBefore;
  await page.locator('[data-testid="update-later"]').click();
  const hiddenAfterLater = await page.locator('[data-testid="update-banner"]').count();
  await page.locator('[data-testid="nav-settings"]').click();
  const switchOn = await page.locator('[data-testid="check-updates"]').isChecked();
  await page.locator('[data-testid="check-updates"]').click();
  await page.waitForFunction(() => window.__toteMock.calls.some((c) => c.command === "set_settings" && c.args.settings.checkUpdates === false), null, { timeout: STEP_TIMEOUT_MS }).catch(() => undefined);
  const offWhenSwitchedOff = await page.evaluate(() => window.__toteMock.invoke("check_update", {}));
  await page.locator('[data-testid="check-updates"]').click();
  await page.locator('[data-testid="nav-find"]').click();
  record("A new version shows a banner on every screen; Update now runs the installer, Later hides it, and the Settings switch stops the offer",
    bannerShown && bannerText === "Tote 0.3.0 is ready" && onCleanup === 1 && installed === 1 && hiddenAfterLater === 0 && switchOn && offWhenSwitchedOff === null,
    JSON.stringify({ bannerText, onCleanup, installed, hiddenAfterLater, switchOn, offWhenSwitchedOff }));

  // 17. To do: the New marker, the empty list, adding with Enter and with Add,
  // a due date's group, tick with Undo, a monthly repeat, the quarter option,
  // attaching a file, delete with Undo.
  const markerBefore = await page.locator('[data-testid="nav-todo"] .nav-new').count();
  await page.locator('[data-testid="nav-todo"]').click();
  const todoOpen = await visible(page, '[data-testid="todo-view"]');
  const emptyShown = await visible(page, '[data-testid="todo-empty"]');
  const markerGone = await page.waitForFunction(() => !document.querySelector('[data-testid="nav-todo"] .nav-new'), null, { timeout: STEP_TIMEOUT_MS }).then(() => true, () => false);
  const seenSaved = await page.evaluate(() => window.__toteMock.calls.some((c) => c.command === "set_settings" && c.args.settings?.todoSeen === true));
  await capture(page, "17-todo-empty");
  record("To do opens from the sidebar with a New marker that goes once opened, and shows the empty state", markerBefore === 1 && todoOpen && emptyShown && markerGone && seenSaved, JSON.stringify({ markerBefore, todoOpen, emptyShown, markerGone, seenSaved }));

  // 17b. Enter and Add both add; a due date puts the task under Today.
  const today = await page.evaluate(() => {
    const d = new Date();
    return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
  });
  const taskCount = (n) => page.waitForFunction((want) => document.querySelectorAll('[data-testid="task-row"]').length === want, n, { timeout: STEP_TIMEOUT_MS }).then(() => true, () => false);
  await page.locator('[data-testid="todo-input"]').fill("Print the Grade 7 quiz");
  await page.keyboard.press("Enter");
  const addedByEnter = await taskCount(1);
  await page.locator('[data-testid="todo-input"]').fill("Submit SF2 for October");
  await page.locator('[data-testid="todo-due"]').fill(today);
  await page.locator('[data-testid="todo-add"]').click();
  const addedByButton = await taskCount(2);
  const groupTitles = await page.locator('[data-testid="todo-group"] h2').allTextContents();
  const cleared = (await page.locator('[data-testid="todo-input"]').inputValue()) === "";
  const focusBack = await page.evaluate(() => document.activeElement?.getAttribute("data-testid"));
  await capture(page, "17b-todo-added");
  record("Enter and Add both add a task; the dated one sits under Today, the other under No date, and the box is cleared with focus kept",
    addedByEnter && addedByButton && groupTitles.join(",") === "Today,No date" && cleared && focusBack === "todo-input",
    JSON.stringify({ groupTitles, cleared, focusBack }));

  // 17c. Tick: Done appears and the toast's Undo puts it back.
  const sf2 = () => page.locator('[data-testid="todo-group"] [data-testid="task-row"]', { hasText: "Submit SF2" });
  await sf2().locator('[data-testid="task-check"]').click();
  const doneShown = await visible(page, '[data-testid="todo-done"]');
  const doneToast = page.locator(".toast", { hasText: "Done." });
  const toastHasUndo = (await doneToast.locator('[data-testid="toast-undo"]').count()) === 1;
  await capture(page, "17c-todo-done");
  await doneToast.locator('[data-testid="toast-undo"]').click();
  const doneGone = await hidden(page, '[data-testid="todo-done"]');
  const backOnList = await sf2().count();
  record("Ticking a task moves it to Done with Undo in the toast; Undo puts it back on the list", doneShown && toastHasUndo && doneGone && backOnList === 1, JSON.stringify({ doneShown, toastHasUndo, doneGone, backOnList }));

  // 17d. Every quarter waits for quarter dates; a monthly task makes its next copy.
  await sf2().locator('[data-testid="task-open"]').click();
  await visible(page, '[data-testid="task-details"]');
  const quarterDisabled = await page.locator('[data-testid="task-repeat"] option[value="quarter"]').evaluate((o) => o.disabled);
  await page.locator('[data-testid="task-repeat"]').selectOption("month");
  await page.waitForFunction(() => window.__toteMock.calls.some((c) => c.command === "update_task" && c.args.repeat === "month"), null, { timeout: STEP_TIMEOUT_MS }).catch(() => undefined);
  await capture(page, "17d-todo-details");
  await sf2().first().locator('[data-testid="task-check"]').click();
  const nextToast = await page.locator(".toast", { hasText: "Next one is due" }).textContent({ timeout: STEP_TIMEOUT_MS }).catch(() => null);
  const copyMeta = await page.waitForFunction(() => [...document.querySelectorAll('[data-testid="todo-group"] [data-testid="task-row"]')].some((r) => r.textContent?.includes("Submit SF2") && r.textContent?.includes("every month")), null, { timeout: STEP_TIMEOUT_MS }).then(() => true, () => false);
  await capture(page, "17e-todo-repeat");
  record("Every quarter is unavailable without quarter dates; a monthly task ticked off makes its next copy and the toast names the day",
    quarterDisabled && Boolean(nextToast) && copyMeta, JSON.stringify({ quarterDisabled, nextToast, copyMeta }));

  // 17e. Attach a file through the index search, then open it from the task.
  const print = () => page.locator('[data-testid="task-row"]', { hasText: "Print the Grade 7 quiz" });
  await print().locator('[data-testid="task-open"]').click();
  await print().locator('[data-testid="task-attach"]').click();
  const dialogOpen = await visible(page, '[data-testid="attach-dialog"]');
  await page.locator('[data-testid="attach-search"]').fill("grade");
  const optionShown = await visible(page, '[data-testid="attach-option"]');
  await capture(page, "17f-todo-attach-dialog");
  const picked = await page.locator('[data-testid="attach-option"] .attach-name').first().textContent();
  await page.locator('[data-testid="attach-option"]').first().click();
  const dialogClosed = await hidden(page, '[data-testid="attach-dialog"]');
  const attachedName = await print().locator('[data-testid="task-file"] .task-file-name').first().textContent({ timeout: STEP_TIMEOUT_MS }).catch(() => null);
  const opensBefore = await page.evaluate(() => window.__toteMock.calls.filter((c) => c.command === "open_path").length);
  await print().locator('[data-testid="task-file-open"]').click();
  await page.waitForFunction((n) => window.__toteMock.calls.filter((c) => c.command === "open_path").length > n, opensBefore, { timeout: STEP_TIMEOUT_MS }).catch(() => undefined);
  const fileOpened = (await page.evaluate(() => window.__toteMock.calls.filter((c) => c.command === "open_path").length)) > opensBefore;
  const fileMeta = await print().locator(".task-meta").textContent();
  await capture(page, "17g-todo-attached");
  record("Attach a file searches the index, attaches the picked file, and Open opens it from the task",
    dialogOpen && optionShown && dialogClosed && attachedName === picked && fileOpened && Boolean(fileMeta?.includes("1 file")),
    JSON.stringify({ picked, attachedName, fileOpened, fileMeta }));

  // 17f. Delete at once, Undo from the toast.
  const tasksBefore = await page.locator('[data-testid="task-row"]').count();
  await print().locator('[data-testid="task-delete"]').click();
  const taskRemoved = await taskCount(tasksBefore - 1);
  await page.locator(".toast", { hasText: "Task deleted." }).locator('[data-testid="toast-undo"]').click();
  const taskRestored = await taskCount(tasksBefore);
  record("Delete task removes it at once and the toast's Undo brings it back", taskRemoved && taskRestored, JSON.stringify({ tasksBefore, taskRemoved, taskRestored }));
  await page.locator('[data-testid="nav-find"]').click();

  record("No error toasts during the drive", (await errorToasts(page)) === 0);
  const driveToasts = await toastAudit(page);
  record("No toast in the drive carried program text (os error, Error during, a debug dump)", driveToasts.seen > 0 && driveToasts.raw.length === 0, JSON.stringify(driveToasts));
  await context.close();

  // 15. Narrow windows: the sidebar becomes a rail at 800 px and nothing overflows at 640 px.
  for (const size of [{ width: 800, height: 640 }, { width: 640, height: 600 }]) {
    const narrow = await newPage(browser, size);
    await narrow.page.keyboard.press("Escape");
    await sleep(200);
    const layout = await layoutReport(narrow.page);
    await capture(narrow.page, `13-layout-${size.width}`);
    record(`${size.width} px: the sidebar is a 64 px rail and nothing overflows`, layout.ok && !layout.docOverflow && Math.round(layout.sidebar) === 64, JSON.stringify(layout));
    await narrow.page.locator('[data-testid="nav-todo"]').click();
    await narrow.page.locator('[data-testid="todo-input"]').fill("Submit SF2 for October");
    await narrow.page.keyboard.press("Enter");
    await narrow.page.waitForSelector('[data-testid="task-row"]', { timeout: STEP_TIMEOUT_MS });
    await narrow.page.locator('[data-testid="task-open"]').first().click();
    await narrow.page.waitForSelector('[data-testid="task-details"]', { timeout: STEP_TIMEOUT_MS });
    const todoLayout = await layoutReport(narrow.page);
    await capture(narrow.page, `13-layout-todo-${size.width}`);
    record(`${size.width} px: To do with a task open fits without sideways scrolling`, todoLayout.ok && !todoLayout.docOverflow, JSON.stringify(todoLayout));
    await narrow.context.close();
  }

  // 18. Edge states a real start can report (the mock reads them from the
  // query string): the index was rebuilt, with or without her settings, and
  // the To do store could not be opened.
  const kept = await newPage(browser, VIEWPORT, "/?rebuilt=kept");
  const keptNotice = await visible(kept.page, '.toast:not(.toast-error):has-text("Your pins, templates and settings were kept.")');
  await capture(kept.page, "18-rebuilt-kept");
  await kept.context.close();
  const lost = await newPage(browser, VIEWPORT, "/?rebuilt=lost");
  const lostNotice = await visible(lost.page, '.toast-error:has-text("could not be kept. Check Settings.")');
  const lostText = await lost.page.locator(".toast-error .toast-text").first().textContent().catch(() => null);
  await capture(lost.page, "18-rebuilt-lost");
  await lost.context.close();
  record("A start after an index rebuild shows one notice: quiet when her pins, templates and settings were kept, coral when they were lost",
    keptNotice && lostNotice && Boolean(lostText?.startsWith("Tote's index had to be rebuilt. Your files were not touched.")),
    JSON.stringify({ keptNotice, lostNotice, lostText }));

  const unavailable = await newPage(browser, VIEWPORT, "/?tasks=unavailable");
  await unavailable.page.keyboard.press("Escape");
  await unavailable.page.locator('[data-testid="nav-todo"]').click();
  const unavailableNotice = await visible(unavailable.page, '[data-testid="todo-unavailable"]');
  const unavailableText = await unavailable.page.locator('[data-testid="todo-unavailable"]').textContent().catch(() => null);
  const addOff = await unavailable.page.locator('[data-testid="todo-add"]').isDisabled();
  const inputOff = await unavailable.page.locator('[data-testid="todo-input"]').isDisabled();
  const noSkeleton = (await unavailable.page.locator('[data-testid="todo-view"] .skeleton-row').count()) === 0;
  const noListCall = await unavailable.page.evaluate(() => !window.__toteMock.calls.some((c) => c.command === "list_tasks"));
  await capture(unavailable.page, "18-todo-unavailable");
  await unavailable.context.close();
  record("A To do store that could not be opened shows the plain sentence instead of the skeleton, with the add row off and no list request",
    unavailableNotice && Boolean(unavailableText?.includes("Restart Tote. If this keeps happening, tell Lex.")) && addOff && inputOff && noSkeleton && noListCall,
    JSON.stringify({ unavailableNotice, unavailableText, addOff, inputOff, noSkeleton, noListCall }));

  await driveFailures(browser);
}

/**
 * 19. Failure injection (T-17, T-18, T-20, T-37): the mock rejects the next
 * call of one command, and each screen must show one plain sentence with a
 * way on: never program text, never a loading state for good, never a toast
 * that claims more than happened.
 */
async function driveFailures(browser) {
  const DB_READ = { code: "db", message: "Tote could not read its list right now. Try again in a moment." };
  const DB_WRITE = { code: "db", message: "Tote could not save that. Try again in a moment." };
  const IN_USE = { code: "inUse", message: "That file is open in another program. Close it there, then try again." };
  const RAW = { code: "io", message: "The process cannot access the file because it is being used by another process. (os error 32)" };
  const lineText = (page, testId) => page.locator(`[data-testid="${testId}"] span`).textContent({ timeout: STEP_TIMEOUT_MS }).catch(() => null);

  // 19a. The app shell: get_app_info fails before the first screen; Try again loads it.
  const shell = await openPage(browser);
  await shell.page.addInitScript((error) => { window.__toteMockFail = { command: "get_app_info", error, times: 2 }; }, DB_READ);
  await shell.page.goto(BASE_URL, { waitUntil: "domcontentloaded" });
  const shellLine = await visible(shell.page, '[data-testid="app-load-failed"]');
  const shellText = await lineText(shell.page, "app-load-failed");
  const noStarting = (await shell.page.getByText("Starting Tote...").count()) === 0;
  await capture(shell.page, "19a-shell-load-failed");
  await clearFail(shell.page);
  await shell.page.locator('[data-testid="app-load-failed-retry"]').click();
  const shellRecovered = await visible(shell.page, '[data-testid="find-view"]');
  record("App shell: a failed first load shows one plain sentence with Try again instead of Starting Tote for good, and Try again brings the Find screen",
    shellLine && shellText === DB_READ.message && noStarting && shellRecovered, JSON.stringify({ shellLine, shellText, noStarting, shellRecovered }));
  await shell.context.close();

  const { context, page } = await newPage(browser);
  await page.keyboard.press("Escape");
  await hidden(page, '[data-testid="tour"]');

  // 19b. Find: a failed search keeps the last results and says search is not available, with Try again.
  await page.locator('[data-testid="search-input"]').fill("grade");
  await page.waitForFunction(() => document.querySelectorAll('[data-testid="results"] [data-testid="file-row"]').length > 0, null, { timeout: STEP_TIMEOUT_MS });
  const rowsBefore = await page.locator('[data-testid="results"] [data-testid="file-row"]').count();
  await planFail(page, "find", DB_READ);
  await page.locator('[data-testid="search-input"]').fill("grade 7");
  const searchLine = await visible(page, '[data-testid="search-failed"]');
  const searchText = await lineText(page, "search-failed");
  const rowsKept = await page.locator('[data-testid="results"] [data-testid="file-row"]').count();
  const noNothingMatched = (await page.getByText("Nothing matched yet").count()) === 0;
  const noSearchToast = (await errorToasts(page)) === 0;
  await capture(page, "19b-search-failed");
  await page.locator('[data-testid="search-failed-retry"]').click();
  const searchBack = await hidden(page, '[data-testid="search-failed"]');
  const rowsAfter = await page.locator('[data-testid="results"] [data-testid="file-row"]').count();
  record("Find: a failed search keeps the last results and shows one quiet line with Try again instead of Nothing matched; Try again searches again",
    searchLine && searchText === "Search is not available right now. Try again in a moment." && rowsKept === rowsBefore && rowsBefore > 0 && noNothingMatched && noSearchToast && searchBack && rowsAfter > 0,
    JSON.stringify({ searchLine, searchText, rowsBefore, rowsKept, noNothingMatched, noSearchToast, searchBack, rowsAfter }));
  await page.locator('[data-testid="search-input"]').fill("");
  await hidden(page, '[data-testid="results"]');
  await visible(page, '[data-testid="recent"] [data-testid="file-row"]');

  // 19c. Rename: a file open in another program is refused under the field in plain words.
  await page.locator('[data-testid="recent"] [data-testid="file-row"] input[type=checkbox]').first().check();
  await page.getByRole("button", { name: "Rename" }).click();
  await visible(page, '[data-testid="rename-input"]');
  await planFail(page, "rename_file", IN_USE);
  await page.locator('[data-testid="rename-input"]').fill("Quiz 2 for printing");
  await page.keyboard.press("Enter");
  const renameLine = await visible(page, ".rename-inline .field-error");
  const renameText = await page.locator(".rename-inline .field-error").textContent().catch(() => null);
  await capture(page, "19c-rename-in-use");
  record("Rename: a file open in another program is refused under the field with the plain sentence and its next step", renameLine && renameText === IN_USE.message, JSON.stringify({ renameLine, renameText }));
  await page.locator('[aria-label="Cancel rename"]').click();
  await hidden(page, '[data-testid="rename-input"]');

  // 19d. Move: program text from the native side never reaches the toast; the code's fixed sentence shows instead.
  await page.getByRole("button", { name: "Move to..." }).click();
  await visible(page, '[data-testid="folder-picker"]');
  await page.locator(".folder-option", { hasText: "Documents > Forms" }).click();
  await planFail(page, "move_files", RAW);
  await page.locator('[data-testid="move-confirm"]').click();
  const moveToast = await page.locator(".toast-error .toast-text").last().textContent({ timeout: STEP_TIMEOUT_MS }).catch(() => null);
  record("Move: an os error from the native side shows as the fixed plain sentence for the code, never the program text",
    moveToast === "Windows could not finish that. Try again in a moment." && !PROGRAM_TEXT.test(moveToast ?? ""), JSON.stringify({ moveToast }));
  await page.locator('[aria-label="Clear selection"]').click().catch(() => undefined);

  // 19e. Toast Undo: one of two moves cannot go back; the toast says what went back and names the file with its reason.
  const checks = page.locator('[data-testid="recent"] [data-testid="file-row"] input[type=checkbox]');
  await checks.nth(0).check();
  await checks.nth(1).check();
  await page.getByRole("button", { name: "Move to..." }).click();
  await visible(page, '[data-testid="folder-picker"]');
  await page.locator(".folder-option", { hasText: "Documents > Forms" }).click();
  await page.locator('[data-testid="move-confirm"]').click();
  const movedToast = page.locator(".toast", { hasText: "Moved 2 files to" });
  const movedText = await movedToast.locator(".toast-text").textContent({ timeout: STEP_TIMEOUT_MS }).catch(() => null);
  await planFail(page, "undo", IN_USE);
  await movedToast.locator('[data-testid="toast-undo"]').click();
  const partialText = await page.locator(".toast-error .toast-text", { hasText: "Put back 1 of 2" }).textContent({ timeout: STEP_TIMEOUT_MS }).catch(() => null);
  await capture(page, "19e-undo-partial");
  record("Toast Undo: when one of two moves cannot go back, the toast says Put back 1 of 2 and names the file with the reason; the move toast names the folder",
    movedText === "Moved 2 files to Documents > Forms." && /^Put back 1 of 2\. [^:]+\.(docx|pdf|xlsx): That file is open in another program\. Close it there, then try again\.$/.test(partialText ?? ""),
    JSON.stringify({ movedText, partialText }));

  // 19j. Find's home list: a refresh that fails after a Move keeps the Pinned and Recent lists and says so above them, with Try again.
  await checks.nth(0).check();
  await page.getByRole("button", { name: "Move to..." }).click();
  await visible(page, '[data-testid="folder-picker"]');
  await page.locator(".folder-option", { hasText: "School Files > Word" }).first().click();
  await planFail(page, "find", DB_READ);
  await page.locator('[data-testid="move-confirm"]').click();
  const homeLine = await visible(page, '[data-testid="find-refresh-failed"]');
  const homeText = await lineText(page, "find-refresh-failed");
  const homeRows = await page.locator('[data-testid="recent"] [data-testid="file-row"]').count();
  const homeNoLoadLine = (await page.locator('[data-testid="find-load-failed"]').count()) === 0;
  // The failed refresh is said in the line only; the error toast from 19e's partial Undo may still be up.
  const homeNoErrorToast = (await page.locator(".toast-text", { hasText: DB_READ.message }).count()) === 0;
  await capture(page, "19j-home-refresh-failed");
  await page.locator('[data-testid="find-refresh-failed-retry"]').click();
  const homeBack = await hidden(page, '[data-testid="find-refresh-failed"]');
  record("Find: a refresh that fails after a Move keeps the Pinned and Recent lists and says so in one quiet line above them with Try again, which clears it",
    homeLine && homeText === "Tote could not refresh this list, so it may be out of date. Try again in a moment." && homeRows > 0 && homeNoLoadLine && homeNoErrorToast && homeBack,
    JSON.stringify({ homeLine, homeText, homeRows, homeNoLoadLine, homeNoErrorToast, homeBack }));

  // 19f. To do: a failed load shows the sentence with Try again instead of a skeleton for good.
  await planFail(page, "list_tasks", DB_READ, 2);
  await page.locator('[data-testid="nav-todo"]').click();
  const todoLine = await visible(page, '[data-testid="todo-load-failed"]');
  const todoText = await lineText(page, "todo-load-failed");
  const todoNoSkeleton = (await page.locator('[data-testid="todo-view"] .skeleton-row').count()) === 0;
  await capture(page, "19f-todo-load-failed");
  await clearFail(page);
  await page.locator('[data-testid="todo-load-failed-retry"]').click();
  const todoBack = (await hidden(page, '[data-testid="todo-load-failed"]')) && (await visible(page, '[data-testid="todo-empty"]'));
  record("To do: a failed load shows one plain sentence with Try again instead of the skeleton, and Try again loads the list",
    todoLine && todoText === DB_READ.message && todoNoSkeleton && todoBack, JSON.stringify({ todoLine, todoText, todoNoSkeleton, todoBack }));

  // 19g. Clean up: the same for its report.
  await planFail(page, "cleanup_report", DB_READ, 2);
  await page.locator('[data-testid="nav-cleanup"]').click();
  const cleanupLine = await visible(page, '[data-testid="cleanup-load-failed"]');
  const cleanupText = await lineText(page, "cleanup-load-failed");
  const cleanupNoSkeleton = (await page.locator('[data-testid="cleanup-view"] .skeleton-row').count()) === 0;
  await capture(page, "19g-cleanup-load-failed");
  await clearFail(page);
  await page.locator('[data-testid="cleanup-load-failed-retry"]').click();
  const cleanupBack = (await hidden(page, '[data-testid="cleanup-load-failed"]')) && (await visible(page, '[data-testid="tidy-card"]'));
  record("Clean up: a failed load shows one plain sentence with Try again instead of the skeleton, and Try again loads the cards",
    cleanupLine && cleanupText === DB_READ.message && cleanupNoSkeleton && cleanupBack, JSON.stringify({ cleanupLine, cleanupText, cleanupNoSkeleton, cleanupBack }));

  // 19h. Activity: a failed load, then an Undo the native side refuses.
  await planFail(page, "list_activity", DB_READ, 2);
  await page.locator('[data-testid="nav-activity"]').click();
  const activityLine = await visible(page, '[data-testid="activity-load-failed"]');
  const activityText = await lineText(page, "activity-load-failed");
  await clearFail(page);
  await page.locator('[data-testid="activity-load-failed-retry"]').click();
  const activityBack = (await hidden(page, '[data-testid="activity-load-failed"]')) && (await visible(page, '[data-testid="undo"]'));
  const undoneBefore = await page.locator(".activity-state", { hasText: "Undone" }).count();
  await planFail(page, "undo", IN_USE);
  await page.locator('[data-testid="undo"]').first().click();
  // The exact sentence, on its own: the partial Undo toast from 19e may still be up and contains it too.
  const undoToast = await page.waitForFunction((want) => [...document.querySelectorAll(".toast-error .toast-text")].some((el) => el.textContent === want), IN_USE.message, { timeout: STEP_TIMEOUT_MS }).then(() => IN_USE.message, () => null);
  const undoneAfter = await page.locator(".activity-state", { hasText: "Undone" }).count();
  await capture(page, "19h-activity-undo-failed");
  record("Activity: a failed load shows the sentence with Try again, and an Undo the native side refuses says why in one sentence and leaves the row as it was",
    activityLine && activityText === DB_READ.message && activityBack && undoToast === IN_USE.message && undoneAfter === undoneBefore,
    JSON.stringify({ activityLine, activityText, activityBack, undoToast, undoneBefore, undoneAfter }));

  // 19i. Settings: a save that failed says so and the switch goes back to the kept value.
  await page.locator('[data-testid="nav-settings"]').click();
  await visible(page, '[data-testid="settings-view"]');
  const switchBefore = await page.locator('[data-testid="check-updates"]').isChecked();
  await planFail(page, "set_settings", DB_WRITE);
  await page.locator('[data-testid="check-updates"]').click();
  const saveToast = await page.locator(".toast-error .toast-text", { hasText: "could not save" }).textContent({ timeout: STEP_TIMEOUT_MS }).catch(() => null);
  const switchReverted = await page.waitForFunction((want) => document.querySelector('[data-testid="check-updates"]').checked === want, switchBefore, { timeout: STEP_TIMEOUT_MS }).then(() => true, () => false);
  await capture(page, "19i-settings-save-failed");
  record("Settings: a save that failed shows the save sentence and the switch goes back to the kept value",
    switchBefore && saveToast === DB_WRITE.message && switchReverted, JSON.stringify({ switchBefore, saveToast, switchReverted }));

  // 19k. Recycle Bin, partly refused: the toast names the file that stayed and its Undo brings back the file that went.
  await page.locator('[data-testid="nav-find"]').click();
  await visible(page, '[data-testid="search-input"]');
  await page.locator('[data-testid="search-input"]').fill("quiz");
  await page.waitForFunction(() => document.querySelectorAll('[data-testid="results"] [data-testid="file-row"]').length === 2, null, { timeout: STEP_TIMEOUT_MS });
  const quizChecks = page.locator('[data-testid="results"] [data-testid="file-row"] input[type=checkbox]');
  await quizChecks.nth(0).check();
  await quizChecks.nth(1).check();
  await page.getByRole("button", { name: "Recycle Bin" }).click();
  await visible(page, '[data-testid="recycle-confirm"]');
  await page.getByRole("button", { name: "Move to Recycle Bin" }).click();
  const partialRecycle = page.locator(".toast-error", { hasText: "Moved 1 of 2 to the Recycle Bin" });
  const partialRecycleText = await partialRecycle.locator(".toast-text").textContent({ timeout: STEP_TIMEOUT_MS }).catch(() => null);
  const partialRecycleUndo = await partialRecycle.locator('[data-testid="toast-undo"]').count();
  const rowsAfterRecycle = await page.locator('[data-testid="results"] [data-testid="file-row"]').count();
  await capture(page, "19k-recycle-partial");
  await partialRecycle.locator('[data-testid="toast-undo"]').click();
  const putBackText = await page.locator(".toast:not(.toast-error) .toast-text", { hasText: "Put back" }).textContent({ timeout: STEP_TIMEOUT_MS }).catch(() => null);
  const keptBack = await page.waitForFunction(() => [...document.querySelectorAll('[data-testid="results"] .row-name-text')].some((el) => el.textContent === "Grade 7 Quiz 2.docx"), null, { timeout: STEP_TIMEOUT_MS }).then(() => true, () => false);
  record("Recycle Bin: when one of two files is refused, the toast says Moved 1 of 2 and names the file with the reason, and its Undo brings the file that went back",
    /^Moved 1 of 2 to the Recycle Bin\. Deep Quiz\.docx: This file's folder name is too long for the Recycle Bin\. Move it to a shorter folder first\.$/.test(partialRecycleText ?? "") && partialRecycleUndo === 1 && rowsAfterRecycle === 1 && putBackText === "Put back where it was." && keptBack,
    JSON.stringify({ partialRecycleText, partialRecycleUndo, rowsAfterRecycle, putBackText, keptBack }));

  // 19l. Templates: the toast says the fresh copy is in School Files, and adds how to open it when Tote could not.
  await page.locator('[data-testid="search-input"]').fill("");
  await visible(page, '[data-testid="templates"]');
  await page.locator('[data-testid="templates"] button', { hasText: "SF9 Template" }).click();
  const copyText = await page.locator(".toast:not(.toast-error) .toast-text", { hasText: "fresh copy" }).textContent({ timeout: STEP_TIMEOUT_MS }).catch(() => null);
  const unopened = await newPage(browser, VIEWPORT, "/?template=unopened");
  await unopened.page.keyboard.press("Escape");
  await hidden(unopened.page, '[data-testid="tour"]');
  await unopened.page.locator('[data-testid="templates"] button', { hasText: "SF9 Template" }).click();
  const unopenedText = await unopened.page.locator(".toast .toast-text", { hasText: "fresh copy" }).textContent({ timeout: STEP_TIMEOUT_MS }).catch(() => null);
  await capture(unopened.page, "19l-template-unopened");
  await unopened.context.close();
  record("Templates: the toast says the fresh copy is in School Files, and when Tote could not open it, says to open it from Find",
    copyText === "A fresh copy of SF9 Template is in School Files." && unopenedText === "A fresh copy of SF9 Template is in School Files. Tote could not open it; open it from Find.",
    JSON.stringify({ copyText, unopenedText }));

  const failureToasts = await toastAudit(page);
  record("No toast in the failure drive carried program text", failureToasts.seen > 0 && failureToasts.raw.length === 0, JSON.stringify(failureToasts));
  await context.close();
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
