// Protocol fixture only: real flock/rename behavior is covered by native.test.ts.
const [scenario, command] = process.argv.slice(2);
const emit = (state: string) => process.stdout.write(JSON.stringify({ protocol: 1, state }) + "\n");
if (scenario === "malformed") { process.stdout.write('{"protocol":1,"state":"acquired","state":"busy"}\n'); process.exit(); }
if (command === "acl") { emit(scenario === "refused" ? "refused" : "accepted"); process.exit(); }
if (command === "lock") {
  if (scenario === "busy") { emit("busy"); process.exit(); }
  emit("acquired");
  if (scenario === "lost") { setTimeout(() => process.exit(7), 30); }
  else { process.stdin.resume(); process.stdin.once("end", () => { emit("released"); process.exit(); }); }
} else {
  if (scenario === "silent") { process.stdin.resume(); setTimeout(() => process.exit(), 1000); }
  else if (scenario === "delayed-published") { setTimeout(() => { emit("published"); process.exit(); }, 150); }
  else { emit(scenario); process.exit(); }
}
