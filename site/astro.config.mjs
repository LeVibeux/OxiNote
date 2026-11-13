import { defineConfig } from "astro/config";

export default defineConfig({
  trailingSlash: "always",
  prefetch: false,
  build: {
    format: "directory",
    inlineStylesheets: "never",
  },
  vite: {
    build: {
      assetsInlineLimit: 0,
    },
  },
});
