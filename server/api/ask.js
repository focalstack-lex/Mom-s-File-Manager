// POST /api/ask: the Ask proxy for Tote. The logic lives in ../lib/core.js.

import { createHandler } from "../lib/core.js";

const handle = createHandler({ env: process.env, fetchImpl: (...args) => fetch(...args), now: () => new Date() });

export function POST(request) {
  return handle(request);
}

export function GET(request) {
  return handle(request);
}
