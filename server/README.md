# tote-ask

The Ask proxy for Tote (spec: `docs/superpowers/specs/2026-10-14-ask-assistant-design.md`). One Vercel function, `POST /api/ask`, with no npm dependencies. It validates the conversation Tote sends, adds the assistant's instructions and tools, counts questions in Upstash (20 a day per laptop, Philippine days), and streams DeepSeek's answer back.

## Secrets (Vercel environment variables only)

- `DEEPSEEK_API_KEY`: set by Lex in the Vercel project (Settings > Environment Variables, or `vercel env add DEEPSEEK_API_KEY production` in this folder). Never commit it, print it, or paste it into chat.
- The Upstash pair, added by the Upstash integration (Storage > Marketplace > Upstash): `UPSTASH_REDIS_REST_URL` and `UPSTASH_REDIS_REST_TOKEN`, or `KV_REST_API_URL` and `KV_REST_API_TOKEN`.

Without any of them the function answers 503 and never calls DeepSeek.

## Commands

- `npm test`: the tests (fake DeepSeek and fake Upstash, nothing goes online).
- `vercel deploy --prod`: deploy from this folder (project `tote-ask`, region `sin1`).
- Block a laptop: in the Upstash console, `SET blocked:<install id> 1`.
