# Writing for the docs site

The site is Astro Starlight. Pages are Markdown under `src/content/docs/`. Write in US English.
Build it with `pnpm --filter @daimos/docs build` from the repository root, preview it with
`pnpm --filter @daimos/docs dev`.

## Where a page goes

| Section      | Folder          | What belongs there                                                  |
| ------------ | --------------- | ------------------------------------------------------------------- |
| Guides       | `guides/`       | How to do something, in the order a user meets it                   |
| Platforms    | `platforms/`    | What works how on X11, Wayland, Windows, macOS; setup; limits       |
| Reference    | `reference/`    | Exact lists: keys, commands, settings. Generated where possible     |
| Extensions   | `extensions/`   | Writing extensions: tutorial, manifest, protocol, WIT, capabilities |
| Contributing | `contributing/` | Building from source, architecture, release process                 |

Every section has an `index.md` that lists its pages. A new page is added to that index and to the
`sidebar` in `astro.config.mjs`. Moving a page needs an entry under `redirects` so its URL keeps
working.

## Page template

```md
---
title: Short noun phrase or task
description: One sentence, shown in search results.
platforms: [linux-wayland] # leave out when the page applies to all of them
since: '0.3' # first release that has the feature; leave out for general pages
banner: # only while the page is not complete
  content: This page is not complete yet.
---

One or two sentences: what this is and when you need it.

## Steps or details

The main content. Steps are numbered; every command can be copied as written.

## Platform notes

One short paragraph or table per platform that behaves differently, including where it degrades.

## Troubleshooting

Link the troubleshooting entry or the issue for known problems.
```

`platforms` takes `linux-x11`, `linux-wayland`, `windows`, `macos`. The page header shows it as
"Applies to" and `since` as "Since vX.Y".

## Rules

- A page for a feature says what happens on X11, Wayland (KDE and GNOME), Windows, and macOS, and
  says where it degrades. Do not leave a platform out silently.
- Describe what the user sees, not the implementation. Design notes stay in `docs/specs/` and
  `docs/adr/`; the site links to them and does not copy them.
- Prefer short sentences and plain words. Use "you". Name keys in bold (**Ctrl+,**), commands and
  paths in code, and write the macOS key where it differs.
- A pull request that changes what a user sees updates the page in the same pull request. A page
  that cannot be finished yet carries the `banner` above.
- Internal links are relative (`../platforms/wayland/`). Pages that are generated say so at the top
  and are never edited by hand.
