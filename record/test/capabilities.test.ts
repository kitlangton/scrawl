import assert from "node:assert/strict";
import test from "node:test";
import { createRecordFixture, fixtureScenarios } from "../src/capabilities/fixtures.ts";
import type { CapabilitySnapshot, RecordCommand } from "../src/capabilities/index.ts";
type Payload<T = RecordCommand> = T extends RecordCommand ? Omit<T, "requestId" | "expected"> : never;
const command = (snapshot: CapabilitySnapshot, payload: Payload): RecordCommand => ({
  ...payload, requestId: `req-${payload.type}`, expected: { providerEpoch: snapshot.providerEpoch, revision: snapshot.revision },
});

test("permission relaunch and active capture refuse start without manufactured facts", async () => {
  assert.ok(fixtureScenarios.length > 20);
  for (const scenario of ["permission-granted-relaunch-needed", "unrelated-capture"] as const) {
    const { client, driver } = createRecordFixture(scenario);
    const before = driver.current();
    const reply = await client.execute(command(before, { type: "start-capture", delivery: "local-only" }));
    assert.equal(reply.status, "refused");
    assert.deepEqual(driver.current(), before);
    assert.deepEqual(driver.commands(), []);
  }
});

test("synthetic onboarding capability fixtures keep physical trials and durable-note stroke barriers explicit", () => {
  const observed = createRecordFixture("shortcut-trial-physical-press-release").driver.current();
  assert.equal(observed.shortcutTrial?.state, "observed");
  if (observed.shortcutTrial?.state === "observed") {
    assert.equal(observed.shortcutTrial.effectiveBinding, observed.shortcut.effective);
    assert.ok(observed.shortcutTrial.reservationId && observed.shortcutTrial.pressId);
    assert.ok(Date.parse(observed.shortcutTrial.releasedAt) >= Date.parse(observed.shortcutTrial.pressedAt));
  }
  assert.equal(createRecordFixture("shortcut-trial-no-event").driver.current().shortcutTrial?.state, "reserved");
  const unrelatedTrial = createRecordFixture("shortcut-trial-unrelated-request").driver.current().shortcutTrial;
  assert.equal(unrelatedTrial?.state, "observed");
  if (unrelatedTrial?.state === "observed") assert.equal(unrelatedTrial.reservationId, "other-request-reservation");
  assert.equal(createRecordFixture("shortcut-trial-stale-epoch").driver.current().shortcutTrial?.state, "unknown");
  assert.equal(createRecordFixture("shortcut-trial-changed-binding").driver.current().shortcutTrial?.state, "unknown");
  for (const state of ["expired", "cancelled", "unknown"] as const)
    assert.equal(createRecordFixture(`shortcut-trial-${state}`).driver.current().shortcutTrial?.state, state);

  for (const [scenario, reason] of [["practice-note-draft", "draft"], ["practice-note-empty", "empty"], ["practice-note-failed", "failed"], ["practice-note-cancelled", "cancelled"]] as const) {
    const capture = createRecordFixture(scenario).driver.current().captures[0]!;
    assert.deepEqual(capture.outcomes[0], { kind: "note-save", at: "2025-01-01T00:00:05.000Z", ok: false, reason });
  }
  const savedNote = createRecordFixture("practice-note-saved").driver.current().captures[0]!.outcomes[0]!;
  assert.equal(savedNote.kind, "note-save");
  if (savedNote.kind === "note-save" && savedNote.ok) {
    assert.ok(savedNote.noteId && savedNote.receiptId);
    assert.deepEqual(savedNote.screenStrokeBarrier, { historyId: "screen-history-1", sequence: 10 });
    assert.equal(JSON.stringify(savedNote).includes("content"), false);
  }
  const strokeSequence = (scenario: import("../src/capabilities/fixtures.ts").FixtureScenario) => {
    const facts = createRecordFixture(scenario).driver.current();
    return facts.captures.flatMap(capture => capture.outcomes).filter((outcome): outcome is Extract<typeof outcome, { kind: "accepted-stroke"; ok: true; surface: "screen" }> => outcome.kind === "accepted-stroke" && outcome.ok && outcome.surface === "screen");
  };
  assert.equal(strokeSequence("practice-note-early-stroke")[0]?.cursor?.sequence, 9);
  assert.equal(strokeSequence("practice-note-first-post-stroke")[0]?.cursor?.sequence, 11);
  assert.equal(strokeSequence("practice-note-unrelated-history")[0]?.cursor?.historyId, "screen-history-other");
  const unrelated = createRecordFixture("practice-note-unrelated-capture").driver.current().captures;
  assert.notEqual(unrelated[0]?.captureId, unrelated[1]?.captureId);
  assert.equal(strokeSequence("practice-note-late-pre-note-receipt")[0]?.cursor?.sequence, 9);
  const finalized = createRecordFixture("practice-note-late-finalization").driver.current();
  assert.equal(finalized.captures[0]?.phase, "saved");
  const finalNote = finalized.captures[0]?.outcomes.find((outcome): outcome is Extract<typeof outcome, { kind: "note-save" }> => outcome.kind === "note-save");
  assert.deepEqual(finalNote?.ok ? finalNote.screenStrokeBarrier : null, { historyId: "screen-history-1", sequence: 12 });
  assert.equal(strokeSequence("practice-note-late-finalization")[0]?.cursor?.sequence, 12);
});

test("external shortcut save and failed pen write preserve effective facts", async () => {
  const external = createRecordFixture("external-shortcut-save-failure");
  const saved = await external.client.execute(command(external.driver.current(), { type: "shortcut-save", sessionId: "edit-1" }));
  assert.equal(saved.status, "refused");
  assert.equal(saved.snapshot.shortcut.registration, "externally-managed");
  const pen = createRecordFixture("pen-write-failure-preserves-effective");
  const selected = await pen.client.execute(command(pen.driver.current(), { type: "select-pen", value: "blue" }));
  assert.equal(selected.status, "refused");
  assert.equal(selected.snapshot.pen.effective, "coral");
});

test("command acceptance is distinct from explicit provider facts and observer delivery", async () => {
  const { client, driver } = createRecordFixture("pen-write-failure-preserves-effective");
  const seen: CapabilitySnapshot[] = [];
  client.observe(value => seen.push(value));
  const before = driver.current();
  const result = await client.execute(command(before, { type: "start-capture", delivery: "local-only" }));
  assert.equal(result.status, "accepted");
  assert.deepEqual(result.snapshot.captures, []);
  assert.equal(seen.length, 1);
  assert.equal(driver.commands()[0].type, "start-capture");
  assert.deepEqual(driver.commands()[0], command(before, { type: "start-capture", delivery: "local-only" }));
  const started: CapabilitySnapshot = { ...before, revision: 2, captures: [{ captureId: "controlled-id", invocationId: result.requestId, delivery: "unknown", phase: "recording", outcomes: [{ kind: "first-frame-start", at: "2025-01-01T00:00:00Z", ok: true }] }] };
  assert.equal(driver.publish(started), true);
  assert.deepEqual(seen.at(-1), started);
});

test("snapshot publication refuses regression, wrong epochs and connection changes", () => {
  const { client, driver } = createRecordFixture();
  const seen: number[] = [];
  client.observe(value => seen.push(value.revision));
  const first = driver.current();
  assert.equal(driver.publish(first), false);
  assert.equal(driver.publish({ ...first, revision: 0 }), false);
  assert.equal(driver.publish({ ...first, providerEpoch: "other", revision: 2 }), false);
  assert.equal(driver.publish({ ...first, connection: "disconnected", revision: 2 }), false);
  assert.equal(driver.publish({ ...first, revision: 2 }), true);
  assert.deepEqual(seen, [1, 2]);
});

test("reentrant publication preserves observer order and isolates listener failures", () => {
  const { client, driver } = createRecordFixture();
  const seen: number[] = [];
  client.observe(() => { throw new Error("broken consumer"); });
  client.observe(value => { if (value.revision === 2) driver.publish({ ...value, revision: 3 }); });
  client.observe(value => seen.push(value.revision));
  driver.publish({ ...driver.current(), revision: 2 });
  assert.deepEqual(seen, [1, 2, 3]);
});

test("disconnection refuses commands; reconnect preserves saved facts and retires old epochs", async () => {
  const { client, driver } = createRecordFixture("reconnect-with-saved-history");
  const seen: CapabilitySnapshot[] = [];
  client.observe(value => seen.push(value));
  const original = driver.current();
  driver.disconnect();
  const disconnected = driver.current();
  assert.equal((await client.execute(command(disconnected, { type: "refresh" }))).status, "refused");
  assert.equal(driver.reconnect(), true);
  assert.equal(driver.current().providerEpoch, original.providerEpoch);
  assert.equal(driver.reconnect({ ...driver.current(), providerEpoch: "fixture-epoch-2", revision: 1 }), true);
  assert.equal((await client.execute(command(original, { type: "refresh" }))).status, "refused");
  assert.equal(driver.reconnect({ ...original, revision: 100 }), false);
  assert.equal(seen.at(-1)?.captures[0].captureId, "capture-saved");
  assert.equal(seen.at(-1)?.captures[0].outcomes[0].kind, "save");
});

test("shortcut edit/save commands do not change effective binding until provider readback", async () => {
  const { client, driver } = createRecordFixture("reconnect-with-saved-history");
  const original = driver.current();
  assert.equal((await client.execute(command(original, { type: "shortcut-begin-edit" }))).status, "accepted");
  driver.publish({ ...original, revision: 2, shortcut: { ...original.shortcut, draft: { sessionId: "edit-1", value: "Ctrl+Alt+R" } } });
  assert.equal((await client.execute(command(driver.current(), { type: "shortcut-submit", sessionId: "old-session", value: "Q" }))).status, "refused");
  assert.equal((await client.execute(command(driver.current(), { type: "shortcut-submit", sessionId: "edit-1", value: "Ctrl+Alt+R" }))).status, "accepted");
  assert.equal((await client.execute(command(driver.current(), { type: "shortcut-save", sessionId: "edit-1" }))).status, "accepted");
  assert.equal(driver.current().shortcut.effective, original.shortcut.effective);
  driver.publish({ ...driver.current(), revision: 3, shortcut: { ...original.shortcut, effective: "Ctrl+Alt+R", label: "Ctrl Alt R", draft: null } });
  assert.equal(driver.current().shortcut.effective, "Ctrl+Alt+R");
});

test("copy/open admission is not successful clipboard/open outcome", async () => {
  const { client, driver } = createRecordFixture("reconnect-with-saved-history");
  const before = driver.current();
  for (const type of ["copy-local", "open-capture"] as const) {
    assert.equal((await client.execute(command(driver.current(), { type, captureId: "unrelated" }))).status, "refused");
    assert.equal((await client.execute(command(driver.current(), { type, captureId: "capture-saved" }))).status, "accepted");
  }
  assert.deepEqual(driver.current(), before);
  driver.publish({ ...before, revision: 2, captures: [{ ...before.captures[0], outcomes: [...before.captures[0].outcomes, { kind: "copy", at: "2025-01-01T00:00:02Z", ok: false }, { kind: "open-dispatch", at: "2025-01-01T00:00:03Z", ok: true }] }] });
  assert.equal(driver.current().captures[0].outcomes.find(o => o.kind === "copy")?.ok, false);
});

test("ambiguous control preserves identity; unsubscribing/detaching never stops capture", async () => {
  const { client, driver } = createRecordFixture("ambiguous-command-preserves-capture");
  const result = await client.execute(command(driver.current(), { type: "stop-capture", captureId: "capture-active" }));
  assert.equal(result.status, "outcome-unknown");
  assert.equal(driver.current().captures[0].phase, "recording");
  let calls = 0;
  const unsubscribe = client.observe(() => calls++);
  unsubscribe();
  driver.publish({ ...driver.current(), revision: 2 });
  assert.equal(calls, 1);
  client.detach();
  driver.disconnect();
  assert.equal(driver.captureContinues(), true);
  assert.equal(driver.commands().length, 1);
  assert.equal((await client.execute(command(driver.current(), { type: "cancel-capture", captureId: "capture-active" }))).status, "refused");
});

test("artifact open confirmation requires exact capture and saved artifact; dispatch is insufficient", async () => {
  const { hasConfirmedArtifactOpen } = await import('../src/capabilities/helpers.ts');
  const { client, driver } = createRecordFixture('reconnect-with-saved-history');
  const before = driver.current();
  driver.publish({ ...before, revision: 2, captures: [{ ...before.captures[0], artifactId: 'artifact-1', outcomes: [...before.captures[0].outcomes, {kind:'open-dispatch', at:'2025-01-01T00:00:02Z', ok:true}] }] });
  const snapshot = driver.current();
  assert.equal((await client.execute(command(snapshot, {type:'open-capture', captureId:'capture-saved'}))).status, 'accepted');
  assert.equal(hasConfirmedArtifactOpen(driver.current(), 'capture-saved'), false);
  driver.publish({...snapshot, revision:3, captures:[{...snapshot.captures[0], outcomes:[...snapshot.captures[0].outcomes, {kind:'artifact-open', artifactId:'artifact-1', at:'2025-01-01T00:00:03Z', ok:false, reason:'refused'}]}]});
  assert.equal(hasConfirmedArtifactOpen(driver.current(), 'capture-saved'), false);
  const refused = driver.current();
  driver.publish({...refused, revision:4, captures:[{...refused.captures[0], outcomes:[...refused.captures[0].outcomes, {kind:'artifact-open', artifactId:'old-artifact', at:'2025-01-01T00:00:04Z', ok:true, evidence:'viewer-ack'}]}]});
  assert.equal(hasConfirmedArtifactOpen(driver.current(), 'capture-saved'), false);
  const oldArtifact = driver.current();
  driver.publish({...oldArtifact, revision:5, captures:[{...oldArtifact.captures[0], outcomes:[...oldArtifact.captures[0].outcomes, {kind:'artifact-open', artifactId:'artifact-1', at:'2025-01-01T00:00:05Z', ok:true, evidence:'viewer-ack'}]}]});
  assert.equal(hasConfirmedArtifactOpen(driver.current(), 'capture-saved'), true);
  assert.equal(hasConfirmedArtifactOpen(driver.current(), 'another-capture'), false);
  driver.disconnect();
  driver.reconnect();
  assert.equal(hasConfirmedArtifactOpen(driver.current(), 'capture-saved'), true);
});

test("picker and accepted-stroke facts have typed surfaces and cannot use free-form detail", () => {
  const { driver } = createRecordFixture();
  const before = driver.current();
  const typed: CapabilitySnapshot = { ...before, revision: 2, captures: [{
    captureId: "surface-facts", delivery: "unknown", phase: "recording", outcomes: [
      { kind: "picker", at: "2025-01-01T00:00:00Z", ok: true, surface: "screen", target: "window" },
      { kind: "accepted-stroke", at: "2025-01-01T00:00:01Z", ok: true, surface: "whiteboard", strokeId: "stroke-1" },
    ],
  }] };
  assert.equal(driver.publish(typed), true);
  assert.equal(driver.current().captures[0].outcomes[0].kind, "picker");
  assert.equal(driver.current().captures[0].outcomes[1].kind, "accepted-stroke");
  // @ts-expect-error picker success requires a typed screen target, not detail text
  const vaguePicker: import("../src/capabilities/index.ts").CaptureOutcome = { kind: "picker", at: "now", ok: true, detail: "window" };
  // @ts-expect-error accepted strokes require an explicit screen or whiteboard surface
  const vagueStroke: import("../src/capabilities/index.ts").CaptureOutcome = { kind: "accepted-stroke", at: "now", ok: true, detail: "whiteboard" };
  void vaguePicker; void vagueStroke;
});

test("delivery readback preserves unknown and host-default as distinct from local-only", () => {
  const { driver } = createRecordFixture("unrelated-capture");
  assert.equal(driver.current().captures[0].delivery, "unknown");
  const hostDefault: CapabilitySnapshot = { ...driver.current(), revision: 2, captures: [{
    ...driver.current().captures[0], delivery: "host-default",
  }] };
  assert.equal(driver.publish(hostDefault), true);
  assert.equal(driver.current().captures[0].delivery, "host-default");
  assert.notEqual(driver.current().captures[0].delivery, "local-only");
});

test("shortcut admission is refused until supported; dispatch alone does not arm or enforce delivery", async () => {
  const { client, driver } = createRecordFixture("unrelated-capture");
  const existing = driver.current();
  // Existing unrelated capture remains distinguishable and has no invented delivery proof.
  assert.equal(existing.captures[0].captureId, "capture-unrelated");
  assert.equal(existing.captures[0].delivery, "unknown");
  assert.equal((await client.execute(command(existing, { type: "arm-next-shortcut", admissionId: "admission-1", delivery: "local-only", expiresAt: "2025-01-01T00:01:00Z" }))).status, "refused");
  assert.equal(driver.current().nextShortcutAdmission.state, "unavailable");

  const idle = { ...existing, revision: existing.revision + 1, captures: [], nextShortcutAdmission: { state: "idle" } as const };
  assert.equal(driver.publish(idle), true);
  const armReply = await client.execute(command(driver.current(), { type: "arm-next-shortcut", admissionId: "admission-1", delivery: "local-only", expiresAt: "2025-01-01T00:01:00Z" }));
  assert.equal(armReply.status, "accepted");
  assert.equal(driver.current().nextShortcutAdmission.state, "idle");

  driver.setNextResult("refused");
  assert.equal((await client.execute(command(driver.current(), { type: "arm-next-shortcut", admissionId: "admission-refused", delivery: "local-only", expiresAt: "2025-01-01T00:01:00Z" }))).status, "refused");
  assert.equal(driver.current().nextShortcutAdmission.state, "idle");
  driver.setNextResult("outcome-unknown");
  assert.equal((await client.execute(command(driver.current(), { type: "arm-next-shortcut", admissionId: "admission-unknown", delivery: "local-only", expiresAt: "2025-01-01T00:01:00Z" }))).status, "outcome-unknown");
  assert.equal(driver.current().nextShortcutAdmission.state, "idle");

  const armed: CapabilitySnapshot = { ...driver.current(), revision: driver.current().revision + 1, nextShortcutAdmission: { state: "armed", admissionId: "admission-1", delivery: "local-only", expiresAt: "2025-01-01T00:01:00Z" } };
  assert.equal(driver.publish(armed), true);
  assert.equal((await client.execute(command(armed, { type: "start-capture", delivery: "local-only" }))).status, "refused");
  assert.equal(driver.current().captures.length, 0);
  assert.equal(driver.current().nextShortcutAdmission.state, "armed");

  const consumed: CapabilitySnapshot = { ...armed, revision: armed.revision + 1,
    nextShortcutAdmission: { state: "consumed", admissionId: "admission-1", captureId: "capture-new" },
    captures: [{ captureId: "capture-new", invocationId: "shortcut-invocation", shortcutAdmissionId: "admission-1", delivery: "local-only", phase: "recording", outcomes: [
      { kind: "shortcut", at: "2025-01-01T00:00:01Z", ok: true },
      { kind: "first-frame-start", at: "2025-01-01T00:00:02Z", ok: true },
    ] }],
  };
  assert.equal(driver.publish(consumed), true);
  assert.equal(driver.current().captures[0].captureId, "capture-new");
  assert.equal(driver.current().captures[0].shortcutAdmissionId, "admission-1");
  assert.equal(driver.current().captures[0].delivery, "local-only");
});

test("cancelling a shortcut reservation does not cancel an already recording capture", async () => {
  const { client, driver } = createRecordFixture("unrelated-capture");
  const active = driver.current();
  const armed: CapabilitySnapshot = { ...active, revision: active.revision + 1,
    nextShortcutAdmission: { state: "armed", admissionId: "admission-cancel", delivery: "host-default", expiresAt: "2025-01-01T00:01:00Z" },
  };
  assert.equal(driver.publish(armed), true);
  assert.equal((await client.execute(command(armed, { type: "cancel-next-shortcut", admissionId: "admission-cancel" }))).status, "accepted");
  assert.equal(driver.captureContinues(), true);
  const cancelled = { ...driver.current(), revision: driver.current().revision + 1, nextShortcutAdmission: { state: "cancelled", admissionId: "admission-cancel" } as const };
  assert.equal(driver.publish(cancelled), true);
  assert.equal(driver.captureContinues(), true);
});

test('pen colors match named writer values and change only after effective readback', async () => {
  const {PEN_COLOR_RGB}=await import('../src/capabilities/helpers.ts');
  assert.deepEqual(PEN_COLOR_RGB,{coral:[217,76,68],purple:[122,88,199],blue:[59,121,189],green:[57,123,97]});
  const {client,driver}=createRecordFixture('reconnect-with-saved-history');
  const supported=['coral','purple','blue','green'] as const;
  assert.deepEqual(driver.current().pen.supported,supported);
  for(const color of supported) {
    const previous=driver.current();
    const reply=await client.execute(command(previous,{type:'select-pen',value:color}));
    assert.equal(reply.status,'accepted');
    assert.equal(driver.current().pen.effective,previous.pen.effective);
    driver.publish({...previous,revision:previous.revision+1,pen:{...previous.pen,effective:color}});
    assert.equal(driver.current().pen.effective,color);
  }
  // @ts-expect-error a drawing tool is not a penColor value
  const tool=command(driver.current(),{type:'select-pen',value:'ink'});
  assert.equal((await client.execute(tool)).status,'refused');
  const legacy=driver.current();
  driver.publish({...legacy,revision:legacy.revision+1,pen:{...legacy.pen,effective:null}});
  assert.equal(driver.current().pen.effective,null);
});

test('explicit Start resumes only after next-shortcut reservation is no longer armed', async () => {
  const {client,driver}=createRecordFixture('reconnect-with-saved-history');
  const initial=driver.current();
  driver.publish({...initial,revision:initial.revision+1,nextShortcutAdmission:{state:'armed',admissionId:'one',delivery:'local-only',expiresAt:'2025-01-01T00:01:00Z'}});
  assert.equal((await client.execute(command(driver.current(),{type:'start-capture',delivery:'host-default'}))).status,'refused');
  const armed=driver.current();
  driver.publish({...armed,revision:armed.revision+1,nextShortcutAdmission:{state:'cancelled',admissionId:'one'}});
  assert.equal((await client.execute(command(driver.current(),{type:'start-capture',delivery:'host-default'}))).status,'accepted');
  assert.deepEqual(driver.current().captures,initial.captures);
});


test("shortcut trial fixtures admit only the exact idle binding and never create captures", async () => {
  const fixture = createRecordFixture("shortcut-trial-idle");
  const before = fixture.driver.current();
  const trial = before.shortcutTrial;
  assert.equal(trial?.state, "idle");
  if (trial?.state !== "idle") throw new Error("missing fixture idle binding");
  const input = { type: "begin-shortcut-trial" as const, reservationId: "trial-request", bindingId: trial.bindingId, effectiveBinding: trial.effectiveBinding,
    sourceEpoch: trial.sourceEpoch, sourceRevision: trial.sourceRevision, expiresAt: "2026-11-07T00:00:00Z" };
  assert.equal((await fixture.client.execute(command(before, { ...input, bindingId: "different" }))).status, "refused");
  assert.equal((await fixture.client.execute(command(before, input))).status, "accepted");
  assert.equal(fixture.driver.current().shortcutTrial?.state, "idle", "dispatch is not an observed press or reservation");
  assert.deepEqual(fixture.driver.current().captures, []);
  const unavailable = createRecordFixture("unsubscribe-during-recording");
  assert.equal((await unavailable.client.execute(command(unavailable.driver.current(), input))).status, "refused");
  for (const scenario of ["shortcut-trial-stale-epoch", "shortcut-trial-changed-binding", "shortcut-trial-reconnect"] as const) {
    const current = createRecordFixture(scenario).driver.current();
    assert.equal(current.shortcutTrial?.state, "unknown");
    assert.ok(current.shortcutTrial && "reservationId" in current.shortcutTrial);
  }
});

test("terminal rearm fixture keeps predecessor and fresh selectors separate; active unknown cannot rearm", async () => {
  const fixture = createRecordFixture("shortcut-trial-terminal-rearm-available");
  const before = fixture.driver.current();
  assert.equal(before.shortcutTrial?.state, "expired");
  assert.deepEqual(before.shortcutTrialBeginBinding, {
    sourceEpoch: "fixture-native-epoch-current", sourceRevision: 7, bindingId: "binding-current", effectiveBinding: "Alt+R",
  });
  const previous = before.shortcutTrial;
  if (previous?.state !== "expired") throw new Error("missing expired fixture identity");
  assert.notEqual(previous.sourceEpoch, before.shortcutTrialBeginBinding?.sourceEpoch);
  assert.notEqual(previous.bindingId, before.shortcutTrialBeginBinding?.bindingId);
  const begin = {
    type: "begin-shortcut-trial" as const, requestId: "fixture-rearm", reservationId: "fixture-next",
    ...before.shortcutTrialBeginBinding!, replaces: {
      sourceEpoch: previous.sourceEpoch, sourceRevision: previous.sourceRevision, bindingId: previous.bindingId,
      effectiveBinding: previous.effectiveBinding, reservationId: previous.reservationId,
    }, expiresAt: "2026-11-07T00:00:00.000Z", expected: { providerEpoch: before.providerEpoch, revision: before.revision },
  };
  fixture.driver.setNextResult("outcome-unknown");
  assert.equal((await fixture.client.execute(begin)).status, "outcome-unknown");
  assert.deepEqual(fixture.driver.current().shortcutTrial, before.shortcutTrial);
  assert.deepEqual(fixture.driver.current().shortcutTrialBeginBinding, before.shortcutTrialBeginBinding);
  assert.deepEqual(fixture.driver.current().captures, before.captures);

  const active = createRecordFixture("shortcut-trial-active-unknown-no-rearm");
  const activeBefore = active.driver.current();
  assert.equal(activeBefore.shortcutTrial?.state, "unknown");
  assert.equal(activeBefore.shortcutTrialBeginBinding, undefined);
  assert.equal((await active.client.execute({ ...begin, requestId: "fixture-no-active-rearm", expected: { providerEpoch: activeBefore.providerEpoch, revision: activeBefore.revision } })).status, "refused");
});
