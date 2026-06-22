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
      {
        text: 'Tools',
        items: [
          { text: 'Server (HTTP/WS)', link: '/server/' },
          { text: 'CLI', link: '/cli' },
          { text: 'MCP Server', link: '/mcp' },
        ],
      },
      { text: 'Examples', link: '/examples/' },
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
          { text: 'Trade & Quote Conditions', link: '/articles/conditions' },
          { text: 'Building with AI / LLMs', link: '/articles/ai-llms' },
        ],
      },
      {
        text: 'API Reference',
        collapsed: false,
        items: [{ text: 'Overview', link: '/reference/' }, ...(referenceSidebar as any)],
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
      {
        text: 'Tools',
        collapsed: true,
        items: [
          { text: 'CLI', link: '/cli' },
          { text: 'MCP Server', link: '/mcp' },
        ],
      },
      {
        text: 'Code Examples',
        collapsed: true,
        items: [
          { text: 'Query Builder', link: '/examples/' },
          { text: 'Option Chain Snapshot', link: '/examples/option-chain' },
          { text: 'DataFrames', link: '/examples/dataframes' },
          { text: 'Bulk Backfill', link: '/examples/bulk-backfill' },
          { text: 'Streaming Watchlist', link: '/examples/streaming-watchlist' },
          { text: 'Quotes At a Time of Day', link: '/examples/at-time' },
        ],
      },
      {
        text: 'Project',
        collapsed: true,
        items: [
          { text: 'Changelog', link: '/changelog' },
          { text: 'Migration: v12 → v13', link: '/migration/v12-to-v13' }, // VOCAB-OK: nav label for the migration guide whose canonical name names the transitioned versions
          { text: 'Migration: v11 → v12', link: '/migration/v11-to-v12' }, // VOCAB-OK: same rationale
          { text: 'Migration: v9 → v10 (historical)', link: '/migration/v9-to-v10' },
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
