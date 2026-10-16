# tote-ask

The Ask proxy for Tote (spec: `docs/superpowers/specs/2026-10-14-ask-assistant-design.md`). One Vercel function, `POST /api/ask`, with no npm dependencies. It validates the conversation Tote sends, adds the assistant's instructions and tools, counts questions in Upstash (20 a day per laptop, Philippine days), and streams DeepSeek's answer back.

## Secrets (Vercel environment variables only)

- `DEEPSEEK_API_KEY`: set by Lex in the Vercel project (Settings > Environment Variables, or `vercel env add DEEPSEEK_API_KEY production` in this folder). Never commit it, print it, or paste it into chat.
- The Upstash pair, added by the Upstash integration (Storage > Marketplace > Upstash): `UPSTASH_REDIS_REST_URL` and `UPSTASH_REDIS_REST_TOKEN`, or `KV_REST_API_URL` and `KV_REST_API_TOKEN`.

Without any of them the function answers 503 and never calls DeepSeek. The order is the spec's: the method (405), the install id and the body (400) are checked first; the secrets and the counters only after that.

## Limits

Days are Philippine dates; the daily keys expire after 48 hours. Only a request that reaches DeepSeek is counted: the counters are read first and a refused request leaves them alone, and a request DeepSeek fails to answer (502) is given back.

- Questions: 20 a day per laptop (`q:<install id>:<day>`). A question is a request whose last message is hers. Over it: 429 `{ "error": "limit" }`.
- Calls: 100 a day per laptop, tool steps included (`c:<install id>:<day>`). Over it: 429 `{ "error": "busy" }`.
- Everyone: 3,000 calls a day for all laptops together (`all:<day>`). Over it: 429 `busy`.
- One address: 60 requests a minute (`ip:<address>:<minute>`, expires after 60 seconds), whatever install ids it uses, so one client cannot use up the day for everyone. Over it: 429 `busy`. The address comes from `x-forwarded-for`, which Vercel fills in; without the header there is no address limit.
- A blocked laptop: 429 `busy`.

## Commands

- `npm test`: the tests (fake DeepSeek and fake Upstash, nothing goes online).
- `vercel deploy --prod`: deploy from this folder (project `tote-ask`, region `sin1`).
- Block a laptop: in the Upstash console, `SET blocked:<install id> 1`. The install id is read in lower case, so write the key in lower case.
