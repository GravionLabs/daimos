import { docsLoader } from "@astrojs/starlight/loaders";
import { docsSchema } from "@astrojs/starlight/schema";
import { defineCollection } from "astro:content";
import { z } from "astro/zod";

export const platforms = ["linux-x11", "linux-wayland", "windows", "macos"] as const;

export const collections = {
  docs: defineCollection({
    loader: docsLoader(),
    schema: docsSchema({
      extend: z.object({
        // Platforms the page applies to; leave it out when the page applies to all of them.
        platforms: z.array(z.enum(platforms)).optional(),
        // First release that has the feature, for example "0.3".
        since: z.string().regex(/^\d+\.\d+$/).optional(),
      }),
    }),
  }),
};
