/**
 * Tote invariant rules for ESLint.
 *
 * These rules turn prose directives (the Severus house style) into machine
 * enforced failures. Characters written as unicode escapes (for example
 * "\u2014") do not match the raw-text scans, so pattern definitions stay legal.
 *
 * Rules:
 *   tote/no-emoji              Zero emoji in source, copy and comments.
 *   tote/no-dash-punctuation   Zero em dash and en dash.
 *   tote/no-silent-catch       No empty catch block without a stated reason.
 *   tote/no-hardcoded-secret   No credentials committed to source.
 */

const DASH_PATTERN = /[\u2013\u2014]/g;

// Extended_Pictographic also covers a few typographic symbols that are legal in copy.
const ALLOWED_PICTOGRAPHS = new Set(["\u00A9", "\u00AE", "\u2122", "\u2139"]);
const EMOJI_PATTERN = /\p{Extended_Pictographic}/gu;

const SECRET_PATTERNS = [
  { label: "an OpenAI style sk- key", re: /\bsk-[A-Za-z0-9_-]{20,}/ },
  { label: "a GitHub personal access token", re: /\bgh[pousr]_[A-Za-z0-9]{20,}/ },
  { label: "a GitHub fine grained token", re: /\bgithub_pat_[A-Za-z0-9_]{20,}/ },
  { label: "a Google API key", re: /\bAIza[0-9A-Za-z_-]{30,}/ },
  { label: "a Slack token", re: /\bxox[baprs]-[A-Za-z0-9-]{10,}/ },
  { label: "a JSON Web Token", re: /\beyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}/ },
  { label: "an AWS access key id", re: /\b(?:AKIA|ASIA)[A-Z0-9]{16}\b/ },
  { label: "a private key block", re: /-----BEGIN [A-Z ]*PRIVATE KEY-----/ },
];

function locate(sourceCode, index) {
  const loc = sourceCode.getLocFromIndex(index);
  return { line: loc.line, column: loc.column };
}

function scanRawText(context, pattern, messageId, allow) {
  const sourceCode = context.sourceCode;
  const text = sourceCode.getText();
  pattern.lastIndex = 0;
  let match;
  while ((match = pattern.exec(text)) !== null) {
    if (!allow || !allow.has(match[0])) {
      context.report({ loc: locate(sourceCode, match.index), messageId, data: { sample: match[0] } });
    }
    // Zero length matches would loop forever.
    if (match.index === pattern.lastIndex) pattern.lastIndex += 1;
  }
}

const noEmoji = {
  meta: {
    type: "problem",
    docs: { description: "Disallow emoji characters in source, UI copy and comments" },
    schema: [],
    messages: {
      emoji: "Emoji are banned in this codebase (Strict Zero-Emoji Directive). Use a lucide-react icon or a text label instead. Found: {{sample}}",
    },
  },
  create(context) {
    return { Program() { scanRawText(context, EMOJI_PATTERN, "emoji", ALLOWED_PICTOGRAPHS); } };
  },
};

const noDashPunctuation = {
  meta: {
    type: "problem",
    docs: { description: "Disallow em dashes and en dashes in source, copy and comments" },
    schema: [],
    messages: {
      dash: "Em dashes and en dashes are banned (Strict Zero-Em-Dash Directive). Use a period, comma, colon, parentheses or a single hyphen. Found: {{sample}}",
    },
  },
  create(context) {
    return { Program() { scanRawText(context, DASH_PATTERN, "dash", null); } };
  },
};

const noSilentCatch = {
  meta: {
    type: "problem",
    docs: { description: "Disallow empty catch blocks that swallow an error silently" },
    schema: [],
    messages: {
      silent: "This catch block swallows the error silently. Log it, surface it, rethrow it, or state in a comment why the failure is safe to ignore (Defensive and Secure Programming Directive).",
    },
  },
  create(context) {
    return {
      CatchClause(node) {
        if (node.body.body.length > 0) return;
        // A documented best-effort catch is a decision, not a swallow.
        if (context.sourceCode.getCommentsInside(node.body).length > 0) return;
        context.report({ node, messageId: "silent" });
      },
    };
  },
};

const noHardcodedSecret = {
  meta: {
    type: "problem",
    docs: { description: "Disallow hardcoded credentials and API keys in source" },
    schema: [],
    messages: {
      secret: "This looks like {{label}} committed to source. Move it to .env (see .env.example) and keep it out of version control.",
    },
  },
  create(context) {
    function check(value, node) {
      if (typeof value !== "string") return;
      for (const { label, re } of SECRET_PATTERNS) {
        if (re.test(value)) {
          context.report({ node, messageId: "secret", data: { label } });
          return;
        }
      }
    }
    return {
      Literal(node) { check(node.value, node); },
      TemplateElement(node) { check(node.value.raw, node); },
    };
  },
};

export default {
  meta: { name: "tote-invariants", version: "1.0.0" },
  rules: {
    "no-emoji": noEmoji,
    "no-dash-punctuation": noDashPunctuation,
    "no-silent-catch": noSilentCatch,
    "no-hardcoded-secret": noHardcodedSecret,
  },
};
