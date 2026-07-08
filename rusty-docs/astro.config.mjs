// @ts-check
import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';

// https://astro.build/config
export default defineConfig({
  // Set this to your deployed URL (used for sitemap + canonical links).
  site: 'https://docs.rustycli.com',

  integrations: [
    starlight({
      title: 'rusty',
      description: 'A coding agent that never leaves your terminal.',

      // ─── Languages ───
      // English is the default (served at the site root); Chinese lives
      // under /zh-cn/. This makes Starlight's language picker appear next to
      // the theme selector. Pages that aren't translated yet automatically
      // fall back to the English version, so you can translate incrementally.
      defaultLocale: 'root',
      locales: {
        root: { label: 'English', lang: 'en' },
        'zh-cn': { label: '简体中文', lang: 'zh-CN' },
      },

      // Mascot mark + "rusty" wordmark in the header
      logo: {
        src: './src/assets/rusty-mark.svg',
        alt: 'rusty',
        replacesTitle: false,
      },
      favicon: '/favicon.svg',

      // The entire Rusty look lives in this one stylesheet
      customCss: ['./src/styles/rusty.css'],

      // Force Rusty-brand fonts (loaded via @import in rusty.css)
      // and keep the terminal-style code blocks in both light & dark.
      expressiveCode: {
        // Single dark theme so code always reads as a little CRT,
        // matching rustycli.com — regardless of page light/dark mode.
        themes: ['github-dark'],
        styleOverrides: {
          borderRadius: '0.7rem',
          borderColor: '#2A2018',
          codeBackground: '#140E0A',
          frameBoxShadowCssValue: '0 18px 40px -26px rgba(20, 10, 4, 0.55)',
          frames: {
            editorTabBarBackground: '#1B1510',
            editorActiveTabBackground: '#140E0A',
            editorActiveTabIndicatorBottomColor: '#E0703A',
            editorTabBarBorderBottomColor: '#2A2018',
            terminalBackground: '#140E0A',
            terminalTitlebarBackground: '#1B1510',
            terminalTitlebarBorderBottomColor: '#2A2018',
            terminalTitlebarDotsForeground: '#5A4A3A',
          },
        },
      },

      social: [
        { icon: 'github', label: 'GitHub', href: 'https://github.com/pdg-global/rusty' },
      ],

      // Enable "Edit this page" links (adjust or remove to taste)
      editLink: {
        baseUrl: 'https://github.com/pdg-global/rusty/edit/main/docs/',
      },

      // Explicit sidebar for full control over order + labels.
      // `translations` supply the Chinese labels for the language picker.
      sidebar: [
        {
          label: 'Getting Started',
          translations: { 'zh-CN': '开始使用' },
          items: [
            { label: 'Introduction', translations: { 'zh-CN': '介绍' }, slug: 'index' },
            { label: 'Installation', translations: { 'zh-CN': '安装' }, slug: 'getting-started/installation' },
            { label: 'Quickstart', translations: { 'zh-CN': '快速开始' }, slug: 'getting-started/quickstart' },
          ],
        },
        {
          label: 'Guides',
          translations: { 'zh-CN': '指南' },
          items: [
            { label: 'Agent Loop', translations: { 'zh-CN': '代理循环' }, slug: 'guides/agent-loop' },
            { label: 'Running Modes', translations: { 'zh-CN': '运行模式' }, slug: 'guides/running-modes' },
            { label: 'Sessions', translations: { 'zh-CN': '会话' }, slug: 'guides/sessions' },
            { label: 'Slash Commands', translations: { 'zh-CN': '斜杠命令' }, slug: 'guides/slash-commands' },
          ],
        },
        {
          label: 'Configuration',
          translations: { 'zh-CN': '配置' },
          items: [
            { label: 'Settings', translations: { 'zh-CN': '设置' }, slug: 'configuration/settings' },
            { label: 'Permissions', translations: { 'zh-CN': '权限' }, slug: 'configuration/permissions' },
            { label: 'Credentials', translations: { 'zh-CN': '凭据' }, slug: 'configuration/credentials' },
            { label: 'Presets', translations: { 'zh-CN': '预设' }, slug: 'configuration/presets' },
          ],
        },
        {
          label: 'Tools',
          translations: { 'zh-CN': '工具' },
          items: [
            { label: 'Overview', translations: { 'zh-CN': '概览' }, slug: 'tools/overview' },
            { label: 'File Operations', translations: { 'zh-CN': '文件操作' }, slug: 'tools/file-operations' },
            { label: 'Bash', translations: { 'zh-CN': 'Bash' }, slug: 'tools/bash' },
            { label: 'Search', translations: { 'zh-CN': '搜索' }, slug: 'tools/search' },
            { label: 'Web Fetch', translations: { 'zh-CN': '网页抓取' }, slug: 'tools/web-fetch' },
            { label: 'Sub-Agents', translations: { 'zh-CN': '子代理' }, slug: 'tools/sub-agents' },
            { label: 'Task Management', translations: { 'zh-CN': '任务管理' }, slug: 'tools/task-management' },
          ],
        },
        {
          label: 'Reference',
          translations: { 'zh-CN': '参考' },
          items: [
            { label: 'CLI Flags', translations: { 'zh-CN': 'CLI 参数' }, slug: 'reference/cli' },
          ],
        },
      ],
    }),
  ],
});
