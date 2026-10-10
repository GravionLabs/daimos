import solid from "vite-plugin-solid";
import { defineConfig } from "vitest/config";

export default defineConfig({
  // solid-refresh (HMR) has no use under Vitest and breaks on Windows paths.
  plugins: [solid({ hot: !process.env.VITEST })],
  clearScreen: false,
  server: { port: 1431, strictPort: true },
  build: { target: "es2022", outDir: "dist", emptyOutDir: false },
  test: {
    environment: "jsdom",
    globals: true,
    setupFiles: ["./src/test-setup.ts"],
    server: { deps: { inline: [/solid-js/] } },
  },
  resolve: { conditions: ["development", "browser"] },
});
