import {test} from 'node:test';
import assert from 'node:assert/strict';
import {coachPosition, coachDimensions, practiceExit} from '../../electron/onboarding/presentation.ts';
import {borderCommand} from '../../electron/onboarding/hyprland.ts';

test('Omarchy walkthrough: coach clears the compositor-moved board and recording controls', () => {
  // Synthetic geometry reproducing the reported 1920x1200 arrangement; no private media.
  for (const scale of [1, 1.25, 2]) {
    const area = {x: -1920 / scale, y: 30 / scale, width: 1920 / scale, height: 1170 / scale};
    const board = [-955 / scale, 327 / scale, 704 / scale, 580 / scale] as const;
    const pill = [-1254 / scale, 24 / scale, 588 / scale, 66 / scale] as const;
    const p = coachPosition(area, pill, board, 'board-ink'), size = coachDimensions(area, 'board-ink');
    assert.ok(p.x >= area.x && p.x + size.width <= area.x + area.width);
    assert.ok(p.y >= area.y && p.y + size.height <= area.y + area.height);
    for (const [x, y, w, h] of [board, pill]) assert.ok(p.x + size.width <= x || p.x >= x + w || p.y + size.height <= y || p.y >= y + h);
  }
});

test('missing Linux anchor keeps the coach near the top centre instead of the far edge', () => {
  const area = {x: 0, y: 30, width: 1920, height: 1170};
  const p = coachPosition(area, null, null, 'whiteboard');
  assert.equal(p.x + 190, 960);
  assert.ok(p.y < 200);
  assert.equal(p.pointsUp, false, 'an unknown control must not get a guessed arrow');
});

test('closing the active guide explicitly preserves recording and never claims completion', () => {
  assert.match(practiceExit('note').detail, /Recording continues/);
  assert.match(practiceExit('board-ink').detail, /Recording continues/);
  assert.doesNotMatch(practiceExit('preparing').detail, /Recording continues/);
});

test('compositor mutation is addressed to the owned PID and exact native title only', () => {
  const command = borderCommand(4812, 'Apparent onboarding 4812');
  assert.match(command, /#m == 1 and m\[1\].pid == 4812 and m\[1\].xwayland/);
  assert.match(command, /prop = 'border_size', value = '0'/);
  assert.doesNotMatch(command, /active_window|window_rule|bindings/);
  for (const title of ['Apparent onboarding 4820', "'; assert(false); '", 'Settings']) assert.throws(() => borderCommand(4812, title));
});
