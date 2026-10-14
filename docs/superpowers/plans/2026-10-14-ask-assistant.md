# Ask assistant Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ask, the assistant inside Tote: a chat screen, read-only checks she approves, local file search, To do cards, behind our own Vercel proxy that holds the DeepSeek key and enforces 20 questions a day per laptop.

**Architecture:** `server/` is a dependency-free Node function on Vercel (`sin1`) that validates requests, owns the system prompt and tools, counts in Upstash and streams DeepSeek through. Tote's native side (`ask.rs`, `checks.rs`) holds the conversation loop, runs tools locally, and emits `tote://ask` events; the renderer draws the chat.

**Spec:** `docs/superpowers/specs/2026-10-14-ask-assistant-design.md`

## Global Constraints

- No emoji, no em or en dash anywhere. Secrets only in Vercel environment variables; never printed, logged, committed or bundled.
- Fail closed: missing secrets, invalid requests and unknown tools are refused.
- Rust: no unwrap, expect or panic outside tests; disk and network work off the main thread; settings read before the database lock.
- Check scripts are constants; nothing from the model or from her reaches a shell.

---

### Task 1: Server function and its tests
Files: `server/api/ask.js`, `server/lib/core.js` (validation, prompt, tools, limits, day), `server/test/ask.test.js`, `server/package.json`, `server/vercel.json`, `server/README.md`.
Tests (`node --test`): rejects non-POST, bad install id, system role, extra fields, too many or too long messages, unknown tool names, last message from the assistant; prompt and tools always the server's; question counted only when the last message is `user`; 21st question gives 429 limit; call cap and global cap give 429 busy; blocked laptop gives busy; Philippine midnight rolls the day; missing secrets give 503; upstream failure gives 502; a success streams the fake upstream body through with `X-Tote-Questions-Left`.

### Task 2: Deploy the server (with Lex)
`vercel link` a new project `tote-ask` from `server/`, Lex adds `DEEPSEEK_API_KEY` himself, Upstash connected after Lex agrees to its terms, `vercel deploy --prod`, deploy gate, live probe: invalid request 400, a real question streams.

### Task 3: Native checks (`checks.rs`)
Four PowerShell scripts as constants, run hidden with a 15 s limit, JSON out; parsers to findings in plain words. Tests on recorded JSON.

### Task 4: Native conversation (`ask.rs`), history, commands
Chat table, install id, request with reqwest (rustls, stream), SSE parsing with tool-call deltas, tool validation, consent pause and resume, search_files and add_task handling, student-data check, offline answers, events. Tests: SSE assembly, broken tool JSON, unknown tools, student-data check, offline answers, history purge.

### Task 5: Renderer
`AskView`, `ChatMessage`, `AskCards`, `store/ask`, `lib/askText` (safe renderer), sidebar item and New marker, seventh walkthrough step, mock backend with a scripted fake stream. vitest for askText and quota words.

### Task 6: UI drive and native check
UI drive steps for every state; native check opt-in live question (`TOTE_CHECK_ASK=1`) and a key-pattern scan of the installer.

### Task 7: Docs, release 0.4.0
FEATURE_MAP, CODE_MAP, main spec amendments, README, journal; deploy gate; `npm run release`; portfolio deploy; live checks.
