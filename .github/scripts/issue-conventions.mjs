// Issue conventions (AGENTS.md, "Issue conventions"): the checks the issue-conventions workflow runs.

export const LEVELS = { epic: 'Epic', feature: 'Feature', pbi: 'PBI', task: 'Task', bug: 'Bug' };

export const REQUIRED_SECTIONS = {
  pbi: ['Acceptance criteria', 'Depends on', 'Verification'],
  task: ['Files', 'Done when'],
};

export const MARKER = '<!-- issue-conventions -->';
export const NEEDS_REFINEMENT = 'needs-refinement';

/** The level from the issue's labels (names or `{ name }` objects), or `null` when it has none. */
export function levelOf(labels) {
  const names = new Set(labels.map((label) => (typeof label === 'string' ? label : label.name)));
  return Object.keys(LEVELS).find((level) => names.has(level)) ?? null;
}

/** The title with exactly one `[Level] ` prefix; any other leading `[...]` prefix is replaced. */
export function expectedTitle(title, level) {
  const rest = title.replace(/^\s*\[[^\]]*\]\s*/, '').trim();
  return `[${LEVELS[level]}] ${rest}`;
}

/** The required section headings (`## Name` to `#### Name`) that the body lacks. */
export function missingSections(body, level) {
  const headings = new Set(
    [...(body ?? '').matchAll(/^#{2,4}\s+(.+?)\s*$/gm)].map((match) => match[1].toLowerCase()),
  );
  return (REQUIRED_SECTIONS[level] ?? []).filter((name) => !headings.has(name.toLowerCase()));
}

/** The workflow's comment for an issue that misses sections, or that has them all again. */
export function commentBody(level, missing) {
  if (missing.length === 0) {
    return `${MARKER}\nAll required sections for a ${LEVELS[level]} are present. Thanks!`;
  }
  const list = missing.map((name) => `- \`### ${name}\``).join('\n');
  return `${MARKER}
This ${LEVELS[level]} is missing required sections (see "Issue conventions" in AGENTS.md):

${list}

It is labeled \`${NEEDS_REFINEMENT}\` until they are added; pull requests that close it fail the readiness check.`;
}
