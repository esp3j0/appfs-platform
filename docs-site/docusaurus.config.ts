import {themes as prismThemes} from 'prism-react-renderer';
import type {Config} from '@docusaurus/types';
import type * as Preset from '@docusaurus/preset-classic';

// This runs in Node.js - Don't use client-side code here (browser APIs, JSX).

// NOTE on editUrl: this monorepo has no canonical public remote (the git
// remotes point at the standalone `appfs` / `appfs-agent` repos). When a
// public remote for the platform repo exists, set `editUrl` to its GitHub
// tree to re-enable "Edit this page" links.

const config: Config = {
  title: 'AppFS 平台文档',
  tagline: '面向 AI Agent 的文件系统协议、运行时与 Agent 运行时——教学与参考',
  favicon: 'img/favicon.ico',

  future: {
    v4: true,
  },

  // 生产环境的站点 URL。发布前替换为真实域名。
  url: 'https://appfs-platform.local',
  baseUrl: '/',

  onBrokenLinks: 'throw',
  markdown: {
    mermaid: true,
    hooks: {
      onBrokenMarkdownLinks: 'warn',
    },
  },

  themes: ['@docusaurus/theme-mermaid'],

  // 双语：简体中文为默认（与仓库现有 .zh-CN.md 习惯一致），英文为第二语言。
  i18n: {
    defaultLocale: 'zh-Hans',
    locales: ['zh-Hans', 'en'],
    localeConfigs: {
      'zh-Hans': {
        label: '简体中文',
        htmlLang: 'zh-Hans',
      },
      en: {
        label: 'English',
        htmlLang: 'en',
      },
    },
  },

  presets: [
    [
      'classic',
      {
        docs: {
          sidebarPath: './sidebars.ts',
          // editUrl: 'https://github.com/<org>/appfs-platform/tree/main/docs-site/',
        },
        theme: {
          customCss: './src/css/custom.css',
        },
      } satisfies Preset.Options,
    ],
  ],

  themeConfig: {
    colorMode: {
      respectPrefersColorScheme: true,
    },
    navbar: {
      title: 'AppFS 平台文档',
      items: [
        {
          type: 'docSidebar',
          sidebarId: 'tutorialSidebar',
          position: 'left',
          label: '教程',
        },
        {
          type: 'docSidebar',
          sidebarId: 'howtoSidebar',
          position: 'left',
          label: '实操指南',
        },
        {
          type: 'docSidebar',
          sidebarId: 'referenceSidebar',
          position: 'left',
          label: '参考',
        },
        {
          type: 'docSidebar',
          sidebarId: 'explanationSidebar',
          position: 'left',
          label: '原理',
        },
        {
          type: 'localeDropdown',
          position: 'right',
        },
      ],
    },
    footer: {
      style: 'dark',
      links: [
        {
          title: '教程',
          items: [
            {label: '教程总览', to: '/docs/tutorials'},
            {label: '应用开发者路径', to: '/docs/tutorials/user'},
            {label: '项目贡献者路径', to: '/docs/tutorials/contributor'},
          ],
        },
        {
          title: '文档',
          items: [
            {label: '实操指南', to: '/docs/how-to'},
            {label: '参考手册', to: '/docs/reference'},
            {label: '原理解释', to: '/docs/explanation'},
          ],
        },
        {
          title: '仓库',
          items: [
            {label: 'appfs（独立源仓库）', href: 'https://github.com/esp3j0/appfs'},
            {label: 'appfs-agent（独立源仓库）', href: 'https://github.com/esp3j0/appfs-agent'},
          ],
        },
      ],
      copyright: `Copyright © ${new Date().getFullYear()} AppFS Platform. Built with Docusaurus.`,
    },
    prism: {
      theme: prismThemes.github,
      darkTheme: prismThemes.dracula,
      additionalLanguages: ['rust', 'toml', 'yaml'],
    },
  } satisfies Preset.ThemeConfig,
};

export default config;
