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

const cls = computed(() => (clientKind.value === 'unified' ? 'Client' : 'HistoricalClient'))
const reqArgs = computed(() => props.cfg.required.map((r) => `"${vals.value[r.key]}"`).join(', '))
const filledOpt = computed(() => props.cfg.optional.filter((o) => (vals.value[o.key] ?? '') !== ''))
const scalar = computed(() => !!props.cfg.scalar)
function camel(s) {
  return s.replace(/_([a-z0-9])/g, (_, ch) => ch.toUpperCase())
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
  return lang === 'rust' || lang === 'cpp' ? '.historical()' : '.historical'
}

function clientLine(lang) {
  const a = auth.value
  const C = cls.value
  const envNote = a.kind === 'apikey' ? 'THETADATA_API_KEY' : 'THETADATA_EMAIL + THETADATA_PASSWORD'
  const m = {
    python: {
      env: `client = ${C}.from_env()  # reads ${envNote}`,
      apikey: `client = ${C}(api_key="YOUR_API_KEY")`,
      creds: `client = ${C}(email="you@example.com", password="YOUR_PASSWORD")`,
    },
    rust: {
      env: `let client = ${C}::from_env()?; // reads ${envNote}`,
      apikey: `let client = ${C}::with_api_key("YOUR_API_KEY")?;`,
      creds: `let client = ${C}::with_credentials("you@example.com", "YOUR_PASSWORD")?;`,
    },
    typescript: {
      env: `const client = ${C}.fromEnv(); // reads ${envNote}`,
      apikey: `const client = new ${C}({ apiKey: "YOUR_API_KEY" });`,
      creds: `const client = new ${C}({ email: "you@example.com", password: "YOUR_PASSWORD" });`,
    },
    cpp: {
      env: `auto client = thetadatadx::${C}::from_env(); // reads ${envNote}`,
      apikey: `auto client = thetadatadx::${C}::with_api_key("YOUR_API_KEY");`,
      creds: `auto client = thetadatadx::${C}::with_credentials("you@example.com", "YOUR_PASSWORD");`,
    },
  }
  if (a.source === 'env') return m[lang].env
  return a.kind === 'apikey' ? m[lang].apikey : m[lang].creds
}

const code = computed(() => {
  const c = props.cfg
  switch (active.value) {
    case 'python': {
      const opt = filledOpt.value.map((o) => `${o.key}="${vals.value[o.key]}"`).join(', ')
      if (callStyle.value === 'async') {
        const optLine = opt ? `\n        ${opt},` : ''
        return `import asyncio
from thetadatadx import ${cls.value}

async def main():
    ${clientLine('python')}

    rows = await client${hist('python')}.${c.method.python}_async(
        ${reqArgs.value},${optLine}
    )
    for t in rows:
        print(${pyPrint.value})

asyncio.run(main())`
      }
      const optLine = opt ? `\n    ${opt},` : ''
      return `from thetadatadx import ${cls.value}

${clientLine('python')}

rows = client${hist('python')}.${c.method.python}(
    ${reqArgs.value},${optLine}
)
for t in rows:
    print(${pyPrint.value})`
    }
    case 'rust': {
      const opt = filledOpt.value.map((o) => `\n        .${o.key}("${vals.value[o.key]}")`).join('')
      const histLine = hist('rust') ? `\n        ${hist('rust')}` : ''
      return `use thetadatadx::${cls.value};

#[tokio::main]
async fn main() -> Result<(), thetadatadx::Error> {
    ${clientLine('rust')}

    let rows = client${histLine}
        .${c.method.rust}(${reqArgs.value})${opt}
        .await?;

    for t in &rows {
        ${rustPrintln.value};
    }
    Ok(())
}`
    }
    case 'typescript': {
      const opt = filledOpt.value.map((o) => `${camel(o.key)}: "${vals.value[o.key]}"`).join(', ')
      const optLine = opt ? `\n  { ${opt} },` : ''
      return `import { ${cls.value} } from "thetadatadx";

${clientLine('typescript')}

const rows = await client${hist('typescript')}.${c.method.ts}(
  ${reqArgs.value},${optLine}
);
for (const t of rows) {
  console.log(${tsPrint.value});
}`
    }
    case 'cpp': {
      const opt = filledOpt.value.map((o) => `.with_${o.key}("${vals.value[o.key]}")`).join('')
      const optArg = opt ? `,\n      thetadatadx::EndpointRequestOptions{}${opt}` : ''
      return `#include <thetadatadx/thetadatadx.hpp>
#include <iostream>

int main() {
  ${clientLine('cpp')}
  auto rows = client${hist('cpp')}.${c.method.cpp}(${reqArgs.value}${optArg});
  for (const auto& t : rows) {
    std::cout << ${cppPrint.value} << "\\n";
  }
}`
    }
    case 'curl': {
      const all = [...c.required, ...filledOpt.value]
      const lines = all.map((f) => `  --data-urlencode '${f.key}=${vals.value[f.key]}'`).join(' \\\n')
      return `curl -G 'http://127.0.0.1:25503/${c.httpPath}' \\
${lines}`
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
          <button :class="{ on: clientKind === 'historical' }" @click="clientKind = 'historical'">Historical</button>
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
      <div class="rb-resp-h"><span class="rb-dot" /> Sample response · <code>{{ cfg.returns || 'rows' }}</code></div>
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
