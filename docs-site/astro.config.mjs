import starlight from "@astrojs/starlight";
import { defineConfig } from "astro/config";

export default defineConfig({
  site: "https://gravionlabs.github.io",
  base: "/daimos",
  redirects: {
    "/guides/wayland": "/daimos/platforms/wayland/",
  },
  integrations: [
    starlight({
      title: "Daimos",
      description: "A cross-platform command palette for Linux (X11 and Wayland), Windows, and macOS.",
      favicon: "/favicon.png",
      social: [{ icon: "github", label: "GitHub", href: "https://github.com/GravionLabs/daimos" }],
      components: { PageTitle: "./src/components/PageTitle.astro" },
      sidebar: [
        {
          label: "Guides",
          items: [
            { label: "Overview", slug: "guides" },
            { label: "Install", slug: "guides/install" },
          ],
        },
        {
          label: "Platforms",
          items: [
            { label: "Overview", slug: "platforms" },
            { label: "Wayland setup", slug: "platforms/wayland" },
          ],
        },
        {
          label: "Reference",
          items: [
            { label: "Overview", slug: "reference" },
            { label: "Keyboard", slug: "reference/keyboard" },
            { label: "Command line", slug: "reference/cli" },
          ],
        },
        {
          label: "Extensions",
          items: [
            { label: "Overview", slug: "extensions" },
            { label: "Manifest", slug: "extensions/manifest" },
            { label: "Script extensions", slug: "extensions/scripts" },
            { label: "Extension commands", slug: "extensions/cli" },
          ],
        },
        { label: "Contributing", items: [{ label: "Overview", slug: "contributing" }] },
      ],
    }),
  ],
});
