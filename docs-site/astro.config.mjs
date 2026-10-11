import starlight from "@astrojs/starlight";
import { defineConfig } from "astro/config";

export default defineConfig({
  site: "https://gravionlabs.github.io",
  base: "/daimos",
  integrations: [
    starlight({
      title: "Daimos",
      description: "A cross-platform command palette for Linux (X11 and Wayland), Windows, and macOS.",
      favicon: "/favicon.png",
      social: [{ icon: "github", label: "GitHub", href: "https://github.com/GravionLabs/daimos" }],
      sidebar: [
        {
          label: "Guides",
          items: [
            { label: "Install", slug: "guides/install" },
            { label: "Wayland setup", slug: "guides/wayland" },
          ],
        },
        {
          label: "Reference",
          items: [
            { label: "Keyboard", slug: "reference/keyboard" },
            { label: "Command line", slug: "reference/cli" },
          ],
        },
      ],
    }),
  ],
});
