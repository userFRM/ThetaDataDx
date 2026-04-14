import { defineConfig } from 'vitepress'
import { withMermaid } from 'vitepress-plugin-mermaid'

export default withMermaid(defineConfig({
  title: 'ThetaDataDx',
  description: 'Direct-wire SDK for ThetaData market data',
  base: '/ThetaDataDx/',
  cleanUrls: true,

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
      { text: 'Query Builder', link: '/query-builder' },
      { text: 'Tools', link: '/tools/cli' },
      { text: 'Changelog', link: '/changelog' },
      {
        text: 'ThetaData Docs',
        link: 'https://docs.thetadata.us/',
      },
      {
        text: 'GitHub',
        link: 'https://github.com/userFRM/ThetaDataDx',
      },
    ],

    sidebar: [
      {
        text: 'Query Builder',
        items: [
          { text: 'Interactive Builder', link: '/query-builder' },
        ],
      },
      {
        text: 'Getting Started',
        collapsed: false,
        items: [
          { text: 'Overview', link: '/getting-started/' },
          { text: 'Subscription Tiers', link: '/getting-started/subscriptions' },
          { text: 'Installation', link: '/getting-started/installation' },
          { text: 'Authentication', link: '/getting-started/authentication' },
          { text: 'Quick Start', link: '/getting-started/quickstart' },
        ],
      },
      {
        text: 'Historical Data',
        link: '/historical/',
        collapsed: true,
        items: [
          {
            text: 'Stock',
            collapsed: true,
            items: [
              { text: 'Overview', link: '/historical/stock/' },
              {
                text: 'List',
                collapsed: true,
                items: [
                  { text: 'Symbols', link: '/historical/stock/list/symbols' },
                  { text: 'Dates', link: '/historical/stock/list/dates' },
                ],
              },
              {
                text: 'Snapshot',
                collapsed: true,
                items: [
                  { text: 'OHLC', link: '/historical/stock/snapshot/ohlc' },
                  { text: 'Trade', link: '/historical/stock/snapshot/trade' },
                  { text: 'Quote', link: '/historical/stock/snapshot/quote' },
                  { text: 'Market Value', link: '/historical/stock/snapshot/market-value' },
                ],
              },
              {
                text: 'History',
                collapsed: true,
                items: [
                  { text: 'EOD', link: '/historical/stock/history/eod' },
                  { text: 'OHLC', link: '/historical/stock/history/ohlc' },
                  { text: 'Trade', link: '/historical/stock/history/trade' },
                  { text: 'Quote', link: '/historical/stock/history/quote' },
                  { text: 'Trade Quote', link: '/historical/stock/history/trade-quote' },
                ],
              },
              {
                text: 'At-Time',
                collapsed: true,
                items: [
                  { text: 'Trade', link: '/historical/stock/at-time/trade' },
                  { text: 'Quote', link: '/historical/stock/at-time/quote' },
                ],
              },
            ],
          },
          {
            text: 'Option',
            link: '/historical/option/',
            collapsed: true,
            items: [
              {
                text: 'List',
                collapsed: true,
                items: [
                  { text: 'Roots', link: '/historical/option/list/roots' },
                  { text: 'Dates', link: '/historical/option/list/dates' },
                  { text: 'Strikes', link: '/historical/option/list/strikes' },
                  { text: 'Expirations', link: '/historical/option/list/expirations' },
                  { text: 'Contracts', link: '/historical/option/list/contracts' },
                ],
              },
              {
                text: 'Snapshot',
                collapsed: true,
                items: [
                  { text: 'OHLC', link: '/historical/option/snapshot/ohlc' },
                  { text: 'Trade', link: '/historical/option/snapshot/trade' },
                  { text: 'Quote', link: '/historical/option/snapshot/quote' },
                  { text: 'Open Interest', link: '/historical/option/snapshot/open-interest' },
                  { text: 'Greeks IV', link: '/historical/option/snapshot/greeks-iv' },
                  { text: 'Greeks All', link: '/historical/option/snapshot/greeks-all' },
                  { text: 'Greeks 1st Order', link: '/historical/option/snapshot/greeks-first-order' },
                  { text: 'Greeks 2nd Order', link: '/historical/option/snapshot/greeks-second-order' },
                  { text: 'Greeks 3rd Order', link: '/historical/option/snapshot/greeks-third-order' },
                ],
              },
              {
                text: 'History',
                collapsed: true,
                items: [
                  { text: 'EOD', link: '/historical/option/history/eod' },
                  { text: 'OHLC', link: '/historical/option/history/ohlc' },
                  { text: 'Trade', link: '/historical/option/history/trade' },
                  { text: 'Quote', link: '/historical/option/history/quote' },
                  { text: 'Trade Quote', link: '/historical/option/history/trade-quote' },
                  { text: 'Open Interest', link: '/historical/option/history/open-interest' },
                  { text: 'Greeks EOD', link: '/historical/option/history/greeks-eod' },
                  { text: 'Greeks All', link: '/historical/option/history/greeks-all' },
                  { text: 'Greeks 1st Order', link: '/historical/option/history/greeks-first-order' },
                  { text: 'Greeks 2nd Order', link: '/historical/option/history/greeks-second-order' },
                  { text: 'Greeks 3rd Order', link: '/historical/option/history/greeks-third-order' },
                  { text: 'Greeks IV', link: '/historical/option/history/greeks-iv' },
                  { text: 'Trade Greeks All', link: '/historical/option/history/trade-greeks-all' },
                  { text: 'Trade Greeks 1st Order', link: '/historical/option/history/trade-greeks-first-order' },
                  { text: 'Trade Greeks 2nd Order', link: '/historical/option/history/trade-greeks-second-order' },
                  { text: 'Trade Greeks 3rd Order', link: '/historical/option/history/trade-greeks-third-order' },
                  { text: 'Trade Greeks IV', link: '/historical/option/history/trade-greeks-iv' },
                ],
              },
              {
                text: 'At-Time',
                collapsed: true,
                items: [
                  { text: 'Trade', link: '/historical/option/at-time/trade' },
                  { text: 'Quote', link: '/historical/option/at-time/quote' },
                ],
              },
            ],
          },
          {
            text: 'Index',
            link: '/historical/index-data/',
            collapsed: true,
            items: [
              {
                text: 'List',
                collapsed: true,
                items: [
                  { text: 'Symbols', link: '/historical/index-data/list/symbols' },
                  { text: 'Dates', link: '/historical/index-data/list/dates' },
                ],
              },
              {
                text: 'Snapshot',
                collapsed: true,
                items: [
                  { text: 'OHLC', link: '/historical/index-data/snapshot/ohlc' },
                  { text: 'Price', link: '/historical/index-data/snapshot/price' },
                  { text: 'Market Value', link: '/historical/index-data/snapshot/market-value' },
                ],
              },
              {
                text: 'History',
                collapsed: true,
                items: [
                  { text: 'EOD', link: '/historical/index-data/history/eod' },
                  { text: 'OHLC', link: '/historical/index-data/history/ohlc' },
                  { text: 'Price', link: '/historical/index-data/history/price' },
                ],
              },
              {
                text: 'At-Time',
                collapsed: true,
                items: [
                  { text: 'Price', link: '/historical/index-data/at-time/price' },
                ],
              },
            ],
          },
          {
            text: 'Calendar',
            link: '/historical/calendar/',
            collapsed: true,
            items: [
              { text: 'Open Today', link: '/historical/calendar/open-today' },
              { text: 'On Date', link: '/historical/calendar/on-date' },
              { text: 'Year', link: '/historical/calendar/year' },
            ],
          },
          {
            text: 'Rate',
            link: '/historical/rate/',
            collapsed: true,
            items: [
              { text: 'EOD', link: '/historical/rate/eod' },
            ],
          },
        ],
      },
      {
        text: 'Real-Time Streaming',
        collapsed: true,
        items: [
          { text: 'Overview', link: '/streaming/' },
          { text: 'Connecting & Subscribing', link: '/streaming/connection' },
          { text: 'Handling Events', link: '/streaming/events' },
          { text: 'Reconnection & Errors', link: '/streaming/reconnection' },
          { text: 'Latency Measurement', link: '/streaming/latency' },
        ],
      },
      {
        text: 'More',
        collapsed: true,
        items: [
          { text: 'Options & Greeks', link: '/options' },
          { text: 'Configuration', link: '/configuration' },
          { text: 'Jupyter Notebooks', link: '/notebooks' },
        ],
      },
      {
        text: 'Reference',
        collapsed: true,
        items: [
          { text: 'API Reference', link: '/api-reference' },
        ],
      },
      {
        text: 'Tools',
        link: '/tools/',
        collapsed: true,
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
      { icon: 'github', link: 'https://github.com/userFRM/ThetaDataDx' },
    ],

    search: {
      provider: 'local',
    },

    footer: {
      message: 'Released under the GPL-3.0-or-later License.',
      copyright: 'Copyright 2024-present ThetaDataDx Contributors',
    },

    editLink: {
      pattern: 'https://github.com/userFRM/ThetaDataDx/edit/main/docs-site/docs/:path',
      text: 'Edit this page on GitHub',
    },
  },
}))
