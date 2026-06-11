import DefaultTheme from 'vitepress/theme'
import type { Theme } from 'vitepress'
import QueryBuilder from './components/QueryBuilder.vue'
import SdkTabs from './components/SdkTabs.vue'
import TierBadge from './components/TierBadge.vue'
import './style.css'

export default {
  extends: DefaultTheme,
  enhanceApp({ app }) {
    app.component('QueryBuilder', QueryBuilder)
    app.component('SdkTabs', SdkTabs)
    app.component('TierBadge', TierBadge)
  },
} satisfies Theme
