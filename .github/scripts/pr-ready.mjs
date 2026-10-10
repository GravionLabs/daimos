// Definition of ready (AGENTS.md, "Issue conventions"): the checks the pr-ready workflow runs.

export const NOT_READY_LABELS = ['needs-refinement', 'needs-decision'];

/** Issue numbers the PR body closes with GitHub's keywords (`Closes #1`, `fixes: #2`, `Resolved #3`). */
export function closingRefs(body) {
  const refs = [
    ...(body ?? '').matchAll(/\b(?:close[sd]?|fix(?:e[sd])?|resolve[sd]?)\s*:?\s+#(\d+)\b/gi),
  ];
  return [...new Set(refs.map((match) => Number(match[1])))];
}

/**
 * Why an issue is not ready to be closed by a PR: a not-ready label, or a "blocked by" issue that is
 * still open. `blockers` are the issues it is blocked by, each `{ number, state }`.
 */
export function readinessProblems(issue, blockers) {
  const labels = issue.labels.map((label) => (typeof label === 'string' ? label : label.name));
  return [
    ...NOT_READY_LABELS.filter((name) => labels.includes(name)).map(
      (name) => `#${issue.number} has the label \`${name}\``,
    ),
    ...blockers
      .filter((blocker) => blocker.state === 'open')
      .map((blocker) => `#${issue.number} is blocked by #${blocker.number}, which is still open`),
  ];
}
