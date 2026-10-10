import { defineConfig } from "vite";

export default defineConfig({
  clearScreen: false,
  ssr: {
    noExternal: ["gotiengviet"],
  },
  server: {
    port: 1420,
    strictPort: true,
  },
});
