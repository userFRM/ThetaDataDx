<script setup lang="ts">
import { ref, computed, watch, onMounted, onBeforeUnmount } from 'vue'

// ─── Types ────────────────────────────────────────────────────────────────────

type RecipeId =
  | 'stock_price_history'
  | 'option_chain_snapshot'
  | 'gamma_exposure'
  | 'vol_surface'
  | 'unusual_activity'
  | 'put_call_ratio'
  | 'historical_greeks'
  | 'volume_profile'
  | 'market_calendar'
  | 'live_quote_monitor'
  | 'trade_tape'
  | 'option_flow_scanner'
  | 'live_option_chain'

type Language = 'python' | 'rust'

interface Recipe {
  id: RecipeId
  icon: string
  title: string
  description: string
  category: 'historical' | 'realtime'
  params: ParamKey[]
}

type ParamKey =
  | 'symbol'
  | 'symbols_list'
  | 'date_range'
  | 'single_date'
  | 'interval'
  | 'expiration'
  | 'strike'
  | 'right'
  | 'year'
  | 'min_size'

interface SymbolSuggestion {
  symbol: string
  name: string
  category: string
}

interface DatePreset {
  label: string
  value: () => string
  rangeValue?: () => { start: string; end: string }
  isRange?: boolean
}

// ─── Constants ────────────────────────────────────────────────────────────────

const SYMBOL_SUGGESTIONS: SymbolSuggestion[] = [
  // Mega Cap
  { symbol: 'AAPL',  name: 'Apple Inc.',              category: 'Mega Cap' },
  { symbol: 'MSFT',  name: 'Microsoft Corp.',          category: 'Mega Cap' },
  { symbol: 'GOOG',  name: 'Alphabet Inc.',            category: 'Mega Cap' },
  { symbol: 'AMZN',  name: 'Amazon.com Inc.',          category: 'Mega Cap' },
  { symbol: 'NVDA',  name: 'NVIDIA Corp.',             category: 'Mega Cap' },
  { symbol: 'META',  name: 'Meta Platforms Inc.',      category: 'Mega Cap' },
  { symbol: 'TSLA',  name: 'Tesla Inc.',               category: 'Mega Cap' },
  { symbol: 'BRK.B', name: 'Berkshire Hathaway B',    category: 'Mega Cap' },
  { symbol: 'NFLX',  name: 'Netflix Inc.',             category: 'Mega Cap' },
  { symbol: 'JPM',   name: 'JPMorgan Chase & Co.',    category: 'Mega Cap' },
  { symbol: 'V',     name: 'Visa Inc.',                category: 'Mega Cap' },
  { symbol: 'AVGO',  name: 'Broadcom Inc.',            category: 'Mega Cap' },
  // Popular ETFs
  { symbol: 'SPY',   name: 'SPDR S&P 500 ETF',         category: 'ETF' },
  { symbol: 'QQQ',   name: 'Invesco QQQ Trust',         category: 'ETF' },
  { symbol: 'IWM',   name: 'iShares Russell 2000 ETF',  category: 'ETF' },
  { symbol: 'DIA',   name: 'SPDR Dow Jones ETF',        category: 'ETF' },
  { symbol: 'XLF',   name: 'Financial Select SPDR',     category: 'ETF' },
  { symbol: 'XLE',   name: 'Energy Select SPDR',        category: 'ETF' },
  { symbol: 'GLD',   name: 'SPDR Gold Shares',          category: 'ETF' },
  { symbol: 'TLT',   name: 'iShares 20+ Year Treasury', category: 'ETF' },
  { symbol: 'TQQQ',  name: 'ProShares UltraPro QQQ',    category: 'ETF' },
  { symbol: 'SQQQ',  name: 'ProShares UltraPro Short',  category: 'ETF' },
  { symbol: 'ARKK',  name: 'ARK Innovation ETF',        category: 'ETF' },
  { symbol: 'VXX',   name: 'iPath Series B S&P 500 VIX', category: 'ETF' },
  { symbol: 'XLK',   name: 'Technology Select SPDR',    category: 'ETF' },
  // Index
  { symbol: 'VIX',   name: 'CBOE Volatility Index',     category: 'Index' },
  // Meme / Active
  { symbol: 'GME',   name: 'GameStop Corp.',            category: 'Active' },
  { symbol: 'AMC',   name: 'AMC Entertainment',         category: 'Active' },
  { symbol: 'PLTR',  name: 'Palantir Technologies',     category: 'Active' },
  { symbol: 'SOFI',  name: 'SoFi Technologies',         category: 'Active' },
  { symbol: 'RIVN',  name: 'Rivian Automotive',         category: 'Active' },
  { symbol: 'LCID',  name: 'Lucid Group Inc.',          category: 'Active' },
  { symbol: 'SMCI',  name: 'Super Micro Computer',      category: 'Active' },
  { symbol: 'ARM',   name: 'Arm Holdings plc',          category: 'Active' },
  { symbol: 'COIN',  name: 'Coinbase Global Inc.',      category: 'Active' },
  { symbol: 'MSTR',  name: 'MicroStrategy Inc.',        category: 'Active' },
]

// Inline SVG icons (24x24, stroke-based, currentColor)
const SVG_ICONS = {
  lineChart: '<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="22 12 18 12 15 21 9 3 6 12 2 12"/></svg>',
  chainLink: '<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.71"/><path d="M14 11a5 5 0 0 0-7.54-.54l-3 3a5 5 0 0 0 7.07 7.07l1.71-1.71"/></svg>',
  lightning: '<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polygon points="13 2 3 14 12 14 11 22 21 10 12 10 13 2"/></svg>',
  sineWave: '<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M2 12c2-4 4-8 6-8s4 8 6 8 4-8 6-8"/></svg>',
  magnifier: '<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="11" cy="11" r="8"/><line x1="21" y1="21" x2="16.65" y2="16.65"/></svg>',
  scale: '<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="12" y1="3" x2="12" y2="21"/><path d="M5 8l7-5 7 5"/><path d="M3 13l4 5h-4z"/><path d="M17 13l4 5h-4z"/><circle cx="5" cy="16" r="3"/><circle cx="19" cy="16" r="3"/></svg>',
  delta: '<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 4L4 20h16L12 4z"/></svg>',
  barChart: '<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="4" y1="6" x2="16" y2="6"/><line x1="4" y1="10" x2="20" y2="10"/><line x1="4" y1="14" x2="12" y2="14"/><line x1="4" y1="18" x2="18" y2="18"/></svg>',
  calendar: '<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="4" width="18" height="18" rx="2" ry="2"/><line x1="16" y1="2" x2="16" y2="6"/><line x1="8" y1="2" x2="8" y2="6"/><line x1="3" y1="10" x2="21" y2="10"/></svg>',
  radioTower: '<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M4.9 19.1C1 15.2 1 8.8 4.9 4.9"/><path d="M7.8 16.2c-2.3-2.3-2.3-6.1 0-8.4"/><circle cx="12" cy="12" r="2"/><path d="M16.2 7.8c2.3 2.3 2.3 6.1 0 8.4"/><path d="M19.1 4.9C23 8.8 23 15.2 19.1 19.1"/></svg>',
  tickerTape: '<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="2" y="7" width="20" height="10" rx="2"/><polyline points="6 11 9 14 13 10 18 15"/></svg>',
  flowArrows: '<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M5 12h14"/><path d="M15 6l6 6-6 6"/><path d="M5 6v12"/></svg>',
  gear: '<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 2.83-2.83l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z"/></svg>',
  tool: '<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M14.7 6.3a1 1 0 0 0 0 1.4l1.6 1.6a1 1 0 0 0 1.4 0l3.77-3.77a6 6 0 0 1-7.94 7.94l-6.91 6.91a2.12 2.12 0 0 1-3-3l6.91-6.91a6 6 0 0 1 7.94-7.94l-3.76 3.76z"/></svg>',
} as const

const RECIPES: Recipe[] = [
  // Historical
  {
    id: 'stock_price_history',
    icon: SVG_ICONS.lineChart,
    title: 'Stock Price History',
    description: 'OHLC bars, EOD data, or trade ticks for any symbol',
    category: 'historical',
    params: ['symbol', 'date_range', 'interval'],
  },
  {
    id: 'option_chain_snapshot',
    icon: SVG_ICONS.chainLink,
    title: 'Option Chain Snapshot',
    description: 'Full chain with Greeks for a given expiration',
    category: 'historical',
    params: ['symbol', 'expiration'],
  },
  {
    id: 'gamma_exposure',
    icon: SVG_ICONS.lightning,
    title: 'Gamma Exposure (GEX)',
    description: 'Net gamma exposure across all strikes — dealer positioning',
    category: 'historical',
    params: ['symbol', 'expiration'],
  },
  {
    id: 'vol_surface',
    icon: SVG_ICONS.sineWave,
    title: 'Volatility Surface',
    description: 'IV across strikes and expirations for a 3D surface',
    category: 'historical',
    params: ['symbol'],
  },
  {
    id: 'unusual_activity',
    icon: SVG_ICONS.magnifier,
    title: 'Unusual Options Activity',
    description: 'High volume/OI ratio contracts signaling smart money',
    category: 'historical',
    params: ['symbol', 'single_date'],
  },
  {
    id: 'put_call_ratio',
    icon: SVG_ICONS.scale,
    title: 'Put/Call Ratio',
    description: 'Aggregate put vs call volume and open interest',
    category: 'historical',
    params: ['symbol', 'expiration'],
  },
  {
    id: 'historical_greeks',
    icon: SVG_ICONS.delta,
    title: 'Historical Greeks',
    description: 'Track how Greeks evolved over time for a contract',
    category: 'historical',
    params: ['symbol', 'expiration', 'strike', 'right', 'single_date', 'interval'],
  },
  {
    id: 'volume_profile',
    icon: SVG_ICONS.barChart,
    title: 'Volume Profile',
    description: 'Trade distribution by price level across a date range',
    category: 'historical',
    params: ['symbol', 'date_range', 'interval'],
  },
  {
    id: 'market_calendar',
    icon: SVG_ICONS.calendar,
    title: 'Market Calendar',
    description: 'Trading days, holidays, and early closes for a year',
    category: 'historical',
    params: ['year'],
  },
  // Real-Time
  {
    id: 'live_quote_monitor',
    icon: SVG_ICONS.radioTower,
    title: 'Live Quote Monitor',
    description: 'Real-time bid/ask stream for one or more stocks',
    category: 'realtime',
    params: ['symbols_list'],
  },
  {
    id: 'trade_tape',
    icon: SVG_ICONS.tickerTape,
    title: 'Trade Tape',
    description: 'Real-time trade stream with symbol filtering',
    category: 'realtime',
    params: ['symbols_list'],
  },
  {
    id: 'option_flow_scanner',
    icon: SVG_ICONS.flowArrows,
    title: 'Option Flow Scanner',
    description: 'Full option trade firehose — flag unusual premium',
    category: 'realtime',
    params: ['min_size'],
  },
  {
    id: 'live_option_chain',
    icon: SVG_ICONS.gear,
    title: 'Live Option Chain',
    description: 'Streaming quotes for an entire chain in real time',
    category: 'realtime',
    params: ['symbol', 'expiration'],
  },
]

const INTERVAL_OPTIONS = [
  { label: 'Every tick', value: '0' },
  { label: '1 min',      value: '60000' },
  { label: '5 min',      value: '300000' },
  { label: '15 min',     value: '900000' },
  { label: '30 min',     value: '1800000' },
  { label: '1 hour',     value: '3600000' },
]

// ─── Date helpers ─────────────────────────────────────────────────────────────

function toYYYYMMDD(d: Date): string {
  const y = d.getFullYear()
  const m = String(d.getMonth() + 1).padStart(2, '0')
  const dy = String(d.getDate()).padStart(2, '0')
  return `${y}${m}${dy}`
}

function prevWeekday(d: Date): Date {
  const result = new Date(d)
  const day = result.getDay()
  if (day === 0) result.setDate(result.getDate() - 2)
  else if (day === 6) result.setDate(result.getDate() - 1)
  return result
}

function lastTradingDay(): string {
  const now = new Date()
  // Before 4pm on a weekday use today, otherwise previous weekday
  const day = now.getDay()
  const hour = now.getHours()
  const isWeekday = day >= 1 && day <= 5
  if (isWeekday && hour < 16) return toYYYYMMDD(now)
  return toYYYYMMDD(prevWeekday(now))
}

function daysAgo(n: number): string {
  const d = new Date()
  d.setDate(d.getDate() - n)
  return toYYYYMMDD(prevWeekday(d))
}

function ytdStart(): string {
  const d = new Date()
  return `${d.getFullYear()}0101`
}

const DATE_PRESETS: DatePreset[] = [
  {
    label: 'Today',
    isRange: false,
    value: () => lastTradingDay(),
  },
  {
    label: 'Yesterday',
    isRange: false,
    value: () => daysAgo(1),
  },
  {
    label: 'Last Week',
    isRange: true,
    value: () => daysAgo(7),
    rangeValue: () => ({ start: daysAgo(7), end: lastTradingDay() }),
  },
  {
    label: 'Last Month',
    isRange: true,
    value: () => daysAgo(30),
    rangeValue: () => ({ start: daysAgo(30), end: lastTradingDay() }),
  },
  {
    label: 'Last 3 Months',
    isRange: true,
    value: () => daysAgo(90),
    rangeValue: () => ({ start: daysAgo(90), end: lastTradingDay() }),
  },
  {
    label: 'YTD',
    isRange: true,
    value: () => ytdStart(),
    rangeValue: () => ({ start: ytdStart(), end: lastTradingDay() }),
  },
  {
    label: 'Last Year',
    isRange: true,
    value: () => daysAgo(365),
    rangeValue: () => ({ start: daysAgo(365), end: lastTradingDay() }),
  },
]

// ─── State ────────────────────────────────────────────────────────────────────

const step = ref<1 | 2 | 3 | 4>(1)
const selectedRecipe = ref<Recipe | null>(null)
const language = ref<Language>('python')
const copied = ref(false)

const params = ref({
  symbol:      'AAPL',
  symbolsRaw:  'AAPL, MSFT, SPY',
  start_date:  daysAgo(30),
  end_date:    lastTradingDay(),
  date:        lastTradingDay(),
  interval:    '300000',
  expiration:  '20251219',
  strike:      '500000',
  right:       'C' as 'C' | 'P',
  year:        String(new Date().getFullYear()),
  min_size:    '100',
})

// Symbol autocomplete
const symbolInput = ref('')
const symbolInputFocused = ref(false)
const autocompleteVisible = ref(false)
const acSelectedIdx = ref(-1)
const autocompleteRef = ref<HTMLElement | null>(null)

// Active date preset for range/single
const activeDatePreset = ref<string>('')
const activeSinglePreset = ref<string>('')

// ─── Computed ─────────────────────────────────────────────────────────────────

const historicalRecipes = computed(() => RECIPES.filter(r => r.category === 'historical'))
const realtimeRecipes   = computed(() => RECIPES.filter(r => r.category === 'realtime'))

const currentRecipe = computed(() => selectedRecipe.value)

const needsSymbol       = computed(() => currentRecipe.value?.params.includes('symbol') ?? false)
const needsSymbolsList  = computed(() => currentRecipe.value?.params.includes('symbols_list') ?? false)
const needsDateRange    = computed(() => currentRecipe.value?.params.includes('date_range') ?? false)
const needsSingleDate   = computed(() => currentRecipe.value?.params.includes('single_date') ?? false)
const needsInterval     = computed(() => currentRecipe.value?.params.includes('interval') ?? false)
const needsExpiration   = computed(() => currentRecipe.value?.params.includes('expiration') ?? false)
const needsStrike       = computed(() => currentRecipe.value?.params.includes('strike') ?? false)
const needsRight        = computed(() => currentRecipe.value?.params.includes('right') ?? false)
const needsYear         = computed(() => currentRecipe.value?.params.includes('year') ?? false)
const needsMinSize      = computed(() => currentRecipe.value?.params.includes('min_size') ?? false)

const filteredSuggestions = computed(() => {
  const q = symbolInput.value.toUpperCase().trim()
  if (!q) return SYMBOL_SUGGESTIONS
  return SYMBOL_SUGGESTIONS.filter(
    s => s.symbol.startsWith(q) || s.name.toUpperCase().includes(q)
  )
})

const symbolsByCategory = computed(() => {
  const map = new Map<string, SymbolSuggestion[]>()
  for (const s of filteredSuggestions.value) {
    if (!map.has(s.category)) map.set(s.category, [])
    map.get(s.category)!.push(s)
  }
  return map
})

const symbolsList = computed(() =>
  params.value.symbolsRaw
    .split(/[,\s]+/)
    .map(s => s.trim().toUpperCase())
    .filter(Boolean)
)

const generatedCode = computed((): string => {
  if (!currentRecipe.value) return ''
  if (language.value === 'python') return genPython()
  return genRust()
})

// ─── Actions ─────────────────────────────────────────────────────────────────

function pickRecipe(recipe: Recipe) {
  selectedRecipe.value = recipe
  symbolInput.value = params.value.symbol
  step.value = 2
}

function goStep(n: 1 | 2 | 3 | 4) {
  if (n <= step.value) step.value = n
}

function toStep3() {
  step.value = 3
}

function toStep4(lang: Language) {
  language.value = lang
  if (typeof localStorage !== 'undefined') localStorage.setItem('qb-lang', lang)
  step.value = 4
}

function startOver() {
  step.value = 1
  selectedRecipe.value = null
  activeDatePreset.value = ''
  activeSinglePreset.value = ''
}

function selectDatePreset(preset: DatePreset) {
  if (preset.isRange && preset.rangeValue) {
    const { start, end } = preset.rangeValue()
    params.value.start_date = start
    params.value.end_date = end
    activeDatePreset.value = preset.label
  } else {
    params.value.date = preset.value()
    activeSinglePreset.value = preset.label
  }
}

function selectSymbol(s: SymbolSuggestion) {
  params.value.symbol = s.symbol
  symbolInput.value = s.symbol
  autocompleteVisible.value = false
  acSelectedIdx.value = -1
}

function onSymbolInput() {
  params.value.symbol = symbolInput.value.toUpperCase()
  autocompleteVisible.value = true
  acSelectedIdx.value = -1
}

function onSymbolFocus() {
  symbolInputFocused.value = true
  autocompleteVisible.value = true
}

function onSymbolBlur() {
  // Delay to allow click on suggestion
  setTimeout(() => {
    autocompleteVisible.value = false
    symbolInputFocused.value = false
  }, 150)
}

function onSymbolKeydown(e: KeyboardEvent) {
  const flat = filteredSuggestions.value
  if (e.key === 'ArrowDown') {
    e.preventDefault()
    acSelectedIdx.value = Math.min(acSelectedIdx.value + 1, flat.length - 1)
  } else if (e.key === 'ArrowUp') {
    e.preventDefault()
    acSelectedIdx.value = Math.max(acSelectedIdx.value - 1, -1)
  } else if (e.key === 'Enter' && acSelectedIdx.value >= 0) {
    e.preventDefault()
    selectSymbol(flat[acSelectedIdx.value])
  } else if (e.key === 'Escape') {
    autocompleteVisible.value = false
  }
}

async function copyCode() {
  if (!generatedCode.value) return
  try {
    await navigator.clipboard.writeText(generatedCode.value)
    copied.value = true
    setTimeout(() => { copied.value = false }, 2000)
  } catch {
    // fallback
    const ta = document.createElement('textarea')
    ta.value = generatedCode.value
    document.body.appendChild(ta)
    ta.select()
    document.execCommand('copy')
    document.body.removeChild(ta)
    copied.value = true
    setTimeout(() => { copied.value = false }, 2000)
  }
}

// ─── Init ─────────────────────────────────────────────────────────────────────

onMounted(() => {
  if (typeof localStorage !== 'undefined') {
    const saved = localStorage.getItem('qb-lang')
    if (saved === 'python' || saved === 'rust') language.value = saved
  }
  symbolInput.value = params.value.symbol
})

// ─── Syntax highlighting (regex-based) ───────────────────────────────────────

function highlight(code: string, lang: 'python' | 'rust'): string {
  // Placeholder-based highlighting to prevent regex cross-contamination.
  // 1. Extract tokens into a list, replace with \x00N\x00 placeholders
  // 2. Apply remaining highlights on the placeholder'd text
  // 3. Re-inject extracted tokens at the end
  const tokens: string[] = []
  function stash(cls: string, text: string): string {
    const i = tokens.length
    tokens.push(`<span class="hl-${cls}">${text}</span>`)
    return `\x00${i}\x00`
  }

  // Escape HTML first
  let s = code
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')

  if (lang === 'python') {
    // Strings - stash first to protect from later regexes
    s = s.replace(/("""[\s\S]*?"""|'''[\s\S]*?'''|f?"(?:[^"\\]|\\.)*"|f?'(?:[^'\\]|\\.)*')/g,
      (m) => stash('string', m))
    // Comments
    s = s.replace(/(#[^\n]*)/g, (m) => stash('comment', m))
    // Keywords
    s = s.replace(/\b(from|import|def|class|return|for|in|if|else|elif|while|True|False|None|and|or|not|with|as|try|except|finally|lambda|yield|pass|break|continue|async|await|raise|del|global|nonlocal|assert)\b/g,
      '<span class="hl-keyword">$1</span>')
    // Built-ins
    s = s.replace(/\b(print|len|range|str|int|float|list|dict|set|tuple|bool|type|isinstance|enumerate|zip|map|filter|sorted|reversed|sum|min|max|abs|round|open|super|divmod)\b/g,
      '<span class="hl-builtin">$1</span>')
    // Decorators
    s = s.replace(/(@\w+)/g, '<span class="hl-decorator">$1</span>')
  } else {
    // Rust - strings first
    s = s.replace(/(r?"(?:[^"\\]|\\.)*")/g, (m) => stash('string', m))
    // Comments
    s = s.replace(/(\/\/[^\n]*)/g, (m) => stash('comment', m))
    // Attributes
    s = s.replace(/(#\[.*?\])/g, (m) => stash('decorator', m))
    // Keywords
    s = s.replace(/\b(use|fn|let|mut|pub|struct|enum|impl|trait|for|in|if|else|while|loop|match|return|async|await|move|ref|type|where|const|static|mod|crate|self|Self|super|as|break|continue|unsafe|extern|dyn|true|false)\b/g,
      '<span class="hl-keyword">$1</span>')
    // Types
    s = s.replace(/\b(String|Vec|HashMap|Option|Result|Box|Arc|Mutex|i8|i16|i32|i64|i128|u8|u16|u32|u64|u128|f32|f64|bool|usize|isize|str)\b/g,
      '<span class="hl-type">$1</span>')
    // Macros
    s = s.replace(/\b(\w+!)/g, '<span class="hl-macro">$1</span>')
  }

  // Re-inject stashed tokens
  s = s.replace(/\x00(\d+)\x00/g, (_, i) => tokens[parseInt(i)])

  return s
}

const highlightedCode = computed(() => highlight(generatedCode.value, language.value))

// ─── Code generators ─────────────────────────────────────────────────────────

function sym(): string { return params.value.symbol.toUpperCase() }
function exp(): string { return params.value.expiration }
function strike(): string { return params.value.strike }
function right(): string { return params.value.right }
function interval(): string { return params.value.interval }
function startDate(): string { return params.value.start_date }
function endDate(): string { return params.value.end_date }
function singleDate(): string { return params.value.date }
function minSize(): string { return params.value.min_size }
function yearVal(): string { return params.value.year }
function symsListPy(): string {
  return symbolsList.value.map(s => `"${s}"`).join(', ')
}
function symsListRust(): string {
  return symbolsList.value.map(s => `"${s}"`).join(', ')
}

function pyHeader(): string {
  return `from thetadatadx import ThetaDataDx, Credentials, Config

creds = Credentials.from_file("creds.txt")
# Or inline: creds = Credentials("user@example.com", "your-password")
tdx = ThetaDataDx(creds, Config.production())`
}

function rustHeader(): string {
  return `use thetadatadx::{ThetaDataDx, Credentials, DirectConfig};`
}

function rustMain(body: string): string {
  return `${rustHeader()}

#[tokio::main]
async fn main() -> Result<(), thetadatadx::Error> {
    let creds = Credentials::from_file("creds.txt")?;
    // Or inline: let creds = Credentials::new("user@example.com", "your-password");
    let tdx = ThetaDataDx::connect(&creds, DirectConfig::production()).await?;

${body}
    Ok(())
}`
}

function genPython(): string {
  const id = currentRecipe.value!.id
  const h = pyHeader()

  switch (id) {
    case 'stock_price_history': return `${h}

symbol = "${sym()}"

# EOD bars (date range)
eod = tdx.stock_history_eod(symbol, "${startDate()}", "${endDate()}")
for tick in eod:
    print(f"{tick['date']}: open={tick['open']} high={tick['high']} low={tick['low']} close={tick['close']} vol={tick['volume']}")

# Intraday OHLC (interval: ${interval()} ms)
ohlc = tdx.stock_history_ohlc(symbol, "${endDate()}", "${interval()}")
for tick in ohlc:
    print(f"{tick['date']} ms={tick['ms_of_day']}: open={tick['open']} close={tick['close']} vol={tick['volume']}")`

    case 'option_chain_snapshot': return `${h}

symbol = "${sym()}"
exp    = "${exp()}"

# Get all strikes for this expiration
strikes = tdx.option_list_strikes(symbol, exp)
print(f"Found {len(strikes)} strikes for {symbol} {exp}")

# Fetch Greeks for each strike (calls + puts)
chain = []
for strike in strikes:
    for right in ["C", "P"]:
        greeks = tdx.option_snapshot_greeks_all(symbol, exp, strike, right)
        if greeks:
            chain.append({
                "strike": int(strike) / 1000,  # scaled int -> dollars
                "right":  right,
                **greeks[0],
            })

from thetadatadx import to_dataframe
df = to_dataframe(chain)
print(df[["strike", "right", "implied_volatility", "delta", "gamma", "theta", "vega"]].to_string())`

    case 'gamma_exposure': return `${h}

symbol = "${sym()}"
exp    = "${exp()}"

strikes  = tdx.option_list_strikes(symbol, exp)

gex_data = []
for strike in strikes:
    for right in ["C", "P"]:
        greeks = tdx.option_snapshot_greeks_all(symbol, exp, strike, right)
        oi     = tdx.option_snapshot_open_interest(symbol, exp, strike, right)
        if greeks and oi:
            gamma        = greeks[0]["gamma"]
            open_interest = oi[0]["open_interest"]
            # GEX = gamma * OI * 100 * spot^2 / 10^7
            # Calls add positive gamma, puts subtract (dealers are short puts)
            sign = 1 if right == "C" else -1
            gex_data.append({
                "strike": int(strike) / 1000,  # scaled int -> dollars
                "right":  right,
                "gamma":  gamma,
                "oi":     open_interest,
                "gex":    sign * gamma * open_interest * 100,
            })

from thetadatadx import to_dataframe
df = to_dataframe(gex_data)
print(df.sort_values("strike").to_string())
print(f"\\nNet GEX: {df['gex'].sum():.2f}")`

    case 'vol_surface': return `${h}

symbol = "${sym()}"

# Get all available expirations
exps = tdx.option_list_expirations(symbol)
print(f"Found {len(exps)} expirations")

surface = []
for exp in exps[:8]:  # first 8 expirations for a manageable surface
    strikes = tdx.option_list_strikes(symbol, exp)
    for strike in strikes:
        iv_data = tdx.option_snapshot_greeks_implied_volatility(symbol, exp, strike, "C")
        if iv_data and iv_data[0]["implied_volatility"] > 0:
            surface.append({
                "expiration": exp,
                "strike":     int(strike) / 1000,  # scaled int -> dollars
                "iv":         iv_data[0]["implied_volatility"],
            })

from thetadatadx import to_dataframe
df    = to_dataframe(surface)
pivot = df.pivot(index="strike", columns="expiration", values="iv")
print(pivot.to_string())`

    case 'unusual_activity': return `${h}

symbol = "${sym()}"
date   = "${singleDate()}"

# Get all option contracts that traded on this date
contracts = tdx.option_list_contracts("EOD", symbol, date)
print(f"Scanning {len(contracts)} contracts...")

unusual = []
for c in contracts:
    exp_c   = c["expiration"]
    strike_c = c["strike"]
    right_c  = c["right"]

    oi_data = tdx.option_history_open_interest(symbol, str(exp_c), str(strike_c), right_c, date)
    trades  = tdx.option_history_trade(symbol, str(exp_c), str(strike_c), right_c, date)

    if oi_data and trades:
        total_volume = len(trades)
        open_int     = oi_data[0]["open_interest"] if oi_data[0]["open_interest"] > 0 else 1
        vol_oi       = total_volume / open_int

        if vol_oi > 2.0:  # volume > 2x open interest = unusual
            unusual.append({
                "contract":     f"{symbol} {exp_c} {right_c} {int(strike_c)/1000}",
                "volume":       total_volume,
                "oi":           open_int,
                "vol_oi_ratio": round(vol_oi, 2),
            })

from thetadatadx import to_dataframe
df = to_dataframe(unusual)
if df.empty:
    print("No unusual activity found.")
else:
    print(df.sort_values("vol_oi_ratio", ascending=False).head(20).to_string())`

    case 'put_call_ratio': return `${h}

symbol = "${sym()}"
exp    = "${exp()}"

strikes = tdx.option_list_strikes(symbol, exp)

total_call_vol = 0
total_put_vol  = 0
total_call_oi  = 0
total_put_oi   = 0

for strike in strikes:
    for right in ["C", "P"]:
        snap = tdx.option_snapshot_trade(symbol, exp, strike, right)
        oi   = tdx.option_snapshot_open_interest(symbol, exp, strike, right)

        if snap:
            if right == "C":
                total_call_vol += snap[0].get("size", 0)
            else:
                total_put_vol  += snap[0].get("size", 0)

        if oi:
            if right == "C":
                total_call_oi += oi[0].get("open_interest", 0)
            else:
                total_put_oi  += oi[0].get("open_interest", 0)

call_vol = total_call_vol if total_call_vol > 0 else 1
call_oi  = total_call_oi  if total_call_oi  > 0 else 1

print(f"  Call volume : {total_call_vol:>10,}")
print(f"  Put  volume : {total_put_vol:>10,}")
print(f"  P/C vol ratio: {total_put_vol / call_vol:.4f}")
print()
print(f"  Call OI     : {total_call_oi:>10,}")
print(f"  Put  OI     : {total_put_oi:>10,}")
print(f"  P/C OI ratio: {total_put_oi / call_oi:.4f}")`

    case 'historical_greeks': return `${h}

symbol = "${sym()}"
exp    = "${exp()}"
# Strike uses scaled integers: 500000 = $500.00
strike = "${strike()}"
right  = "${right()}"  # "C" for call, "P" for put

# Fetch historical Greeks at ${interval()} ms interval
ticks = tdx.option_history_greeks_all(symbol, exp, strike, right, "${singleDate()}", "${interval()}")
for tick in ticks:
    print(
        f"{tick['date']} ms={tick['ms_of_day']:>8}: "
        f"iv={tick['implied_volatility']:.4f}  "
        f"delta={tick['delta']:+.4f}  "
        f"gamma={tick['gamma']:.6f}  "
        f"theta={tick['theta']:+.4f}  "
        f"vega={tick['vega']:.4f}"
    )

print(f"\\nTotal ticks: {len(ticks)}")`

    case 'volume_profile': return `${h}

symbol = "${sym()}"

# Fetch all trades over the date range
from collections import defaultdict
price_buckets: dict = defaultdict(int)

# Get each day's trades and aggregate by price bucket
from datetime import date, timedelta

start = "${startDate()}"
end   = "${endDate()}"

# Build list of trading dates
exps = []
current = date(int(start[:4]), int(start[4:6]), int(start[6:]))
end_dt  = date(int(end[:4]),   int(end[4:6]),   int(end[6:]))

while current <= end_dt:
    if current.weekday() < 5:  # Monday-Friday
        exps.append(current.strftime("%Y%m%d"))
    current += timedelta(days=1)

for trading_date in exps:
    trades = tdx.stock_history_trade(symbol, trading_date)
    for t in trades:
        # Round price to nearest $0.25 bucket
        bucket = round(t["price"] * 4) / 4
        price_buckets[bucket] += t["size"]

# Sort and display
sorted_profile = sorted(price_buckets.items())
max_vol = max(v for _, v in sorted_profile) if sorted_profile else 1
print(f"Volume Profile for {symbol} ({start} - {end}):")
print(f"{'Price':>10}  {'Volume':>12}  {'Bar'}")
print("-" * 50)
for price, vol in sorted_profile:
    bar_len = int(vol / max_vol * 40)
    bar = "#" * bar_len
    print(f"\${price:>9.2f}  {vol:>12,}  {bar}")`

    case 'market_calendar': return `${h}

# Get all trading days, holidays, and early closes for ${yearVal()}
days = tdx.calendar_year("${yearVal()}")

trading_days = [d for d in days if d["is_open"]]
holidays     = [d for d in days if not d["is_open"]]
# Early close = close_time before 57600000 ms (4:00 PM ET)
early_closes = [d for d in trading_days if d["close_time"] < 57600000]

print(f"Year ${yearVal()} Summary:")
print(f"  Trading days : {len(trading_days)}")
print(f"  Holidays     : {len(holidays)}")
print(f"  Early closes : {len(early_closes)}")
print()
print("Holidays:")
for d in holidays:
    print(f"  {d['date']}")
print()
print("Early Closes:")
for d in early_closes:
    print(f"  {d['date']}  closes={d['close_time']}")`

    case 'live_quote_monitor': return `${h}

tdx.start_streaming()

# Subscribe to multiple symbols
symbols = [${symsListPy()}]
for sym in symbols:
    tdx.subscribe_quotes(sym)

print(f"Monitoring quotes for: {symbols}")
print(f"{'Symbol':<8}  {'Bid':>8}  {'Ask':>8}  {'Spread':>8}  {'Mid':>8}")
print("-" * 50)

contracts = {}
while True:
    event = tdx.next_event(timeout_ms=5000)
    if event is None:
        continue
    if event["kind"] == "contract_assigned":
        contracts[event["id"]] = event["detail"]
    elif event["kind"] == "quote":
        name   = contracts.get(event.get("contract_id"), "?")
        bid    = event["bid"]
        ask    = event["ask"]
        spread = ask - bid
        mid    = (bid + ask) / 2
        print(f"\\r{name:<8}  {bid:>8.2f}  {ask:>8.2f}  {spread:>8.4f}  {mid:>8.2f}", end="", flush=True)`

    case 'trade_tape': return `${h}

tdx.start_streaming()

# Subscribe to trade stream for each symbol
symbols = [${symsListPy()}]
for sym in symbols:
    tdx.subscribe_trades(sym)

print(f"Trade tape for: {symbols}")
print(f"{'Time':>12}  {'Symbol':<8}  {'Price':>8}  {'Size':>8}  {'Cond'}")
print("-" * 55)

contracts = {}
while True:
    event = tdx.next_event(timeout_ms=5000)
    if event is None:
        continue
    if event["kind"] == "contract_assigned":
        contracts[event["id"]] = event["detail"]
    elif event["kind"] == "trade":
        name  = contracts.get(event.get("contract_id"), "?")
        price = event["price"]
        size  = event["size"]
        ms    = event.get("ms_of_day", 0)
        h, remainder = divmod(ms, 3600000)
        m, s_ms      = divmod(remainder, 60000)
        s            = s_ms // 1000
        time_str     = f"{h:02d}:{m:02d}:{s:02d}"
        cond         = event.get("condition", "")
        print(f"{time_str:>12}  {name:<8}  {price:>8.2f}  {size:>8,}  {cond}")`

    case 'option_flow_scanner': return `${h}

tdx.start_streaming()
tdx.subscribe_full_trades("OPTION")

print(f"Option Flow Scanner — alerting on size >= ${minSize()}")
print(f"{'Contract':<35}  {'Size':>6}  {'Price':>8}  {'Premium':>12}")
print("-" * 70)

contracts = {}
while True:
    event = tdx.next_event(timeout_ms=5000)
    if event is None:
        continue
    if event["kind"] == "contract_assigned":
        contracts[event["id"]] = event["detail"]
    elif event["kind"] == "trade":
        contract = contracts.get(event.get("contract_id"), "?")
        size     = event["size"]
        price    = event["price"]

        if size >= ${minSize()}:
            premium = price * size * 100
            print(f"{contract:<35}  {size:>6,}  {price:>8.2f}  \${premium:>11,.0f}")`

    case 'live_option_chain': return `${h}

tdx.start_streaming()

symbol = "${sym()}"
exp    = "${exp()}"

# Get all strikes, then subscribe to option quotes for each contract
strikes = tdx.option_list_strikes(symbol, exp)
for strike in strikes:
    for right in ["C", "P"]:
        tdx.subscribe_option_quotes(symbol, exp, strike, right)

print(f"Live chain: {symbol} {exp}  ({len(strikes) * 2} contracts)")
print(f"{'Contract':<30}  {'Bid':>8}  {'Ask':>8}  {'Spread':>8}  {'Mid':>8}")
print("-" * 75)

chain_state: dict = {}
contracts = {}
while True:
    event = tdx.next_event(timeout_ms=5000)
    if event is None:
        continue
    if event["kind"] == "contract_assigned":
        contracts[event["id"]] = event["detail"]
    elif event["kind"] == "quote":
        name = contracts.get(event.get("contract_id"), "?")
        bid  = event["bid"]
        ask  = event["ask"]
        chain_state[name] = {"bid": bid, "ask": ask}
        # Reprint sorted by contract name
        print("\\033[H\\033[J", end="")  # clear screen
        for k, v in sorted(chain_state.items()):
            spread = v["ask"] - v["bid"]
            mid    = (v["bid"] + v["ask"]) / 2
            print(
                f"{k:<30}  {v['bid']:>8.2f}  {v['ask']:>8.2f}  "
                f"{spread:>8.4f}  {mid:>8.2f}"
            )`

    default:
      return '# Recipe not yet implemented'
  }
}

function genRust(): string {
  const id = currentRecipe.value!.id
  const h = rustHeader()

  switch (id) {
    case 'stock_price_history': return rustMain(`    let symbol = "${sym()}";

    // EOD bars (date range)
    let eod = tdx.stock_history_eod(symbol, "${startDate()}", "${endDate()}").await?;
    println!("EOD bars: {} records", eod.len());
    for tick in &eod {
        println!("{}: open={} high={} low={} close={} vol={}",
            tick.date, tick.open_price(), tick.high_price(),
            tick.low_price(), tick.close_price(), tick.volume);
    }

    // Intraday OHLC (interval: ${interval()} ms)
    let ohlc = tdx.stock_history_ohlc(symbol, "${endDate()}", "${interval()}").await?;
    for tick in &ohlc {
        println!("{} ms={}: open={} close={} vol={}",
            tick.date, tick.ms_of_day, tick.open_price(), tick.close_price(), tick.volume);
    }`)

    case 'option_chain_snapshot': return rustMain(`    let symbol = "${sym()}";
    let exp    = "${exp()}";

    // Get all strikes for this expiration
    let strikes = tdx.option_list_strikes(symbol, exp).await?;
    println!("Found {} strikes for {} {}", strikes.len(), symbol, exp);

    // Fetch Greeks for each strike (calls + puts)
    for strike in &strikes {
        for right in &["C", "P"] {
            if let Ok(greeks) = tdx.option_snapshot_greeks_all(symbol, exp, strike, right).await {
                if let Some(g) = greeks.first() {
                    // Strike is a scaled integer: divide by 1000 for dollar price
                    let strike_f = strike.parse::<f64>().unwrap_or(0.0) / 1000.0;
                    println!("{} \${:.2}: iv={:.4} delta={:+.4} gamma={:.6} theta={:+.4} vega={:.4}",
                        right, strike_f,
                        g.implied_volatility, g.delta, g.gamma, g.theta, g.vega);
                }
            }
        }
    }`)

    case 'gamma_exposure': return rustMain(`    let symbol = "${sym()}";
    let exp    = "${exp()}";

    let strikes = tdx.option_list_strikes(symbol, exp).await?;

    let mut net_gex: f64 = 0.0;
    for strike in &strikes {
        for right in &["C", "P"] {
            let greeks_res = tdx.option_snapshot_greeks_all(symbol, exp, strike, right).await;
            let oi_res     = tdx.option_snapshot_open_interest(symbol, exp, strike, right).await;

            if let (Ok(greeks), Ok(oi)) = (greeks_res, oi_res) {
                if let (Some(g), Some(o)) = (greeks.first(), oi.first()) {
                    // GEX = gamma * OI * 100 (calls +, puts -)
                    let sign = if *right == "C" { 1.0_f64 } else { -1.0_f64 };
                    let gex  = sign * g.gamma * o.open_interest as f64 * 100.0;
                    net_gex += gex;
                    // Strike is scaled: 500000 = $500
                    let strike_f = strike.parse::<f64>().unwrap_or(0.0) / 1000.0;
                    println!("\${:.2} {}: gamma={:.6} oi={} gex={:.2}",
                        strike_f, right, g.gamma, o.open_interest, gex);
                }
            }
        }
    }
    println!("\\nNet GEX: {:.2}", net_gex);`)

    case 'vol_surface': return rustMain(`    let symbol = "${sym()}";

    // Fetch all available expirations
    let exps = tdx.option_list_expirations(symbol).await?;
    println!("Found {} expirations", exps.len());

    // Build vol surface — first 8 expirations
    for exp in exps.iter().take(8) {
        let strikes = tdx.option_list_strikes(symbol, exp).await?;
        for strike in &strikes {
            if let Ok(iv_data) = tdx.option_snapshot_greeks_implied_volatility(symbol, exp, strike, "C").await {
                if let Some(iv) = iv_data.first() {
                    if iv.implied_volatility > 0.0 {
                        // Strike is scaled: divide by 1000 for dollar price
                        let strike_f = strike.parse::<f64>().unwrap_or(0.0) / 1000.0;
                        println!("exp={} strike=\${:.2} iv={:.4}",
                            exp, strike_f, iv.implied_volatility);
                    }
                }
            }
        }
    }`)

    case 'unusual_activity': return rustMain(`    let symbol = "${sym()}";
    let date   = "${singleDate()}";

    // Get all contracts that traded on this date
    let contracts = tdx.option_list_contracts("EOD", symbol, date).await?;
    println!("Scanning {} contracts...", contracts.len());

    let mut unusual = Vec::new();
    for c in &contracts {
        let right_str = if c.right == 0 { "C" } else { "P" };
        let oi_res = tdx.option_history_open_interest(
            symbol, &c.expiration.to_string(), &c.strike.to_string(), right_str, date).await;
        let trades_res = tdx.option_history_trade(
            symbol, &c.expiration.to_string(), &c.strike.to_string(), right_str, date).await;

        if let (Ok(oi), Ok(trades)) = (oi_res, trades_res) {
            let volume   = trades.len() as f64;
            let oi_count = oi.first().map(|o| o.open_interest as f64).unwrap_or(1.0).max(1.0);
            let vol_oi   = volume / oi_count;

            if vol_oi > 2.0 {
                unusual.push((c.clone(), right_str, volume as usize, oi_count as usize, vol_oi));
            }
        }
    }

    unusual.sort_by(|a, b| b.4.partial_cmp(&a.4).unwrap());
    println!("\\n{:<35}  {:>8}  {:>8}  {:>10}", "Contract", "Volume", "OI", "Vol/OI");
    println!("{}", "-".repeat(70));
    for (c, right_str, vol, oi, ratio) in unusual.iter().take(20) {
        // Strike is scaled: divide by 1000 for dollar price
        println!("{} {} {} \${:.2}  {:>8}  {:>8}  {:>10.2}",
            symbol, c.expiration, right_str, c.strike as f64 / 1000.0, vol, oi, ratio);
    }`)

    case 'put_call_ratio': return rustMain(`    let symbol = "${sym()}";
    let exp    = "${exp()}";

    let strikes = tdx.option_list_strikes(symbol, exp).await?;

    let (mut call_vol, mut put_vol, mut call_oi, mut put_oi) = (0u64, 0u64, 0u64, 0u64);

    for strike in &strikes {
        for right in &["C", "P"] {
            let is_call = *right == "C";
            if let Ok(snap) = tdx.option_snapshot_trade(symbol, exp, strike, right).await {
                if let Some(s) = snap.first() {
                    if is_call { call_vol += s.size as u64; } else { put_vol += s.size as u64; }
                }
            }
            if let Ok(oi_data) = tdx.option_snapshot_open_interest(symbol, exp, strike, right).await {
                if let Some(o) = oi_data.first() {
                    if is_call { call_oi += o.open_interest as u64; } else { put_oi += o.open_interest as u64; }
                }
            }
        }
    }

    let cv = call_vol.max(1) as f64;
    let co = call_oi.max(1)  as f64;
    println!("  Call volume  : {:>10}", call_vol);
    println!("  Put  volume  : {:>10}", put_vol);
    println!("  P/C vol ratio: {:.4}", put_vol as f64 / cv);
    println!();
    println!("  Call OI      : {:>10}", call_oi);
    println!("  Put  OI      : {:>10}", put_oi);
    println!("  P/C OI ratio : {:.4}", put_oi as f64 / co);`)

    case 'historical_greeks': return rustMain(`    let symbol = "${sym()}";
    let exp    = "${exp()}";
    // Strike uses scaled integers: 500000 = $500.00
    let strike = "${strike()}";
    let right  = "${right()}";  // "C" for call, "P" for put

    let ticks = tdx.option_history_greeks_all(
        symbol, exp, strike, right, "${singleDate()}", "${interval()}").await?;

    println!("Historical Greeks: {} ticks", ticks.len());
    println!("{:>12}  {:>7}  {:>7}  {:>9}  {:>7}  {:>7}",
        "Time(ms)", "IV", "Delta", "Gamma", "Theta", "Vega");
    println!("{}", "-".repeat(65));
    for tick in &ticks {
        println!("{:>12}  {:>7.4}  {:>+7.4}  {:>9.6}  {:>+7.4}  {:>7.4}",
            tick.ms_of_day, tick.implied_volatility,
            tick.delta, tick.gamma, tick.theta, tick.vega);
    }`)

    case 'volume_profile': return rustMain(`    use std::collections::BTreeMap;

    let symbol = "${sym()}";

    let mut price_buckets: BTreeMap<u64, u64> = BTreeMap::new();

    // Fetch EOD list to know which dates had data
    let eod = tdx.stock_history_eod(symbol, "${startDate()}", "${endDate()}").await?;
    println!("Fetching trades for {} trading days...", eod.len());

    for day in &eod {
        if let Ok(trades) = tdx.stock_history_trade(symbol, &day.date.to_string()).await {
            for t in &trades {
                // Bucket to nearest $0.25 — use get_price() to convert raw int to f64
                let price_f = t.get_price().to_f64();
                let bucket = (price_f / 0.25).round() as u64;
                *price_buckets.entry(bucket).or_insert(0) += t.size as u64;
            }
        }
    }

    let max_vol = price_buckets.values().copied().max().unwrap_or(1);
    println!("\\nVolume Profile for {} ({} - {}):", symbol, "${startDate()}", "${endDate()}");
    println!("{:>10}  {:>12}  {}", "Price", "Volume", "Bar");
    println!("{}", "-".repeat(60));
    for (bucket, vol) in &price_buckets {
        let price  = *bucket as f64 * 0.25;
        let bar    = "#".repeat((*vol as f64 / max_vol as f64 * 40.0) as usize);
        println!("\${:>9.2}  {:>12}  {}", price, vol, bar);
    }`)

    case 'market_calendar': return rustMain(`    // Get all trading days, holidays, and early closes for ${yearVal()}
    let days = tdx.calendar_year("${yearVal()}").await?;

    let trading: Vec<_> = days.iter().filter(|d| d.is_open).collect();
    let holidays: Vec<_> = days.iter().filter(|d| !d.is_open).collect();
    // Early close = close_time before 57600000 ms (4:00 PM ET)
    let early: Vec<_> = trading.iter().filter(|d| d.close_time < 57600000).collect();

    println!("Year ${yearVal()} Summary:");
    println!("  Trading days : {}", trading.len());
    println!("  Holidays     : {}", holidays.len());
    println!("  Early closes : {}", early.len());
    println!();
    println!("Holidays:");
    for d in &holidays { println!("  {}", d.date); }
    println!();
    println!("Early Closes:");
    for d in &early { println!("  {}  closes={}", d.date, d.close_time); }`)

    case 'live_quote_monitor': return `${rustHeader()}
use thetadatadx::fpss::{FpssEvent, FpssData, FpssControl};
use thetadatadx::fpss::protocol::Contract;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[tokio::main]
async fn main() -> Result<(), thetadatadx::Error> {
    let creds = Credentials::from_file("creds.txt")?;
    // Or inline: let creds = Credentials::new("user@example.com", "your-password");
    let tdx = ThetaDataDx::connect(&creds, DirectConfig::production()).await?;

    let contracts: Arc<Mutex<HashMap<i32, String>>> = Arc::new(Mutex::new(HashMap::new()));
    let contracts_clone = contracts.clone();

    println!("Monitoring quotes for: [${symsListRust()}]");
    println!("{:<8}  {:>8}  {:>8}  {:>8}  {:>8}", "Symbol", "Bid", "Ask", "Spread", "Mid");
    println!("{}", "-".repeat(50));

    tdx.start_streaming(move |event: &FpssEvent| {
        match event {
            FpssEvent::Control(FpssControl::ContractAssigned { id, contract }) => {
                contracts_clone.lock().unwrap().insert(*id, format!("{contract}"));
            }
            FpssEvent::Data(FpssData::Quote { contract_id, bid, ask, price_type, .. }) => {
                let name = contracts_clone.lock().unwrap().get(contract_id).cloned().unwrap_or_default();
                let bid_f = tdbe::Price::new(*bid, *price_type).to_f64();
                let ask_f = tdbe::Price::new(*ask, *price_type).to_f64();
                let spread = ask_f - bid_f;
                let mid = (bid_f + ask_f) / 2.0;
                println!("{:<8}  {:>8.2}  {:>8.2}  {:>8.4}  {:>8.2}", name, bid_f, ask_f, spread, mid);
            }
            _ => {}
        }
    })?;

    let symbols = vec![${symsListRust()}];
    for sym in &symbols {
        tdx.subscribe_quotes(&Contract::stock(sym))?;
    }

    std::thread::park();
    Ok(())
}`

    case 'trade_tape': return `${rustHeader()}
use thetadatadx::fpss::{FpssEvent, FpssData, FpssControl};
use thetadatadx::fpss::protocol::Contract;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[tokio::main]
async fn main() -> Result<(), thetadatadx::Error> {
    let creds = Credentials::from_file("creds.txt")?;
    // Or inline: let creds = Credentials::new("user@example.com", "your-password");
    let tdx = ThetaDataDx::connect(&creds, DirectConfig::production()).await?;

    let contracts: Arc<Mutex<HashMap<i32, String>>> = Arc::new(Mutex::new(HashMap::new()));
    let contracts_clone = contracts.clone();

    println!("Trade tape for: [${symsListRust()}]");
    println!("{:>12}  {:<8}  {:>8}  {:>8}", "Time", "Symbol", "Price", "Size");
    println!("{}", "-".repeat(45));

    tdx.start_streaming(move |event: &FpssEvent| {
        match event {
            FpssEvent::Control(FpssControl::ContractAssigned { id, contract }) => {
                contracts_clone.lock().unwrap().insert(*id, format!("{contract}"));
            }
            FpssEvent::Data(FpssData::Trade { contract_id, price, size, ms_of_day, price_type, .. }) => {
                let name = contracts_clone.lock().unwrap().get(contract_id).cloned().unwrap_or_default();
                let price_f = tdbe::Price::new(*price, *price_type).to_f64();
                let h = ms_of_day / 3_600_000;
                let m = (ms_of_day % 3_600_000) / 60_000;
                let s = (ms_of_day % 60_000) / 1_000;
                println!("{:02}:{:02}:{:02}       {:<8}  {:>8.2}  {:>8}",
                    h, m, s, name, price_f, size);
            }
            _ => {}
        }
    })?;

    let symbols = vec![${symsListRust()}];
    for sym in &symbols {
        tdx.subscribe_quotes(&Contract::stock(sym))?;
    }

    std::thread::park();
    Ok(())
}`

    case 'option_flow_scanner': return `${rustHeader()}
use thetadatadx::fpss::{FpssEvent, FpssData, FpssControl};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[tokio::main]
async fn main() -> Result<(), thetadatadx::Error> {
    let creds = Credentials::from_file("creds.txt")?;
    // Or inline: let creds = Credentials::new("user@example.com", "your-password");
    let tdx = ThetaDataDx::connect(&creds, DirectConfig::production()).await?;

    let min_size: u32 = ${minSize()};
    let contracts: Arc<Mutex<HashMap<i32, String>>> = Arc::new(Mutex::new(HashMap::new()));
    let contracts_clone = contracts.clone();

    println!("Option Flow Scanner — alerting on size >= {}", min_size);
    println!("{:<35}  {:>6}  {:>8}  {:>12}", "Contract", "Size", "Price", "Premium");
    println!("{}", "-".repeat(70));

    tdx.start_streaming(move |event: &FpssEvent| {
        match event {
            FpssEvent::Control(FpssControl::ContractAssigned { id, contract }) => {
                contracts_clone.lock().unwrap().insert(*id, format!("{contract}"));
            }
            FpssEvent::Data(FpssData::Trade { contract_id, price, size, price_type, .. }) => {
                if *size >= min_size {
                    let name = contracts_clone.lock().unwrap().get(contract_id).cloned().unwrap_or_default();
                    let price_f = tdbe::Price::new(*price, *price_type).to_f64();
                    let premium = price_f * *size as f64 * 100.0;
                    println!("{:<35}  {:>6}  {:>8.2}  \${:>11,.0}",
                        name, size, price_f, premium);
                }
            }
            _ => {}
        }
    })?;

    tdx.subscribe_full_trades(tdbe::SecType::Option)?;

    std::thread::park();
    Ok(())
}`

    case 'live_option_chain': return `${rustHeader()}
use thetadatadx::fpss::{FpssEvent, FpssData, FpssControl};
use thetadatadx::fpss::protocol::Contract;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[tokio::main]
async fn main() -> Result<(), thetadatadx::Error> {
    let creds = Credentials::from_file("creds.txt")?;
    // Or inline: let creds = Credentials::new("user@example.com", "your-password");
    let tdx = ThetaDataDx::connect(&creds, DirectConfig::production()).await?;

    let symbol = "${sym()}";
    let exp    = "${exp()}";

    let contracts: Arc<Mutex<HashMap<i32, String>>> = Arc::new(Mutex::new(HashMap::new()));
    let contracts_clone = contracts.clone();

    tdx.start_streaming(move |event: &FpssEvent| {
        match event {
            FpssEvent::Control(FpssControl::ContractAssigned { id, contract }) => {
                contracts_clone.lock().unwrap().insert(*id, format!("{contract}"));
            }
            FpssEvent::Data(FpssData::Quote { contract_id, bid, ask, price_type, .. }) => {
                let name = contracts_clone.lock().unwrap().get(contract_id).cloned().unwrap_or_default();
                let bid_f = tdbe::Price::new(*bid, *price_type).to_f64();
                let ask_f = tdbe::Price::new(*ask, *price_type).to_f64();
                let spread = ask_f - bid_f;
                let mid = (bid_f + ask_f) / 2.0;
                println!("{:<30}  {:>8.2}  {:>8.2}  {:>8.4}  {:>8.2}",
                    name, bid_f, ask_f, spread, mid);
            }
            _ => {}
        }
    })?;

    // Get all strikes, subscribe to option quotes for each contract
    let strikes = tdx.option_list_strikes(symbol, exp).await?;
    for strike in &strikes {
        // Subscribe to both calls and puts
        tdx.subscribe_quotes(&Contract::option(symbol, exp.parse().unwrap_or(0), true, strike.parse().unwrap_or(0)))?;
        tdx.subscribe_quotes(&Contract::option(symbol, exp.parse().unwrap_or(0), false, strike.parse().unwrap_or(0)))?;
    }
    println!("Live chain: {} {}  ({} contracts)", symbol, exp, strikes.len() * 2);

    std::thread::park();
    Ok(())
}`

    default:
      return '// Recipe not yet implemented'
  }
}
</script>

<template>
  <div class="qb-root">
    <!-- ── Header ───────────────────────────────────────────────────── -->
    <div class="qb-header">
      <div class="qb-header-title">
        <span class="qb-header-icon" v-html="SVG_ICONS.tool"></span>
        <div>
          <h2>Code Recipe Builder</h2>
          <p>Pick a use case and get working code in seconds.</p>
        </div>
      </div>
      <!-- Step breadcrumbs -->
      <div class="qb-steps">
        <button
          v-for="(label, idx) in ['Recipe', 'Parameters', 'Language', 'Code']"
          :key="idx"
          class="qb-step-btn"
          :class="{
            'qb-step-active':    step === idx + 1,
            'qb-step-done':      step > idx + 1,
            'qb-step-clickable': idx + 1 < step,
          }"
          :disabled="idx + 1 >= step"
          @click="goStep((idx + 1) as 1|2|3|4)"
        >
          <span class="qb-step-num">{{ idx + 1 }}</span>
          {{ label }}
        </button>
      </div>
    </div>

    <!-- ── Step 1: Choose Recipe ─────────────────────────────────────── -->
    <div v-if="step === 1" class="qb-section">
      <div class="qb-section-label">Historical Data</div>
      <div class="qb-recipe-grid">
        <button
          v-for="recipe in historicalRecipes"
          :key="recipe.id"
          class="qb-recipe-card"
          @click="pickRecipe(recipe)"
        >
          <span class="qb-recipe-icon" v-html="recipe.icon"></span>
          <span class="qb-recipe-title">{{ recipe.title }}</span>
          <span class="qb-recipe-desc">{{ recipe.description }}</span>
        </button>
      </div>

      <div class="qb-section-label qb-mt">Real-Time Streaming</div>
      <div class="qb-recipe-grid">
        <button
          v-for="recipe in realtimeRecipes"
          :key="recipe.id"
          class="qb-recipe-card qb-recipe-card--rt"
          @click="pickRecipe(recipe)"
        >
          <span class="qb-recipe-icon" v-html="recipe.icon"></span>
          <span class="qb-recipe-title">{{ recipe.title }}</span>
          <span class="qb-recipe-desc">{{ recipe.description }}</span>
        </button>
      </div>
    </div>

    <!-- ── Step 2: Parameters ────────────────────────────────────────── -->
    <div v-if="step === 2" class="qb-section">
      <div class="qb-recipe-badge">
        <span v-html="currentRecipe?.icon"></span>
        <strong>{{ currentRecipe?.title }}</strong>
      </div>

      <div class="qb-param-grid">

        <!-- Symbol (single) -->
        <div v-if="needsSymbol" class="qb-param-group qb-param-full">
          <label class="qb-label">Symbol</label>
          <div class="qb-autocomplete-wrap" ref="autocompleteRef">
            <input
              class="qb-input"
              type="text"
              :value="symbolInput"
              placeholder="e.g. AAPL"
              autocomplete="off"
              @input="e => { symbolInput = (e.target as HTMLInputElement).value; onSymbolInput() }"
              @focus="onSymbolFocus"
              @blur="onSymbolBlur"
              @keydown="onSymbolKeydown"
            />
            <div v-if="autocompleteVisible && filteredSuggestions.length > 0" class="qb-dropdown">
              <template v-for="[cat, items] in symbolsByCategory" :key="cat">
                <div class="qb-dropdown-cat">{{ cat }}</div>
                <button
                  v-for="(s, i) in items"
                  :key="s.symbol"
                  class="qb-dropdown-item"
                  :class="{ 'qb-dropdown-item--selected': acSelectedIdx === filteredSuggestions.indexOf(s) }"
                  @mousedown.prevent="selectSymbol(s)"
                >
                  <span class="qb-dropdown-sym">{{ s.symbol }}</span>
                  <span class="qb-dropdown-name">{{ s.name }}</span>
                </button>
              </template>
            </div>
          </div>
        </div>

        <!-- Symbols list -->
        <div v-if="needsSymbolsList" class="qb-param-group qb-param-full">
          <label class="qb-label">Symbols <span class="qb-hint">comma or space separated</span></label>
          <input
            class="qb-input"
            type="text"
            v-model="params.symbolsRaw"
            placeholder="AAPL, MSFT, SPY"
          />
          <div class="qb-helper">Parsed: {{ symbolsList.join(' · ') }}</div>
        </div>

        <!-- Date range -->
        <template v-if="needsDateRange">
          <div class="qb-param-group qb-param-full">
            <label class="qb-label">Date Range</label>
            <div class="qb-chips">
              <button
                v-for="p in DATE_PRESETS.filter(p => p.isRange !== false)"
                :key="p.label"
                class="qb-chip"
                :class="{ 'qb-chip--active': activeDatePreset === p.label }"
                @click="selectDatePreset(p)"
              >{{ p.label }}</button>
            </div>
          </div>
          <div class="qb-param-group">
            <label class="qb-label">Start Date <span class="qb-hint">YYYYMMDD</span></label>
            <input class="qb-input" type="text" v-model="params.start_date" maxlength="8" placeholder="20240101" />
          </div>
          <div class="qb-param-group">
            <label class="qb-label">End Date <span class="qb-hint">YYYYMMDD</span></label>
            <input class="qb-input" type="text" v-model="params.end_date" maxlength="8" placeholder="20241231" />
          </div>
        </template>

        <!-- Single date -->
        <template v-if="needsSingleDate">
          <div class="qb-param-group qb-param-full">
            <label class="qb-label">Date</label>
            <div class="qb-chips">
              <button
                v-for="p in DATE_PRESETS.filter(p => !p.isRange)"
                :key="p.label"
                class="qb-chip"
                :class="{ 'qb-chip--active': activeSinglePreset === p.label }"
                @click="selectDatePreset(p)"
              >{{ p.label }}</button>
            </div>
            <input class="qb-input qb-mt-sm" type="text" v-model="params.date" maxlength="8" placeholder="20240101" />
          </div>
        </template>

        <!-- Interval -->
        <div v-if="needsInterval" class="qb-param-group qb-param-full">
          <label class="qb-label">Interval</label>
          <div class="qb-chips">
            <button
              v-for="opt in INTERVAL_OPTIONS"
              :key="opt.value"
              class="qb-chip"
              :class="{ 'qb-chip--active': params.interval === opt.value }"
              @click="params.interval = opt.value"
            >{{ opt.label }}</button>
          </div>
        </div>

        <!-- Expiration -->
        <div v-if="needsExpiration" class="qb-param-group">
          <label class="qb-label">Expiration <span class="qb-hint">YYYYMMDD</span></label>
          <input class="qb-input" type="text" v-model="params.expiration" maxlength="8" placeholder="20250620" />
          <div class="qb-helper">Use <code>option_list_expirations()</code> to get available dates</div>
        </div>

        <!-- Strike -->
        <div v-if="needsStrike" class="qb-param-group">
          <label class="qb-label">Strike <span class="qb-hint">scaled integer</span></label>
          <input class="qb-input" type="text" v-model="params.strike" placeholder="500000" />
          <div class="qb-helper">500000 = $500.00 &mdash; use <code>option_list_strikes()</code></div>
        </div>

        <!-- Right (Call/Put) -->
        <div v-if="needsRight" class="qb-param-group">
          <label class="qb-label">Right</label>
          <div class="qb-toggle-group">
            <button
              class="qb-toggle-btn"
              :class="{ 'qb-toggle-btn--call': params.right === 'C' }"
              @click="params.right = 'C'"
            >Call</button>
            <button
              class="qb-toggle-btn"
              :class="{ 'qb-toggle-btn--put': params.right === 'P' }"
              @click="params.right = 'P'"
            >Put</button>
          </div>
        </div>

        <!-- Year -->
        <div v-if="needsYear" class="qb-param-group">
          <label class="qb-label">Year</label>
          <input class="qb-input" type="number" v-model="params.year" min="2000" max="2030" />
        </div>

        <!-- Min size -->
        <div v-if="needsMinSize" class="qb-param-group">
          <label class="qb-label">Min Contract Size <span class="qb-hint">alert threshold</span></label>
          <input class="qb-input" type="number" v-model="params.min_size" min="1" />
          <div class="qb-helper">Flag trades with size &ge; this value as "unusual"</div>
        </div>

      </div><!-- end param-grid -->

      <div class="qb-actions">
        <button class="qb-btn qb-btn--secondary" @click="startOver">Start Over</button>
        <button class="qb-btn qb-btn--primary" @click="toStep3">Choose Language &rarr;</button>
      </div>
    </div>

    <!-- ── Step 3: Language ───────────────────────────────────────────── -->
    <div v-if="step === 3" class="qb-section">
      <div class="qb-recipe-badge">
        <span v-html="currentRecipe?.icon"></span>
        <strong>{{ currentRecipe?.title }}</strong>
      </div>

      <p class="qb-lang-prompt">Choose your language:</p>
      <div class="qb-lang-grid">
        <button
          class="qb-lang-card"
          :class="{ 'qb-lang-card--selected': language === 'python' }"
          @click="toStep4('python')"
        >
          <span class="qb-lang-name">Python</span>
          <span class="qb-lang-desc">thetadatadx -- pandas-friendly</span>
        </button>
        <button
          class="qb-lang-card"
          :class="{ 'qb-lang-card--selected': language === 'rust' }"
          @click="toStep4('rust')"
        >
          <span class="qb-lang-name">Rust</span>
          <span class="qb-lang-desc">thetadatadx -- tokio async</span>
        </button>
      </div>

      <div class="qb-actions">
        <button class="qb-btn qb-btn--secondary" @click="startOver">Start Over</button>
      </div>
    </div>

    <!-- ── Step 4: Generated Code ─────────────────────────────────────── -->
    <div v-if="step === 4" class="qb-section">
      <div class="qb-code-header">
        <div class="qb-recipe-badge">
          <span v-html="currentRecipe?.icon"></span>
          <strong>{{ currentRecipe?.title }}</strong>
          <span class="qb-lang-pill" :class="`qb-lang-pill--${language}`">
            {{ language === 'python' ? 'Python' : 'Rust' }}
          </span>
        </div>
        <button class="qb-copy-btn" :class="{ 'qb-copy-btn--done': copied }" @click="copyCode">
          <span v-if="!copied">Copy</span>
          <span v-else>Copied!</span>
        </button>
      </div>

      <div class="qb-code-wrap">
        <pre class="qb-code" v-html="highlightedCode"></pre>
      </div>

      <div class="qb-actions">
        <button class="qb-btn qb-btn--secondary" @click="startOver">Start Over</button>
        <button class="qb-btn qb-btn--ghost" @click="goStep(2)">Edit Parameters</button>
        <button class="qb-btn qb-btn--ghost" @click="goStep(3)">Switch Language</button>
      </div>
    </div>
  </div>
</template>

<style scoped>
/* ─── Root ─────────────────────────────────────────────────────────────────── */
.qb-root {
  font-family: var(--vp-font-family-base, -apple-system, BlinkMacSystemFont, sans-serif);
  color: var(--vp-c-text-1);
  background: var(--vp-c-bg);
  border: 1px solid var(--vp-c-divider);
  border-radius: 12px;
  overflow: hidden;
}

/* ─── Header ────────────────────────────────────────────────────────────────── */
.qb-header {
  display: flex;
  flex-wrap: wrap;
  gap: 16px;
  align-items: center;
  justify-content: space-between;
  padding: 20px 24px 16px;
  background: var(--vp-c-bg-soft);
  border-bottom: 1px solid var(--vp-c-divider);
}

.qb-header-title {
  display: flex;
  align-items: center;
  gap: 12px;
}

.qb-header-icon {
  display: inline-flex;
  align-items: center;
  width: 28px;
  height: 28px;
  color: var(--vp-c-brand-1);
}

.qb-header-icon :deep(svg) {
  width: 28px;
  height: 28px;
}

.qb-header-title h2 {
  margin: 0;
  font-size: 18px;
  font-weight: 700;
  line-height: 1.3;
  border: none;
}

.qb-header-title p {
  margin: 2px 0 0;
  font-size: 13px;
  color: var(--vp-c-text-2);
  line-height: 1.4;
}

/* ─── Step breadcrumbs ─────────────────────────────────────────────────────── */
.qb-steps {
  display: flex;
  gap: 4px;
  flex-wrap: wrap;
}

.qb-step-btn {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 6px 12px;
  background: transparent;
  border: 1px solid var(--vp-c-divider);
  border-radius: 20px;
  font-size: 12px;
  font-weight: 500;
  color: var(--vp-c-text-2);
  cursor: default;
  transition: all 0.15s;
  white-space: nowrap;
}

.qb-step-btn.qb-step-clickable {
  cursor: pointer;
  border-color: var(--vp-c-brand-1);
  color: var(--vp-c-brand-1);
}

.qb-step-btn.qb-step-clickable:hover {
  background: var(--vp-c-brand-soft);
}

.qb-step-btn.qb-step-active {
  background: var(--vp-c-brand-1);
  border-color: var(--vp-c-brand-1);
  color: #fff;
  cursor: default;
}

.qb-step-btn.qb-step-done {
  border-color: var(--vp-c-brand-1);
  color: var(--vp-c-brand-1);
}

.qb-step-num {
  width: 18px;
  height: 18px;
  border-radius: 50%;
  background: rgba(255,255,255,0.25);
  display: inline-flex;
  align-items: center;
  justify-content: center;
  font-size: 10px;
  font-weight: 700;
  flex-shrink: 0;
}

.qb-step-active .qb-step-num { background: rgba(255,255,255,0.3); }

/* ─── Section ───────────────────────────────────────────────────────────────── */
.qb-section {
  padding: 24px;
}

.qb-section-label {
  font-size: 11px;
  font-weight: 700;
  letter-spacing: 0.07em;
  text-transform: uppercase;
  color: var(--vp-c-text-2);
  margin-bottom: 12px;
}

.qb-mt { margin-top: 28px; }

/* ─── Recipe cards ──────────────────────────────────────────────────────────── */
.qb-recipe-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
  gap: 10px;
}

.qb-recipe-card {
  display: flex;
  flex-direction: column;
  gap: 5px;
  padding: 14px 16px;
  background: var(--vp-c-bg);
  border: 1px solid var(--vp-c-divider);
  border-radius: 8px;
  text-align: left;
  cursor: pointer;
  transition: border-color 0.15s, box-shadow 0.15s, background 0.15s;
  position: relative;
  overflow: hidden;
}

.qb-recipe-card::before {
  content: '';
  position: absolute;
  inset: 0;
  background: var(--vp-c-brand-soft);
  opacity: 0;
  transition: opacity 0.15s;
}

.qb-recipe-card:hover {
  border-color: var(--vp-c-brand-1);
  box-shadow: 0 0 0 2px var(--vp-c-brand-soft);
}

.qb-recipe-card:hover::before { opacity: 1; }

.qb-recipe-card--rt {
  border-left: 2px solid var(--vp-c-brand-3);
}

.qb-recipe-card--rt:hover {
  border-color: var(--vp-c-brand-2);
}

.qb-recipe-icon {
  display: inline-flex;
  align-items: center;
  width: 24px;
  height: 24px;
  position: relative;
  z-index: 1;
  color: var(--vp-c-brand-1);
}

.qb-recipe-icon :deep(svg) {
  width: 22px;
  height: 22px;
}

.qb-recipe-title {
  font-size: 13.5px;
  font-weight: 600;
  color: var(--vp-c-text-1);
  line-height: 1.3;
  position: relative;
  z-index: 1;
}

.qb-recipe-desc {
  font-size: 12px;
  color: var(--vp-c-text-2);
  line-height: 1.45;
  position: relative;
  z-index: 1;
}

/* ─── Recipe badge (step 2/3/4 header) ────────────────────────────────────── */
.qb-recipe-badge {
  display: inline-flex;
  align-items: center;
  gap: 8px;
  padding: 6px 12px;
  background: var(--vp-c-bg-soft);
  border: 1px solid var(--vp-c-divider);
  border-radius: 6px;
  font-size: 13px;
  margin-bottom: 20px;
  flex-wrap: wrap;
}

/* ─── Parameter grid ────────────────────────────────────────────────────────── */
.qb-param-grid {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 18px 24px;
}

@media (max-width: 600px) {
  .qb-param-grid { grid-template-columns: 1fr; }
}

.qb-param-group { display: flex; flex-direction: column; gap: 6px; }
.qb-param-full  { grid-column: 1 / -1; }

/* ─── Labels / inputs ───────────────────────────────────────────────────────── */
.qb-label {
  font-size: 12px;
  font-weight: 600;
  color: var(--vp-c-text-1);
  display: flex;
  align-items: center;
  gap: 6px;
}

.qb-hint {
  font-size: 11px;
  font-weight: 400;
  color: var(--vp-c-text-3);
}

.qb-input {
  width: 100%;
  padding: 8px 10px;
  font-size: 13px;
  font-family: var(--vp-font-family-base);
  color: var(--vp-c-text-1);
  background: var(--vp-c-bg);
  border: 1px solid var(--vp-c-divider);
  border-radius: 6px;
  outline: none;
  transition: border-color 0.15s, box-shadow 0.15s;
  box-sizing: border-box;
}

.qb-input:focus {
  border-color: var(--vp-c-brand-1);
  box-shadow: 0 0 0 2px var(--vp-c-brand-soft);
}

.qb-helper {
  font-size: 11.5px;
  color: var(--vp-c-text-3);
  line-height: 1.5;
}

.qb-helper code {
  font-family: var(--vp-font-family-mono, monospace);
  font-size: 11px;
  background: var(--vp-c-bg-soft);
  padding: 1px 4px;
  border-radius: 3px;
  color: var(--vp-c-brand-1);
}

.qb-mt-sm { margin-top: 6px; }

/* ─── Chips ─────────────────────────────────────────────────────────────────── */
.qb-chips {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
}

.qb-chip {
  padding: 4px 12px;
  border: 1px solid var(--vp-c-divider);
  border-radius: 20px;
  font-size: 12px;
  font-weight: 500;
  color: var(--vp-c-text-2);
  background: var(--vp-c-bg);
  cursor: pointer;
  transition: all 0.12s;
  white-space: nowrap;
}

.qb-chip:hover {
  border-color: var(--vp-c-brand-1);
  color: var(--vp-c-brand-1);
}

.qb-chip--active {
  background: var(--vp-c-brand-1);
  border-color: var(--vp-c-brand-1);
  color: #fff;
}

/* ─── Toggle (Call/Put) ─────────────────────────────────────────────────────── */
.qb-toggle-group {
  display: flex;
  gap: 0;
  border-radius: 6px;
  overflow: hidden;
  border: 1px solid var(--vp-c-divider);
  width: fit-content;
}

.qb-toggle-btn {
  padding: 7px 20px;
  font-size: 13px;
  font-weight: 600;
  color: var(--vp-c-text-2);
  background: var(--vp-c-bg);
  border: none;
  cursor: pointer;
  transition: all 0.12s;
}

.qb-toggle-btn:first-child { border-right: 1px solid var(--vp-c-divider); }

.qb-toggle-btn--call {
  background: #16a34a;
  color: #fff;
}

.qb-toggle-btn--put {
  background: #dc2626;
  color: #fff;
}

/* ─── Autocomplete dropdown ─────────────────────────────────────────────────── */
.qb-autocomplete-wrap { position: relative; }

.qb-dropdown {
  position: absolute;
  top: calc(100% + 3px);
  left: 0;
  right: 0;
  z-index: 200;
  background: var(--vp-c-bg);
  border: 1px solid var(--vp-c-divider);
  border-radius: 8px;
  box-shadow: 0 8px 24px rgba(0,0,0,0.12);
  max-height: 280px;
  overflow-y: auto;
  padding: 4px 0;
}

.qb-dropdown-cat {
  padding: 6px 12px 3px;
  font-size: 10px;
  font-weight: 700;
  letter-spacing: 0.06em;
  text-transform: uppercase;
  color: var(--vp-c-text-3);
}

.qb-dropdown-item {
  display: flex;
  align-items: center;
  gap: 10px;
  width: 100%;
  padding: 7px 12px;
  border: none;
  background: transparent;
  cursor: pointer;
  text-align: left;
  transition: background 0.1s;
}

.qb-dropdown-item:hover,
.qb-dropdown-item--selected {
  background: var(--vp-c-bg-soft);
}

.qb-dropdown-sym {
  font-size: 13px;
  font-weight: 700;
  color: var(--vp-c-text-1);
  font-family: var(--vp-font-family-mono, monospace);
  min-width: 52px;
}

.qb-dropdown-name {
  font-size: 12px;
  color: var(--vp-c-text-2);
}

/* ─── Language cards ────────────────────────────────────────────────────────── */
.qb-lang-prompt {
  font-size: 15px;
  color: var(--vp-c-text-2);
  margin-bottom: 16px;
}

.qb-lang-grid {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 14px;
  max-width: 480px;
}

@media (max-width: 500px) {
  .qb-lang-grid { grid-template-columns: 1fr; }
}

.qb-lang-card {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 8px;
  padding: 24px 20px;
  border: 2px solid var(--vp-c-divider);
  border-radius: 10px;
  background: var(--vp-c-bg);
  cursor: pointer;
  transition: all 0.15s;
}

.qb-lang-card:hover {
  border-color: var(--vp-c-brand-1);
  box-shadow: 0 0 0 3px var(--vp-c-brand-soft);
}

.qb-lang-card--selected {
  border-color: var(--vp-c-brand-1);
  background: var(--vp-c-brand-soft);
}

.qb-lang-name { font-size: 18px; font-weight: 700; }
.qb-lang-desc { font-size: 12px; color: var(--vp-c-text-2); text-align: center; }

/* ─── Code output ───────────────────────────────────────────────────────────── */
.qb-code-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  flex-wrap: wrap;
  gap: 10px;
  margin-bottom: 12px;
}

.qb-lang-pill {
  font-size: 11px;
  font-weight: 600;
  padding: 2px 8px;
  border-radius: 10px;
  border: 1px solid var(--vp-c-divider);
  background: var(--vp-c-bg-soft);
}

.qb-lang-pill--python { color: #2e7d32; border-color: #a5d6a7; background: #f1f8e9; }
.dark .qb-lang-pill--python { color: #81c784; border-color: #388e3c; background: rgba(56,142,60,0.15); }
.qb-lang-pill--rust { color: #bf360c; border-color: #ffab91; background: #fbe9e7; }
.dark .qb-lang-pill--rust { color: #ff7043; border-color: #bf360c; background: rgba(191,54,12,0.15); }

.qb-copy-btn {
  padding: 6px 16px;
  font-size: 12px;
  font-weight: 600;
  color: var(--vp-c-brand-1);
  background: transparent;
  border: 1px solid var(--vp-c-brand-1);
  border-radius: 6px;
  cursor: pointer;
  transition: all 0.15s;
  white-space: nowrap;
}

.qb-copy-btn:hover { background: var(--vp-c-brand-soft); }
.qb-copy-btn--done { background: #16a34a; border-color: #16a34a; color: #fff; }

.qb-code-wrap {
  border-radius: 8px;
  border: 1px solid var(--vp-c-divider);
  background: #1a1b26;
  overflow: auto;
  max-height: 640px;
}

.qb-code {
  margin: 0;
  padding: 20px 22px;
  font-family: var(--vp-font-family-mono, 'JetBrains Mono', monospace);
  font-size: 13px;
  line-height: 1.72;
  color: #cdd6f4;
  white-space: pre;
  tab-size: 4;
}

/* Syntax tokens */
.qb-code :deep(.hl-keyword)   { color: #cba6f7; }
.qb-code :deep(.hl-string)    { color: #a6e3a1; }
.qb-code :deep(.hl-comment)   { color: #6c7086; font-style: italic; }
.qb-code :deep(.hl-number)    { color: #fab387; }
.qb-code :deep(.hl-func)      { color: #89b4fa; }
.qb-code :deep(.hl-builtin)   { color: #94e2d5; }
.qb-code :deep(.hl-type)      { color: #f38ba8; }
.qb-code :deep(.hl-macro)     { color: #f9e2af; }
.qb-code :deep(.hl-decorator) { color: #f5c2e7; }

/* ─── Action buttons ────────────────────────────────────────────────────────── */
.qb-actions {
  display: flex;
  gap: 10px;
  flex-wrap: wrap;
  margin-top: 24px;
}

.qb-btn {
  padding: 8px 18px;
  border-radius: 6px;
  font-size: 13px;
  font-weight: 600;
  cursor: pointer;
  transition: all 0.15s;
  border: 1px solid transparent;
  white-space: nowrap;
}

.qb-btn--primary {
  background: var(--vp-c-brand-1);
  color: #fff;
  border-color: var(--vp-c-brand-1);
}

.qb-btn--primary:hover {
  background: var(--vp-c-brand-2);
  border-color: var(--vp-c-brand-2);
}

.qb-btn--secondary {
  background: transparent;
  color: var(--vp-c-text-2);
  border-color: var(--vp-c-divider);
}

.qb-btn--secondary:hover {
  border-color: var(--vp-c-text-2);
  color: var(--vp-c-text-1);
}

.qb-btn--ghost {
  background: transparent;
  color: var(--vp-c-brand-1);
  border-color: var(--vp-c-brand-soft);
}

.qb-btn--ghost:hover {
  background: var(--vp-c-brand-soft);
}
</style>
