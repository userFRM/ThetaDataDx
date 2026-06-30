<script setup lang="ts">
const props = defineProps<{
  tier: 'free' | 'value' | 'standard' | 'professional'
}>()

const tiers = ['free', 'value', 'standard', 'professional'] as const

const tierLabels: Record<string, string> = {
  free: 'Free',
  value: 'Value',
  standard: 'Standard',
  professional: 'Pro',
}

function isActive(t: string): boolean {
  const minIndex = tiers.indexOf(props.tier as any)
  const thisIndex = tiers.indexOf(t as any)
  return thisIndex >= minIndex
}
</script>

<template>
  <div class="tier-badges">
    <span
      v-for="t in tiers"
      :key="t"
      :class="['tier-badge', `tier-${t}`, isActive(t) ? 'on' : 'off']"
    >{{ tierLabels[t] }}</span>
  </div>
</template>

<style scoped>
.tier-badges {
  display: inline-flex;
  margin: 6px 0 16px;
  border-radius: 6px;
  overflow: hidden;
  border: 1px solid var(--vp-c-divider);
}

.tier-badge {
  padding: 3px 11px;
  font-size: 10.5px;
  font-weight: 600;
  letter-spacing: 0.05em;
  line-height: 1.5;
  color: #fff;
  text-shadow: 0 1px 1px rgba(0, 0, 0, 0.14);
}

/* distinct per-tier hues along one cohesive cool ramp */
.tier-badge.tier-free.on { background: #14b8a6; }        /* teal */
.tier-badge.tier-value.on { background: #3b82f6; }       /* blue */
.tier-badge.tier-standard.on { background: #6366f1; }    /* indigo */
.tier-badge.tier-professional.on { background: #8b5cf6; } /* violet */

.tier-badge.off {
  background: var(--vp-c-bg-soft);
  color: var(--vp-c-text-3);
  text-shadow: none;
  font-weight: 500;
}
</style>
