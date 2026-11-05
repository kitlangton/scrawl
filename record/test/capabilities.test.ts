import assert from "node:assert/strict";
import test from "node:test";
import { createRecordFixture, fixtureScenarios } from "../src/capabilities/fixtures.ts";
import type { RecordCommand } from "../src/capabilities/index.ts";

const command = (snapshot: ReturnType<ReturnType<typeof createRecordFixture>["driver"]["current"]>, type: RecordCommand["type"], extra: object = {}): RecordCommand => ({
  type, requestId: `req-${type}`, expected: { providerEpoch: snapshot.providerEpoch, revision: snapshot.revision }, ...extra,
} as RecordCommand);

test("scenario catalog stays deterministic and includes the requested host conditions", async () => {
  assert.deepEqual(fixtureScenarios, ["permission-granted-relaunch-needed", "external-shortcut-save-failure", "pen-write-failure-preserves-effective", "unrelated-capture", "reconnect-with-saved-history", "ambiguous-command-preserves-capture", "unsubscribe-during-recording"]);
  const permission = createRecordFixture("permission-granted-relaunch-needed").driver.current();
  assert.equal(permission.readiness.requirements[0].status, "granted");
  assert.equal(permission.readiness.requirements[0].usableNow, false);
  assert.equal(permission.readiness.requirements[0].relaunchNeeded, true);
  const external = createRecordFixture("external-shortcut-save-failure").driver.current();
  assert.equal(external.shortcut.registration, "externally-managed");
  assert.equal(external.shortcut.externallyManaged, true);
  const { client, driver } = createRecordFixture("external-shortcut-save-failure");
  const save = await client.execute(command(driver.current(), "shortcut-save", { sessionId: "edit-1" }));
  assert.equal(save.status, "refused");
  assert.equal(save.snapshot.shortcut.registration, "externally-managed");
});

test("observe immediately supplies full state and isolates throwing listeners", () => {
  const { client, driver } = createRecordFixture();
  const seen: number[] = [];
  client.observe(() => { throw new Error("broken consumer"); });
  client.observe(snapshot => seen.push(snapshot.revision));
  driver.publish({ ...driver.current(), revision: 2 });
  assert.deepEqual(seen, [1, 2]);
});

test("stale revisions and old provider epochs fail closed", async () => {
  const { client, driver } = createRecordFixture();
  const first = driver.current();
  driver.publish({ ...first, revision: first.revision + 1 });
  assert.equal((await client.execute(command(first, "refresh"))).status, "refused");
  const current = driver.current();
  driver.reconnect();
  assert.equal((await client.execute(command(current, "refresh"))).status, "refused");
});

test("reconnect sends a full replacement with new epoch, higher revision and factual saved history", () => {
  const { client, driver } = createRecordFixture("reconnect-with-saved-history");
  const seen: ReturnType<typeof driver.current>[] = [];
  client.observe(snapshot => seen.push(snapshot));
  driver.disconnect();
  driver.reconnect();
  assert.equal(seen.at(-1)?.connection, "connected");
  assert.notEqual(seen.at(-1)?.providerEpoch, seen[0].providerEpoch);
  assert.ok(seen.at(-1)!.revision > seen[0].revision);
  assert.equal(seen.at(-1)?.captures[0].captureId, "capture-saved");
  assert.equal(seen.at(-1)?.captures[0].outcomes[0].kind, "save");
});

test("unknown command outcome does not assert capture success or replace identity", async () => {
  const { client, driver } = createRecordFixture("ambiguous-command-preserves-capture");
  const before = driver.current();
  driver.setNextResult("outcome-unknown");
  const result = await client.execute(command(before, "start-capture", { delivery: "local-only" }));
  assert.equal(result.status, "outcome-unknown");
  assert.deepEqual(result.snapshot.captures.map(c => c.captureId), ["capture-active"]);
});

test("failed pen write preserves effective preference", async () => {
  const { client, driver } = createRecordFixture("pen-write-failure-preserves-effective");
  const result = await client.execute(command(driver.current(), "select-pen", { value: "laser" }));
  assert.equal(result.status, "refused");
  assert.equal(result.snapshot.pen.effective, "ink");
  assert.ok(result.snapshot.pen.error);
});

test("local-only is explicit intent and creates only factual capture identity/outcome", async () => {
  const { client, driver } = createRecordFixture();
  const result = await client.execute(command(driver.current(), "start-capture", { delivery: "local-only" }));
  assert.equal(result.status, "applied");
  assert.equal(result.snapshot.captures[0].captureId, result.receipt?.captureId);
  assert.equal(result.snapshot.captures[0].invocationId, "req-start-capture");
  assert.equal(result.snapshot.captures[0].outcomes[0].kind, "first-frame-start");
  assert.equal("upload" in result.snapshot.captures[0], false);
});

test("unsubscribe and detach during capture do not stop native capture", async () => {
  const { client, driver } = createRecordFixture("unsubscribe-during-recording");
  const unsubscribe = client.observe(() => undefined);
  unsubscribe();
  client.detach();
  assert.equal(driver.captureContinues(), true);
  assert.equal(driver.current().captures[0].phase, "recording");
});

test("fixture can report cancellation separately from stopping capture", async () => {
  const { client, driver } = createRecordFixture("unrelated-capture");
  const snapshot = driver.current();
  const result = await client.execute(command(snapshot, "cancel-capture", { captureId: "capture-unrelated" }));
  assert.equal(result.status, "applied");
  assert.equal(result.snapshot.captures[0].phase, "cancelled");
});
