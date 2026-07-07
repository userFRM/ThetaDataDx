import { defineConfig } from 'vitepress'
import referenceSidebar from './generated/reference-sidebar.json'
import streamingSidebar from './generated/streaming-sidebar.json'

export default defineConfig({
  title: 'ThetaDataDx',
  description: 'SDK for ThetaData market data — Rust, Python, TypeScript, C++',
  base: '/ThetaDataDx/',
  cleanUrls: true,

  // The local-search index and Vue runtime chunks legitimately exceed
  // Vite's 500 kB warning threshold; raise the limit so build output
  // stays free of non-actionable warnings while real regressions
  // (a new dependency inflating the bundle) still trip it.
  vite: {
    build: {
      chunkSizeWarningLimit: 1500,
      // esbuild 0.28 no longer downlevels destructuring to the default
      // browser baseline and errors instead. The docs site ships modern
      // ESM and Vue 3 runtime that already require an evergreen browser,
      // so target the current engines directly.
      target: 'esnext',
    },
    optimizeDeps: {
      esbuildOptions: { target: 'esnext' },
    },
  },

  head: [
    ['link', { rel: 'icon', type: 'image/svg+xml', href: '/icons/chart.svg' }],
    ['meta', { name: 'theme-color', content: '#3b82f6' }],
    ['meta', { property: 'og:type', content: 'website' }],
    ['meta', { property: 'og:title', content: 'ThetaDataDx' }],
    [
      'meta',
      {
        property: 'og:description',
        content: 'SDK for ThetaData market data — Rust, Python, TypeScript, C++',
      },
    ],
  ],

  themeConfig: {
    siteTitle: 'ThetaDataDx',

    nav: [
      { text: 'Articles', link: '/articles/getting-started' },
      { text: 'API Reference', link: '/reference/' },
      { text: 'Streaming', link: '/streaming/' },
      { text: 'MCP', link: '/mcp' },
      { text: 'Server', link: '/server/' },
      { text: 'ThetaData Docs', link: 'https://docs.thetadata.us/' },
    ],

    sidebar: [
      {
        text: 'Articles',
        collapsed: false,
        items: [
          { text: 'Getting Started', link: '/articles/getting-started' },
          { text: 'Terminology', link: '/articles/terminology' },
          { text: 'Subscriptions', link: '/articles/subscriptions' },
          { text: 'Symbology & Contract Identity', link: '/articles/symbology' },
          { text: 'Option Greeks', link: '/articles/option-greeks' },
          { text: 'Request Sizing', link: '/articles/request-sizing' },
          { text: 'Concurrent Requests', link: '/articles/concurrent-requests' },
          { text: 'Configuration', link: '/articles/configuration' },
          { text: 'Flat Files', link: '/articles/flat-files' },
          { text: 'Data Issues?', link: '/articles/data-issues' },
          { text: 'Error Codes', link: '/articles/error-codes' },
          { text: 'Exchanges', link: '/articles/exchanges' },
          { text: 'Trade Conditions', link: '/articles/trade-conditions' },
          { text: 'Quote Conditions', link: '/articles/quote-conditions' },
          { text: 'Building with AI / LLMs', link: '/articles/ai-llms' },
        ],
      },
      {
        text: 'API Reference',
        collapsed: false,
        items: [{ text: 'Overview', link: '/reference/' }, { text: 'Flat Files', link: '/articles/flat-files' }, ...(referenceSidebar as any)],
      },
      {
        text: 'Streaming',
        collapsed: true,
        items: [
          { text: 'Getting Started', link: '/streaming/' },
          { text: 'Handling Events', link: '/streaming/events' },
          { text: 'Reconnection & Monitoring', link: '/streaming/reliability' },
          ...(streamingSidebar as any),
        ],
      },
      {
        text: 'Server (HTTP/WS)',
        collapsed: true,
        items: [
          { text: 'Getting Started', link: '/server/' },
          { text: 'HTTP API', link: '/server/http' },
          { text: 'WebSocket Streaming', link: '/server/websocket' },
        ],
      },
      { text: 'MCP', link: '/mcp' },
      {
        text: 'Project',
        collapsed: true,
        items: [
          { text: 'Changelog', link: '/changelog' },
        ],
      },
    ],

    socialLinks: [{ icon: 'github', link: 'https://github.com/userFRM/ThetaDataDx' }],

    search: {
      provider: 'local',
    },

    footer: {
      message: 'Released under the Apache-2.0 License.',
      copyright: 'Copyright 2024-present ThetaDataDx Contributors',
    },

    editLink: {
      pattern: 'https://github.com/userFRM/ThetaDataDx/edit/main/docs-site/docs/:path',
      text: 'Edit this page on GitHub',
    },
  },
})
