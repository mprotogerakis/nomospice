import { dirname } from "node:path";
import { defineConfig, searchForWorkspaceRoot } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

// (scripts/docs-shots.sh with a scene of another site: its scenes folder served too, the scene and
// the modules beside it; never the root)
const scene = process.env.NOMOSPICE_SCENES;
const extra = scene?.startsWith("/") && dirname(scene) !== "/" ? [dirname(scene)] : [];

export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: { alias: { "@": "/src" } },
  clearScreen: false,
  server: { port: 1420, strictPort: true, fs: { allow: [searchForWorkspaceRoot(process.cwd()), ...extra] } },
});
