<script setup lang="ts">
// Language tab carousel for reference pages.
//
// Five fixed tabs — Rust, Python, TypeScript, C++, HTTP — fed by named
// slots. The selection is global and persistent: every SdkTabs
// instance shares one reactive ref, and the choice is stored in
// localStorage so it follows the reader across pages and visits.
// Pages that lack a slot for the selected language show a standard
// availability note instead of an empty panel.
import { computed, onMounted, ref, useSlots } from 'vue'
import { tabIcons } from './sdkTabIcons'

interface TabDef {
  id: string
  label: string
}

const TABS: TabDef[] = [
  { id: 'rust', label: 'Rust' },
  { id: 'python', label: 'Python' },
  { id: 'typescript', label: 'TypeScript' },
  { id: 'cpp', label: 'C++' },
  { id: 'http', label: 'HTTP' },
]

const STORAGE_KEY = 'thetadatadx-docs-lang'
const DEFAULT_TAB = 'rust'

// Module-scoped shared state: one selection across every instance.
// (A module-level ref is created once per app and shared by all
// component instances that import it.)
const sharedActive = ref(DEFAULT_TAB)
let storageSynced = false

const slots = useSlots()
const present = computed(() => TABS.filter((t) => !!slots[t.id]))
const active = sharedActive

function select(id: string) {
  sharedActive.value = id
  if (typeof localStorage !== 'undefined') {
    localStorage.setItem(STORAGE_KEY, id)
  }
}

onMounted(() => {
  if (storageSynced) return
  storageSynced = true
  const stored = typeof localStorage !== 'undefined' ? localStorage.getItem(STORAGE_KEY) : null
  if (stored && TABS.some((t) => t.id === stored)) {
    sharedActive.value = stored
  }
})

const activeAvailable = computed(() => !!slots[active.value])
const availableLabels = computed(() => present.value.map((t) => t.label).join(', '))
const activeLabel = computed(
  () => TABS.find((t) => t.id === active.value)?.label ?? active.value,
)
</script>

<template>
  <div class="sdk-tabs">
    <div class="sdk-tabs-bar" role="tablist" aria-label="Language">
      <button
        v-for="tab in TABS"
        :key="tab.id"
        type="button"
        role="tab"
        class="sdk-tab"
        :class="{ active: active === tab.id }"
        :aria-selected="active === tab.id"
        @click="select(tab.id)"
      >
        <svg class="sdk-tab-icon" viewBox="0 0 24 24" aria-hidden="true" v-html="tabIcons[tab.id]" />
        <span>{{ tab.label }}</span>
      </button>
    </div>
    <div class="sdk-tabs-panels">
      <template v-for="tab in TABS" :key="tab.id">
        <div v-if="slots[tab.id]" v-show="active === tab.id" class="sdk-tab-panel" role="tabpanel">
          <slot :name="tab.id" />
        </div>
      </template>
      <div v-if="!activeAvailable" class="sdk-tab-panel sdk-tab-missing" role="tabpanel">
        <p>
          Not available in {{ activeLabel }}. This page applies to: {{ availableLabels }}.
        </p>
      </div>
    </div>
  </div>
</template>

<style scoped>
.sdk-tabs {
  margin: 20px 0;
  border: 1px solid var(--vp-c-divider);
  border-radius: 8px;
  overflow: hidden;
}

.sdk-tabs-bar {
  display: flex;
  flex-wrap: wrap;
  background: var(--vp-c-bg-soft);
  border-bottom: 1px solid var(--vp-c-divider);
}

.sdk-tab {
  display: inline-flex;
  align-items: center;
  gap: 7px;
  padding: 9px 14px;
  font-size: 13px;
  font-weight: 500;
  color: var(--vp-c-text-2);
  border-bottom: 2px solid transparent;
  margin-bottom: -1px;
  transition: color 0.15s ease, border-color 0.15s ease;
  cursor: pointer;
}

.sdk-tab:hover {
  color: var(--vp-c-text-1);
}

.sdk-tab.active {
  color: var(--vp-c-brand-1);
  border-bottom-color: var(--vp-c-brand-1);
}

.sdk-tab-icon {
  width: 15px;
  height: 15px;
  flex: none;
}

.sdk-tab-panel {
  padding: 4px 18px 14px;
}

.sdk-tab-panel :deep(div[class*='language-']) {
  margin: 12px 0;
}

.sdk-tab-missing p {
  color: var(--vp-c-text-2);
  font-size: 14px;
}
</style>
