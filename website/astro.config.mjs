import { defineConfig } from 'astro/config';
import mdx from '@astrojs/mdx';
import sitemap from '@astrojs/sitemap';

export default defineConfig({
  site: 'https://codiv.ai',
  base: '/',
  integrations: [mdx(), sitemap()],
  redirects: {
    '/install.sh': 'https://raw.githubusercontent.com/razorback16/codiv/main/install.sh',
  },
  markdown: {
    syntaxHighlight: {
      excludeLangs: ['mermaid'],
    },
    shikiConfig: {
      themes: {
        light: 'github-light',
        dark: 'github-dark',
      },
    },
  },
});
