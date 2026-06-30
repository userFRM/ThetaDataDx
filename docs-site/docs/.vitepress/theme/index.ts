import DefaultTheme from 'vitepress/theme'
import type { Theme } from 'vitepress'
import SdkTabs from './components/SdkTabs.vue'
import TierBadge from './components/TierBadge.vue'
import RequestBuilder from './components/RequestBuilder.vue'
import './style.css'

export default {
  extends: DefaultTheme,
  enhanceApp({ app }) {
    app.component('SdkTabs', SdkTabs)
    app.component('TierBadge', TierBadge)
    app.component('RequestBuilder', RequestBuilder)
  },
} satisfies Theme
