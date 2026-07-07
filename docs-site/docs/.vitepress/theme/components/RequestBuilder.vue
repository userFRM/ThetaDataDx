<script setup>
import { ref, computed } from 'vue'
import { icons, brand } from './icons'
import hljs from 'highlight.js/lib/core'
import rust from 'highlight.js/lib/languages/rust'
import python from 'highlight.js/lib/languages/python'
import typescript from 'highlight.js/lib/languages/typescript'
import cpp from 'highlight.js/lib/languages/cpp'
import bash from 'highlight.js/lib/languages/bash'
import json from 'highlight.js/lib/languages/json'

hljs.registerLanguage('rust', rust)
hljs.registerLanguage('python', python)
hljs.registerLanguage('typescript', typescript)
hljs.registerLanguage('cpp', cpp)
hljs.registerLanguage('bash', bash)
hljs.registerLanguage('json', json)

const props = defineProps({
  cfg: {
    type: Object,
    default: () => ({
      httpPath: 'v3/option/history/eod',
      method: { rust: 'option_history_eod', python: 'option_history_eod', ts: 'optionHistoryEOD', cpp: 'option_history_eod' },
      required: [
        { key: 'symbol', type: 'string', default: 'SPY' },
        { key: 'expiration', type: 'date', default: '20250321' },
        { key: 'start_date', type: 'date', default: '20250303' },
        { key: 'end_date', type: 'date', default: '20250306' },
      ],
      optional: [
        { key: 'strike', type: 'string', default: '570' },
        { key: 'right', type: 'enum', default: 'C', opts: ['C', 'P', 'both'] },
      ],
      print: ['date', 'open', 'close', 'volume'],
      sample: [{ date: 20250303, open: 5.3, high: 5.85, low: 5.25, close: 5.6, volume: 1240, count: 88 }],
    }),
  },
})

const allFields = computed(() => [
  ...props.cfg.required.map((f) => ({ ...f, req: true })),
  ...props.cfg.optional.map((f) => ({ ...f, req: false })),
])

const vals = ref(Object.fromEntries(allFields.value.map((f) => [f.key, f.default ?? ''])))

const auth = ref({ kind: 'apikey', source: 'env' })
const callStyle = ref('sync')
const clientKind = ref('unified')

const langs = [
  { id: 'python', label: 'Python', hl: 'python' },
  { id: 'rust', label: 'Rust', hl: 'rust' },
  { id: 'typescript', label: 'TypeScript', hl: 'typescript' },
  { id: 'cpp', label: 'C++', hl: 'cpp' },
  { id: 'curl', label: 'curl', hl: 'bash' },
]
const active = ref('python')
const isCurl = computed(() => active.value === 'curl')

const cls = computed(() => (clientKind.value === 'unified' ? 'Client' : 'MarketDataClient'))
const filledOpt = computed(() => props.cfg.optional.filter((o) => (vals.value[o.key] ?? '') !== ''))
const scalar = computed(() => !!props.cfg.scalar)
function camel(s) {
  return s.replace(/_([a-z0-9])/g, (_, ch) => ch.toUpperCase())
}

// int/float/bool values render as bare literals; every other type (string,
// date, symbols, enum, …) is quoted. `type` is the registry `docs_param_type`
// (int/float/bool/date/symbols/string), carried in the cfg, so the emitted code
// matches each parameter's real Rust/TS/C++/Python type instead of stringifying
// everything. An unquoted allowlist (not a quoted one) keeps an unexpected type
// safely quoted rather than emitted bare.
const UNQUOTED_TYPES = new Set(['int', 'float', 'bool'])
const BOOL_LIT = {
  python: (v) => (truthy(v) ? 'True' : 'False'),
  rust: (v) => (truthy(v) ? 'true' : 'false'),
  typescript: (v) => (truthy(v) ? 'true' : 'false'),
  cpp: (v) => (truthy(v) ? 'true' : 'false'),
}
function truthy(v) {
  return /^(true|1|yes)$/i.test(String(v).trim())
}
// Escape a raw value for a double-quoted string literal: backslash and the quote
// first, then control chars, so a value carrying `"`, `\`, or a newline cannot
// terminate the literal or break the snippet. The escapes (`\\`, `\"`, `\n`,
// `\r`, `\t`) are shared by Rust, TypeScript, Python, and C++ string literals.
function escDquote(s) {
  return String(s)
    .replace(/\\/g, '\\\\')
    .replace(/"/g, '\\"')
    .replace(/\n/g, '\\n')
    .replace(/\r/g, '\\r')
    .replace(/\t/g, '\\t')
}
// Escape a value for a shell single-quoted string: a literal `'` becomes
// `'\''` (close quote, escaped quote, reopen). Newlines stay literal — curl
// accepts them inside the quoted argument.
function escSquote(s) {
  return String(s).replace(/'/g, "'\\''")
}
// Render one parameter value for a code target: bare literal for numeric/bool
// types, an escaped double-quoted string otherwise.
function fmtVal(lang, type, raw) {
  if (type === 'bool') return BOOL_LIT[lang](raw)
  if (UNQUOTED_TYPES.has(type)) return String(raw).trim() // int / float: bare literal
  return `"${escDquote(raw)}"`
}
function reqArgs(lang) {
  return props.cfg.required.map((r) => fmtVal(lang, r.type, vals.value[r.key])).join(', ')
}
// Per-language print bodies. Scalar (list) endpoints print the bare row;
// TypeScript reads camelCase fields, the others snake_case.
const pyPrint = computed(() => (scalar.value ? 't' : props.cfg.print.map((f) => `t.${f}`).join(', ')))
const tsPrint = computed(() =>
  scalar.value ? 't' : props.cfg.print.map((f) => `t.${camel(f)}`).join(', ')
)
const cppPrint = computed(() =>
  scalar.value ? 't' : props.cfg.print.map((f) => `t.${f}`).join(" << ' ' << ")
)
const rustPrintln = computed(() => {
  if (scalar.value) return 'println!("{t}")'
  const fmt = props.cfg.print.map((f) => `${f}={}`).join(' ')
  const vals = props.cfg.print.map((f) => `t.${f}`).join(', ')
  return `println!("${fmt}", ${vals})`
})

function hist(lang) {
  if (clientKind.value !== 'unified') return ''
  const acc = lang === 'typescript' ? 'marketData' : 'market_data'
  return lang === 'rust' || lang === 'cpp' ? `.${acc}()` : `.${acc}`
}

// Client construction, mapped to the real SDK surface. The unified `Client`
// has the ergonomic one-step constructors (builder / connectWith / inline
// kwargs); the market-data-only `MarketDataClient` exposes no such sugar, so it
// is built from a `Credentials` value passed to `connect(creds, config)` (or
// the `from_file` / connectFromFile convenience). The `auth.source === 'env'`
// + `creds` cell sources email + password from a `.env`/creds file — the SDK
// has no inline-from-process-env email+password constructor; the env path is
// API-key only. Returns the construction statement plus any extra symbols to
// import alongside the client class.
function clientLine(lang) {
  const a = auth.value
  // The selected auth cell as a stable key: env-apikey, inline-apikey,
  // env-creds (file), inline-creds.
  const cell = a.source === 'env' ? `env-${a.kind}` : `inline-${a.kind}`

  // Unified Client: rich one-step constructors.
  const unified = {
    python: {
      'env-apikey': { line: `client = Client.from_env()`, imports: [] },
      'inline-apikey': { line: `client = Client(api_key="YOUR_API_KEY")`, imports: [] },
      'inline-creds': {
        line: `client = Client(email="you@example.com", password="YOUR_PASSWORD")`,
        imports: [],
      },
      'env-creds': { line: `client = Client.from_file("creds.txt")`, imports: [] },
    },
    rust: {
      'env-apikey': {
        line: `let client = Client::builder().api_key_from_env().connect().await?;`,
        imports: [],
      },
      'inline-apikey': {
        line: `let client = Client::builder().api_key("YOUR_API_KEY").connect().await?;`,
        imports: [],
      },
      'inline-creds': {
        line: `let client = Client::builder()\n        .email_password("you@example.com", "YOUR_PASSWORD")\n        .connect()\n        .await?;`,
        imports: [],
      },
      'env-creds': {
        line: `let client = Client::connect(&Credentials::from_file("creds.txt")?, DirectConfig::production()).await?;`,
        imports: ['Credentials', 'DirectConfig'],
      },
    },
    typescript: {
      'env-apikey': {
        line: `const client = await Client.connectWith({ apiKeyFromEnv: true });`,
        imports: [],
      },
      'inline-apikey': {
        line: `const client = await Client.connectWith({ apiKey: "YOUR_API_KEY" });`,
        imports: [],
      },
      'inline-creds': {
        line: `const client = await Client.connectWith({ email: "you@example.com", password: "YOUR_PASSWORD" });`,
        imports: [],
      },
      'env-creds': {
        line: `const client = await Client.connectFromFile("creds.txt");`,
        imports: [],
      },
    },
    cpp: {
      'env-apikey': {
        line: `auto client = thetadatadx::Client::builder().api_key_from_env().connect();`,
        imports: [],
      },
      'inline-apikey': {
        line: `auto client = thetadatadx::Client::builder().api_key("YOUR_API_KEY").connect();`,
        imports: [],
      },
      'inline-creds': {
        line: `auto client = thetadatadx::Client::builder()\n      .email_password("you@example.com", "YOUR_PASSWORD")\n      .connect();`,
        imports: [],
      },
      'env-creds': {
        line: `auto client = thetadatadx::Client::from_file("creds.txt");`,
        imports: [],
      },
    },
  }

  // MarketDataClient: build a Credentials, then connect(creds, config).
  const marketData = {
    python: {
      'env-apikey': {
        line: `client = MarketDataClient(Credentials.from_env(), Config.production())`,
        imports: ['Credentials', 'Config'],
      },
      'inline-apikey': {
        line: `client = MarketDataClient(Credentials.from_api_key("YOUR_API_KEY"), Config.production())`,
        imports: ['Credentials', 'Config'],
      },
      'inline-creds': {
        line: `client = MarketDataClient(Credentials("you@example.com", "YOUR_PASSWORD"), Config.production())`,
        imports: ['Credentials', 'Config'],
      },
      'env-creds': { line: `client = MarketDataClient.from_file("creds.txt")`, imports: [] },
    },
    rust: {
      'env-apikey': {
        line: `let client = MarketDataClient::connect(&Credentials::from_env()?, DirectConfig::production()).await?;`,
        imports: ['Credentials', 'DirectConfig'],
      },
      'inline-apikey': {
        line: `let client = MarketDataClient::connect(&Credentials::api_key("YOUR_API_KEY"), DirectConfig::production()).await?;`,
        imports: ['Credentials', 'DirectConfig'],
      },
      'inline-creds': {
        line: `let client = MarketDataClient::connect(&Credentials::new("you@example.com", "YOUR_PASSWORD"), DirectConfig::production()).await?;`,
        imports: ['Credentials', 'DirectConfig'],
      },
      'env-creds': {
        line: `let client = MarketDataClient::connect(&Credentials::from_file("creds.txt")?, DirectConfig::production()).await?;`,
        imports: ['Credentials', 'DirectConfig'],
      },
    },
    typescript: {
      'env-apikey': {
        line: `const client = await MarketDataClient.connect(Credentials.fromEnv());`,
        imports: ['Credentials'],
      },
      'inline-apikey': {
        line: `const client = await MarketDataClient.connect(Credentials.fromApiKey("YOUR_API_KEY"));`,
        imports: ['Credentials'],
      },
      'inline-creds': {
        line: `const client = await MarketDataClient.connect(new Credentials("you@example.com", "YOUR_PASSWORD"));`,
        imports: ['Credentials'],
      },
      'env-creds': {
        line: `const client = await MarketDataClient.connectFromFile("creds.txt");`,
        imports: [],
      },
    },
    cpp: {
      'env-apikey': {
        line: `auto client = thetadatadx::MarketDataClient::connect(thetadatadx::Credentials::from_env(), thetadatadx::Config::production());`,
        imports: [],
      },
      'inline-apikey': {
        line: `auto client = thetadatadx::MarketDataClient::connect(thetadatadx::Credentials::from_api_key("YOUR_API_KEY"), thetadatadx::Config::production());`,
        imports: [],
      },
      'inline-creds': {
        line: `auto client = thetadatadx::MarketDataClient::connect(thetadatadx::Credentials::from_email("you@example.com", "YOUR_PASSWORD"), thetadatadx::Config::production());`,
        imports: [],
      },
      'env-creds': {
        line: `auto client = thetadatadx::MarketDataClient::from_file("creds.txt");`,
        imports: [],
      },
    },
  }

  const table = clientKind.value === 'unified' ? unified : marketData
  return table[lang][cell]
}

// Symbols to import for the current language + auth + client selection: the
// client class always, plus any credential/config types the construction needs.
function importSymbols(lang) {
  return [cls.value, ...clientLine(lang).imports]
}

const code = computed(() => {
  const c = props.cfg
  switch (active.value) {
    case 'python': {
      const imp = importSymbols('python').join(', ')
      const req = reqArgs('python')
      const opt = filledOpt.value
        .map((o) => `${o.key}=${fmtVal('python', o.type, vals.value[o.key])}`)
        .join(', ')
      const empty = !req && !opt
      if (callStyle.value === 'async') {
        const optLine = opt ? `\n        ${opt},` : ''
        // Zero-arg call collapses to bare `method_async()`; otherwise the
        // multi-line arg block (a trailing comma after an empty block is a
        // stray-comma syntax error).
        const call = empty
          ? `${c.method.python}_async()`
          : `${c.method.python}_async(\n        ${req},${optLine}\n    )`
        return `import asyncio
from thetadatadx import ${imp}

async def main():
    ${clientLine('python').line}

    rows = await client${hist('python')}.${call}
    for t in rows:
        print(${pyPrint.value})

asyncio.run(main())`
      }
      const optLine = opt ? `\n    ${opt},` : ''
      const call = empty
        ? `${c.method.python}()`
        : `${c.method.python}(\n    ${req},${optLine}\n)`
      return `from thetadatadx import ${imp}

${clientLine('python').line}

rows = client${hist('python')}.${call}
for t in rows:
    print(${pyPrint.value})`
    }
    case 'rust': {
      const syms = importSymbols('rust')
      const use = syms.length > 1 ? `use thetadatadx::{${syms.join(', ')}};` : `use thetadatadx::${syms[0]};`
      const opt = filledOpt.value
        .map((o) => `\n        .${o.key}(${fmtVal('rust', o.type, vals.value[o.key])})`)
        .join('')
      const histLine = hist('rust') ? `\n        ${hist('rust')}` : ''
      return `${use}

#[tokio::main]
async fn main() -> Result<(), thetadatadx::Error> {
    ${clientLine('rust').line}

    let rows = client${histLine}
        .${c.method.rust}(${reqArgs('rust')})${opt}
        .await?;

    for t in &rows {
        ${rustPrintln.value};
    }
    Ok(())
}`
    }
    case 'typescript': {
      const imp = importSymbols('typescript').join(', ')
      const req = reqArgs('typescript')
      const opt = filledOpt.value
        .map((o) => `${camel(o.key)}: ${fmtVal('typescript', o.type, vals.value[o.key])}`)
        .join(', ')
      const optLine = opt ? `\n  { ${opt} },` : ''
      // Zero-arg call collapses to bare `method()`; otherwise the multi-line
      // arg block (a leading comma on an empty arg list is a syntax error).
      const call = !req && !opt ? `${c.method.ts}()` : `${c.method.ts}(\n  ${req},${optLine}\n)`
      return `import { ${imp} } from "thetadatadx-ts";

${clientLine('typescript').line}

const rows = await client${hist('typescript')}.${call};
for (const t of rows) {
  console.log(${tsPrint.value});
}`
    }
    case 'cpp': {
      const opt = filledOpt.value
        .map((o) => `.with_${o.key}(${fmtVal('cpp', o.type, vals.value[o.key])})`)
        .join('')
      const optArg = opt ? `,\n      thetadatadx::EndpointRequestOptions{}${opt}` : ''
      return `#include <thetadatadx/thetadatadx.hpp>
#include <iostream>

int main() {
  ${clientLine('cpp').line}
  auto rows = client${hist('cpp')}.${c.method.cpp}(${reqArgs('cpp')}${optArg});
  for (const auto& t : rows) {
    std::cout << ${cppPrint.value} << "\\n";
  }
}`
    }
    case 'curl': {
      // Values ride the wire as strings, so every param is quoted; escape any
      // `'` in the value for the surrounding single-quotes (close, escaped
      // quote, reopen) so it cannot break out of the argument.
      const all = [...c.required, ...filledOpt.value]
      const base = `curl -G 'http://127.0.0.1:25503/${c.httpPath}'`
      if (all.length === 0) return base
      const lines = all
        .map((f) => `  --data-urlencode '${f.key}=${escSquote(vals.value[f.key])}'`)
        .join(' \\\n')
      return `${base} \\\n${lines}`
    }
    default:
      return ''
  }
})

const highlighted = computed(() => {
  const hl = langs.find((l) => l.id === active.value)?.hl || 'plaintext'
  return hljs.highlight(code.value, { language: hl }).value
})

const sampleHl = computed(() => {
  if (!props.cfg.sample) return ''
  return hljs.highlight(JSON.stringify(props.cfg.sample, null, 2), { language: 'json' }).value
})

const expanded = ref(false)
const summary = computed(() => props.cfg.required.map((r) => vals.value[r.key]).filter(Boolean).join('  ·  '))

const copied = ref(false)
function copy() {
  navigator.clipboard?.writeText(code.value)
  copied.value = true
  setTimeout(() => (copied.value = false), 1200)
}
</script>

<template>
  <div class="rb">
    <div class="rb-trayBar" @click="expanded = !expanded">
      <button class="rb-trayBtn" @click.stop="expanded = !expanded">
        <span>{{ expanded ? 'Hide options' : 'Customize request' }}</span>
        <svg viewBox="0 0 24 24" class="rb-chev" :class="{ open: expanded }" fill="none" stroke="currentColor" stroke-width="2.2"><path d="M6 9l6 6 6-6" /></svg>
      </button>
      <span class="rb-traySum">{{ summary }}</span>
    </div>

    <div class="rb-tray" :class="{ open: expanded }">
    <div class="rb-bar" v-if="!isCurl">
      <div class="rb-og">
        <span class="rb-ol">Client</span>
        <div class="rb-pg">
          <button :class="{ on: clientKind === 'unified' }" @click="clientKind = 'unified'">Unified</button>
          <button :class="{ on: clientKind === 'market_data' }" @click="clientKind = 'market_data'">Market-data</button>
        </div>
      </div>
      <div class="rb-og">
        <span class="rb-ol">Auth</span>
        <div class="rb-pg">
          <button :class="{ on: auth.kind === 'apikey' }" @click="auth.kind = 'apikey'">API key</button>
          <button :class="{ on: auth.kind === 'creds' }" @click="auth.kind = 'creds'">Email + pw</button>
        </div>
        <div class="rb-pg">
          <button :class="{ on: auth.source === 'env' }" @click="auth.source = 'env'">Env</button>
          <button :class="{ on: auth.source === 'inline' }" @click="auth.source = 'inline'">Inline</button>
        </div>
      </div>
      <div class="rb-og" v-if="active === 'python'">
        <span class="rb-ol">Style</span>
        <div class="rb-pg">
          <button :class="{ on: callStyle === 'sync' }" @click="callStyle = 'sync'">Sync</button>
          <button :class="{ on: callStyle === 'async' }" @click="callStyle = 'async'">Async</button>
        </div>
      </div>
    </div>
    <div v-else class="rb-bar rb-curlnote">curl hits the local server; it holds the credentials, the request carries none.</div>

    <div class="rb-grid">
      <div class="rb-cell" v-for="f in allFields" :key="f.key">
        <label>
          <span class="n">{{ f.key }}</span>
          <span class="t">{{ f.type }}</span>
          <span v-if="f.req" class="r">*</span>
        </label>
        <select v-if="f.opts" v-model="vals[f.key]">
          <option v-for="o in f.opts" :key="o" :value="o">{{ o }}</option>
        </select>
        <input v-else v-model="vals[f.key]" spellcheck="false" :placeholder="f.ph || (f.req ? '' : 'optional')" />
      </div>
    </div>
    </div>

    <div class="rb-split" :class="{ solo: !cfg.sample }">
    <div class="rb-code">
      <div class="rb-tabs">
        <button
          v-for="l in langs"
          :key="l.id"
          class="rb-tab"
          :class="{ on: active === l.id }"
          :style="active === l.id ? { color: brand[l.id], borderColor: brand[l.id] } : {}"
          @click="active = l.id"
        >
          <svg viewBox="0 0 24 24" class="rb-ico"><path :d="icons[l.id]" /></svg>
          <span>{{ l.label }}</span>
        </button>
        <button class="rb-copy" @click="copy">{{ copied ? 'Copied ✓' : 'Copy' }}</button>
      </div>
      <pre class="rb-pre"><code class="hljs" v-html="highlighted"></code></pre>
    </div>

    <div v-if="cfg.sample" class="rb-resp">
      <div class="rb-resp-h"><span class="rb-dot" /> Sample response · <code>JSON</code></div>
      <pre class="rb-pre"><code class="hljs" v-html="sampleHl"></code></pre>
    </div>
    </div>
  </div>
</template>

<style>
@import 'highlight.js/styles/github-dark.css';
</style>

<style scoped>
.rb {
  border: 1px solid var(--vp-c-divider);
  border-radius: 8px;
  overflow: hidden;
  margin: 18px 0;
}

.rb-trayBar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 7px 13px;
  background: var(--vp-c-bg-soft);
  border-bottom: 1px solid var(--vp-c-divider);
  cursor: pointer;
  user-select: none;
  transition: background 0.16s ease, box-shadow 0.16s ease;
}
.rb-trayBar:hover {
  background: var(--vp-c-bg-mute);
  box-shadow: inset 0 -2px 0 var(--vp-c-brand-1);
}
.rb-trayBar:hover .rb-trayBtn { color: var(--vp-c-brand-1); }
.rb-trayBar:hover .rb-chev:not(.open) { animation: rb-bounce 0.7s ease infinite; }
@keyframes rb-bounce {
  0%, 100% { transform: translateY(0); }
  50% { transform: translateY(3px); }
}
.rb-trayBtn {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  background: transparent;
  border: none;
  color: var(--vp-c-text-1);
  font: 600 12px var(--vp-font-family-base);
  cursor: pointer;
  padding: 0;
  flex: none;
}
.rb-trayBtn:hover { color: var(--vp-c-brand-1); }
.rb-chev { width: 13px; height: 13px; transition: transform 0.22s ease; }
.rb-chev.open { transform: rotate(180deg); }
.rb-traySum {
  font: 11.5px var(--vp-font-family-base);
  color: var(--vp-c-text-3);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.rb-tray {
  max-height: 0;
  overflow: hidden;
  opacity: 0;
  transition: max-height 0.26s ease, opacity 0.2s ease;
}
.rb-tray.open { max-height: 640px; opacity: 1; }
/* peek: cracking the tray open on hover signals it expands */
.rb-trayBar:hover ~ .rb-tray:not(.open),
.rb-tray:not(.open):hover { max-height: 44px; opacity: 1; }

.rb-bar {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 8px 18px;
  padding: 9px 13px;
  background: var(--vp-c-bg-soft);
  border-bottom: 1px solid var(--vp-c-divider);
}
.rb-og { display: flex; align-items: center; gap: 6px; }
.rb-ol {
  font: 600 9.5px/1 var(--vp-font-family-base);
  text-transform: uppercase;
  letter-spacing: 0.06em;
  color: var(--vp-c-text-3);
}
.rb-pg {
  display: inline-flex;
  background: var(--vp-c-bg);
  border: 1px solid var(--vp-c-divider);
  border-radius: 999px;
  padding: 2px;
}
.rb-pg button {
  border: none;
  background: transparent;
  color: var(--vp-c-text-2);
  font: 500 11px var(--vp-font-family-base);
  padding: 2px 12px;
  border-radius: 999px;
  cursor: pointer;
  transition: all 0.14s ease;
  white-space: nowrap;
}
.rb-pg button:hover:not(.on) { color: var(--vp-c-text-1); }
.rb-pg button.on {
  background: var(--vp-c-brand-1);
  color: #fff;
  font-weight: 600;
}
.rb-curlnote { font-size: 12px; color: var(--vp-c-text-2); }

.rb-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(150px, 1fr));
  gap: 8px 10px;
  padding: 12px 13px;
  background: var(--vp-c-bg-soft);
  border-bottom: 1px solid var(--vp-c-divider);
}
.rb-cell { display: flex; flex-direction: column; min-width: 0; }
.rb-cell label { display: flex; align-items: baseline; gap: 5px; margin-bottom: 3px; }
.rb-cell .n { font: 600 11.5px var(--vp-font-family-base); color: var(--vp-c-text-1); }
.rb-cell .t { font-size: 10px; color: var(--vp-c-text-3); }
.rb-cell .r { color: var(--vp-c-danger-1); font-size: 10px; margin-left: auto; }
.rb-cell input,
.rb-cell select {
  background: var(--vp-c-bg);
  border: 1px solid var(--vp-c-divider);
  border-radius: 5px;
  padding: 4px 7px;
  font: 11.5px var(--vp-font-family-base);
  color: var(--vp-c-text-1);
  outline: none;
  width: 100%;
  transition: border-color 0.12s;
}
.rb-cell input::placeholder { color: var(--vp-c-text-3); opacity: 0.7; }
.rb-cell input:focus,
.rb-cell select:focus { border-color: var(--vp-c-brand-1); }

.rb-split { display: grid; grid-template-columns: minmax(0, 1.3fr) minmax(0, 1fr); }
.rb-split.solo { grid-template-columns: 1fr; }
.rb-split.solo .rb-code { border-right: none; }
@media (max-width: 820px) { .rb-split { grid-template-columns: 1fr; } }
.rb-code { display: flex; flex-direction: column; min-width: 0; background: #0d1117; border-right: 1px solid #21262d; }
.rb-tabs {
  display: flex;
  align-items: stretch;
  gap: 2px;
  padding: 0 8px;
  background: #161b22;
  border-bottom: 1px solid #21262d;
  flex-wrap: nowrap;
  overflow-x: auto;
  height: 38px;
  box-sizing: border-box;
}
.rb-tab {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  background: transparent;
  border: none;
  border-bottom: 2px solid transparent;
  color: #8b949e;
  font-size: 11.5px;
  padding: 6px 10px;
  cursor: pointer;
  transition: color 0.12s;
}
.rb-tab:hover { color: #c9d1d9; }
.rb-tab.on { font-weight: 600; }
.rb-ico { width: 15px; height: 15px; fill: currentColor; flex: none; }
.rb-copy {
  margin-left: auto;
  align-self: center;
  background: #21262d;
  border: 1px solid #30363d;
  color: #c9d1d9;
  font-size: 11.5px;
  padding: 4px 10px;
  border-radius: 6px;
  cursor: pointer;
}
.rb-copy:hover { background: #30363d; }
.rb-pre { margin: 0; padding: 12px 14px; overflow-x: auto; background: #0d1117; flex: 1 1 auto; }
.rb-pre code { font: 11.5px/1.55 var(--vp-font-family-mono); background: transparent; padding: 0; white-space: pre; }

.rb-resp { background: #0d1117; display: flex; flex-direction: column; }
@media (max-width: 820px) { .rb-code { border-right: none; border-bottom: 1px solid #21262d; } }
.rb-resp-h {
  display: flex;
  align-items: center;
  gap: 7px;
  padding: 0 13px;
  height: 38px;
  box-sizing: border-box;
  font: 600 10px/1 var(--vp-font-family-base);
  text-transform: uppercase;
  letter-spacing: 0.06em;
  color: #8b949e;
  background: #161b22;
  border-bottom: 1px solid #21262d;
}
.rb-resp-h code {
  font-family: var(--vp-font-family-mono);
  text-transform: none;
  letter-spacing: 0;
  color: #c9d1d9;
  background: transparent;
  font-size: 11px;
}
.rb-dot { width: 7px; height: 7px; border-radius: 50%; background: #16a34a; flex: none; }
</style>
