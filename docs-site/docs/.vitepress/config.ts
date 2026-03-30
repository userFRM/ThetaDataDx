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
      { text: 'API Reference', link: '/api-reference' },
      { text: 'Tools', link: '/tools/cli' },
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
          { text: 'Overview', link: '/getting-started/' },
          { text: 'Installation', link: '/getting-started/installation' },
          { text: 'Authentication', link: '/getting-started/authentication' },
          { text: 'Quick Start', link: '/getting-started/quickstart' },
        ],
      },
      {
        text: 'Historical Data',
        collapsed: false,
        items: [
          { text: 'Overview', link: '/historical/' },
          { text: 'Stock Endpoints', link: '/historical/stock' },
          { text: 'Option Endpoints', link: '/historical/option' },
          { text: 'Index Endpoints', link: '/historical/index-data' },
          { text: 'Calendar & Rates', link: '/historical/calendar' },
        ],
      },
      {
        text: 'Real-Time Streaming',
        collapsed: false,
        items: [
          { text: 'Overview', link: '/streaming/' },
          { text: 'Connecting & Subscribing', link: '/streaming/connection' },
          { text: 'Handling Events', link: '/streaming/events' },
          { text: 'Reconnection & Errors', link: '/streaming/reconnection' },
        ],
      },
      {
        text: 'More',
        collapsed: false,
        items: [
          { text: 'Options & Greeks', link: '/options' },
          { text: 'Configuration', link: '/configuration' },
          { text: 'Jupyter Notebooks', link: '/notebooks' },
        ],
      },
      {
        text: 'Reference',
        collapsed: false,
        items: [
          { text: 'API Reference', link: '/api-reference' },
        ],
      },
      {
        text: 'Tools',
        collapsed: false,
        items: [
          { text: 'CLI', link: '/tools/cli' },
          { text: 'MCP Server', link: '/tools/mcp' },
          { text: 'REST Server', link: '/tools/server' },
        ],
      },
      {
        text: 'Project',
        collapsed: true,
        items: [
          { text: 'Changelog', link: '/changelog' },
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
      pattern: 'https://github.com/userFRM/thetadatadx/edit/main/docs-site/docs/:path',
      text: 'Edit this page on GitHub',
    },
  },
})
