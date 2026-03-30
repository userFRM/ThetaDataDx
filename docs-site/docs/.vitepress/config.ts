import { defineConfig } from 'vitepress'

export default defineConfig({
  title: 'ThetaDataDx',
  description: 'Direct-wire SDK for ThetaData market data',
  base: '/ThetaDataDx/',
  cleanUrls: true,
  ignoreDeadLinks: true,

  head: [
    ['link', { rel: 'icon', type: 'image/svg+xml', href: '/logo.svg' }],
    ['meta', { name: 'theme-color', content: '#3b82f6' }],
    ['meta', { property: 'og:type', content: 'website' }],
    ['meta', { property: 'og:title', content: 'ThetaDataDx' }],
    ['meta', { property: 'og:description', content: 'Direct-wire SDK for ThetaData market data' }],
  ],

  themeConfig: {
    logo: '/logo.svg',
    siteTitle: 'ThetaDataDx',

    nav: [
      { text: 'Guide', link: '/getting-started/' },
      { text: 'API Reference', link: '/api-reference/' },
      { text: 'Tools', link: '/tools/' },
      { text: 'Changelog', link: '/changelog' },
      {
        text: 'GitHub',
        link: 'https://github.com/userFRM/thetadatadx',
      },
    ],

    sidebar: [
      {
        text: 'Getting Started',
        collapsed: false,
        items: [
          { text: 'Introduction', link: '/getting-started/' },
          { text: 'Installation', link: '/getting-started/installation' },
          { text: 'Quick Start', link: '/getting-started/quickstart' },
          { text: 'Authentication', link: '/getting-started/authentication' },
          { text: 'Configuration', link: '/configuration' },
        ],
      },
      {
        text: 'Historical Data',
        collapsed: false,
        items: [
          { text: 'Overview', link: '/historical/' },
          { text: 'End-of-Day', link: '/historical/eod' },
          { text: 'Intraday Bars', link: '/historical/intraday' },
          { text: 'Tick-Level', link: '/historical/tick-level' },
          { text: 'Bulk Snapshots', link: '/historical/bulk-snapshots' },
        ],
      },
      {
        text: 'Real-Time Streaming',
        collapsed: false,
        items: [
          { text: 'Overview', link: '/streaming/' },
          { text: 'WebSocket Connection', link: '/streaming/websocket' },
          { text: 'Subscribing to Feeds', link: '/streaming/subscribing' },
          { text: 'Handling Messages', link: '/streaming/messages' },
          { text: 'Reconnection', link: '/streaming/reconnection' },
        ],
      },
      {
        text: 'Options & Greeks',
        collapsed: false,
        items: [
          { text: 'Options & Greeks', link: '/options' },
        ],
      },
      {
        text: 'API Reference',
        collapsed: true,
        items: [
          { text: 'Overview', link: '/api-reference/' },
          { text: 'Client', link: '/api-reference/client' },
          { text: 'Request Types', link: '/api-reference/requests' },
          { text: 'Response Types', link: '/api-reference/responses' },
          { text: 'Error Handling', link: '/api-reference/errors' },
        ],
      },
      {
        text: 'Guides',
        collapsed: true,
        items: [
          { text: 'Jupyter Notebooks', link: '/notebooks' },
        ],
      },
      {
        text: 'Tools',
        collapsed: true,
        items: [
          { text: 'CLI', link: '/tools/cli' },
          { text: 'MCP Server', link: '/tools/mcp' },
          { text: 'REST Server', link: '/tools/server' },
        ],
      },
      {
        text: 'Changelog',
        items: [
          { text: 'Release Notes', link: '/changelog' },
        ],
      },
    ],

    socialLinks: [
      { icon: 'github', link: 'https://github.com/userFRM/thetadatadx' },
    ],

    search: {
      provider: 'local',
    },

    footer: {
      message: 'Released under the MIT License.',
      copyright: 'Copyright 2024-present ThetaDataDx Contributors',
    },

    editLink: {
      pattern: 'https://github.com/userFRM/thetadatadx/edit/main/docs-site-v2/docs/:path',
      text: 'Edit this page on GitHub',
    },
  },
})
