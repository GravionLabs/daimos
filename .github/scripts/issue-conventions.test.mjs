import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import {
  MARKER,
  commentBody,
  expectedTitle,
  levelOf,
  missingSections,
} from './issue-conventions.mjs';

describe('levelOf', () => {
  it('reads the level from label names or label objects', () => {
    assert.equal(levelOf(['area:bff', 'pbi']), 'pbi');
    assert.equal(levelOf([{ name: 'task' }, { name: 'area:infra' }]), 'task');
  });

  it('is null for issues without a level label', () => {
    assert.equal(levelOf(['dependencies']), null);
    assert.equal(levelOf([]), null);
  });
});

describe('expectedTitle', () => {
  it('adds a missing prefix', () => {
    assert.equal(expectedTitle('feat: board list', 'pbi'), '[PBI] feat: board list');
  });

  it('keeps a correct prefix', () => {
    assert.equal(expectedTitle('[Task] test: e2e', 'task'), '[Task] test: e2e');
  });

  it('replaces a wrong or differently written prefix', () => {
    assert.equal(expectedTitle('[Task] feat: x', 'pbi'), '[PBI] feat: x');
    assert.equal(expectedTitle('[pbi]feat: x', 'pbi'), '[PBI] feat: x');
    assert.equal(expectedTitle('  [EPIC]  Identity ', 'epic'), '[Epic] Identity');
  });
});

describe('missingSections', () => {
  const pbi =
    '### Parent Feature\n\n#1\n\n## Acceptance criteria\n\n- a\n\n### Depends on\n\n- none\n\n### Verification\n\n```sh\npnpm test\n```';

  it('accepts a PBI with all sections, at heading levels 2 to 4', () => {
    assert.deepEqual(missingSections(pbi, 'pbi'), []);
  });

  it('lists what a PBI lacks', () => {
    assert.deepEqual(missingSections('## Acceptance criteria\n- a', 'pbi'), [
      'Depends on',
      'Verification',
    ]);
  });

  it('lists what a task lacks and matches case-insensitively', () => {
    assert.deepEqual(missingSections('### files\n- x', 'task'), ['Done when']);
    assert.deepEqual(missingSections(null, 'task'), ['Files', 'Done when']);
  });

  it('does not count a heading inside a line', () => {
    assert.deepEqual(missingSections('see ### Files below\n### Done when', 'task'), ['Files']);
  });

  it('requires nothing of epics, features and bugs', () => {
    assert.deepEqual(missingSections('', 'epic'), []);
    assert.deepEqual(missingSections('', 'feature'), []);
    assert.deepEqual(missingSections('', 'bug'), []);
  });
});

describe('commentBody', () => {
  it('carries the marker and names the missing headings', () => {
    const body = commentBody('task', ['Files']);
    assert.ok(body.startsWith(MARKER));
    assert.match(body, /`### Files`/);
    assert.match(body, /needs-refinement/);
  });

  it('confirms when nothing is missing', () => {
    assert.match(commentBody('pbi', []), /All required sections for a PBI are present/);
  });
});
