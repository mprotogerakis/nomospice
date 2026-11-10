import { dirname } from "node:path";
import { defineConfig, searchForWorkspaceRoot } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

// (scripts/docs-shots.sh with a scene of another site: its docs folder served too)
const scene = process.env.NOMOSPICE_SCENES;
const extra = scene?.startsWith("/") ? [dirname(dirname(scene))] : [];

export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: { alias: { "@": "/src" } },
  clearScreen: false,
  server: { port: 1420, strictPort: true, fs: { allow: [searchForWorkspaceRoot(process.cwd()), ...extra] } },
});
