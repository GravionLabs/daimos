import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import { closingRefs, readinessProblems } from './pr-ready.mjs';

describe('closingRefs', () => {
  it('finds every closing keyword form, once per issue', () => {
    const body = 'Closes #407, closes #408\nFixes: #12 and resolved #13. Close #407 again.';
    assert.deepEqual(closingRefs(body), [407, 408, 12, 13]);
  });

  it('ignores mentions without a keyword and empty bodies', () => {
    assert.deepEqual(closingRefs('Relates to #5, see #6'), []);
    assert.deepEqual(closingRefs(null), []);
  });

  it('does not match inside other words', () => {
    assert.deepEqual(closingRefs('prefixes #3 and unclosed #4'), []);
  });
});

describe('readinessProblems', () => {
  const issue = (labels) => ({ number: 7, labels });

  it('is empty for a ready issue', () => {
    assert.deepEqual(readinessProblems(issue(['pbi']), [{ number: 3, state: 'closed' }]), []);
  });

  it('names both not-ready labels', () => {
    assert.deepEqual(
      readinessProblems(issue([{ name: 'needs-refinement' }, 'needs-decision']), []),
      ['#7 has the label `needs-refinement`', '#7 has the label `needs-decision`'],
    );
  });

  it('names open blockers only', () => {
    const blockers = [
      { number: 3, state: 'open' },
      { number: 4, state: 'closed' },
    ];
    assert.deepEqual(readinessProblems(issue(['pbi']), blockers), [
      '#7 is blocked by #3, which is still open',
    ]);
  });
});
