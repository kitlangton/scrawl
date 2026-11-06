import assert from "node:assert/strict";
import test from "node:test";
import { parseConnectCommand } from "../../src/cli/connect.ts";

test("connect and cloud prefixes normalize, with login defaults and explicit intents", () => {
  for (const prefix of ["connect", "cloud"]) {
    assert.deepEqual(parseConnectCommand([prefix, "login"]), {
      kind: "login", scope: "read", consentUpload: false, browser: "default",
    });
    assert.deepEqual(parseConnectCommand([prefix, "login", "--scope=upload", "--consent-upload", "--no-browser", "--server", "https://example.test/connect"]), {
      kind: "login", scope: "upload", consentUpload: true, browser: "print-only", server: "https://example.test/connect",
    });
    assert.deepEqual(parseConnectCommand([prefix, "login", "--browser", "--scope", "read"]), {
      kind: "login", scope: "read", consentUpload: false, browser: "open",
    });
  }
});

test("all non-login actions preserve positional values", () => {
  assert.deepEqual(parseConnectCommand(["connect", "status"]), { kind: "status", account: false });
  assert.deepEqual(parseConnectCommand(["cloud", "status", "--account"]), { kind: "status", account: true });
  for (const verb of ["disable", "logout", "retry"] as const) assert.deepEqual(parseConnectCommand([verb]), { kind: verb });
  assert.deepEqual(parseConnectCommand(["upload", "./synthetic capture-雪.apparent"]), { kind: "upload", bundle: "./synthetic capture-雪.apparent" });
  assert.deepEqual(parseConnectCommand(["upload", "--", "-capture.apparent"]), { kind: "upload", bundle: "-capture.apparent" });
  assert.deepEqual(parseConnectCommand(["shares", "list"]), { kind: "shares-list" });
  assert.deepEqual(parseConnectCommand(["shares", "revoke", "capture-id-is-not-validated"]), { kind: "shares-revoke", id: "capture-id-is-not-validated" });
});

test("invalid flags, values, conflicts, duplicates, arity and separators fail safely", () => {
  const rejected = [
    [], ["connect"], ["login", "--browser", "--no-browser"], ["login", "--browser", "--browser"],
    ["login", "--scope", "admin"], ["login", "--scope=read", "--scope", "upload"],
    ["login", "--server"], ["login", "--server", "--browser"], ["login", "--server=x", "--server=y"],
    ["login", "--wat"], ["login", "--" , "extra"], ["status", "--account", "--account"],
    ["status", "--wat"], ["disable", "extra"], ["logout", "--account"], ["retry", "extra"],
    ["upload"], ["upload", "a", "b"], ["upload", "--wat"], ["shares"], ["shares", "wat"],
    ["shares", "list", "extra"], ["shares", "revoke"], ["shares", "revoke", "a", "b"],
  ];
  for (const args of rejected) assert.throws(() => parseConnectCommand(args), { category: "invalid_arguments" }, args.join(" "));
});

test("errors do not echo supplied URL, token-like values or paths", () => {
  for (const secret of ["https://private.example/token-value", "/private/capture.apparent"]) {
    assert.throws(() => parseConnectCommand(["login", "--scope", secret]), error => {
      assert.equal((error as { category?: string }).category, "invalid_arguments");
      assert.equal((error as Error).message.includes(secret), false);
      return true;
    });
  }
});

test("returned actions are frozen", () => {
  assert.equal(Object.isFrozen(parseConnectCommand(["login"])), true);
  assert.equal(Object.isFrozen(parseConnectCommand(["upload", "bundle"])), true);
});

test("terminal separators preserve actions without positional arguments", () => {
  assert.deepEqual(parseConnectCommand(["connect", "status", "--"]), { kind: "status", account: false });
  assert.deepEqual(parseConnectCommand(["status", "--account", "--"]), { kind: "status", account: true });
  for (const verb of ["disable", "logout", "retry"] as const) assert.deepEqual(parseConnectCommand([verb, "--"]), { kind: verb });
  assert.deepEqual(parseConnectCommand(["shares", "list", "--"]), { kind: "shares-list" });
  assert.throws(() => parseConnectCommand(["status", "--", "--account"]), { category: "invalid_arguments" });
});

test("attached values and lone hyphens remain values, while separate flags are not consumed", () => {
  assert.equal((parseConnectCommand(["login", "--server=--literal"]) as { server: string }).server, "--literal");
  assert.equal((parseConnectCommand(["login", "--server", "-"]) as { server: string }).server, "-");
  assert.deepEqual(parseConnectCommand(["upload", "-"]), { kind: "upload", bundle: "-" });
  assert.deepEqual(parseConnectCommand(["shares", "revoke", "-"]), { kind: "shares-revoke", id: "-" });
  assert.throws(() => parseConnectCommand(["login", "--server", "-x"]), { category: "invalid_arguments" });
});
