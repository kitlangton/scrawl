use super::*;
use std::io::Write;
use std::process::{Command, Stdio};

/// Exercises the actual emitted Lua, with an inert model of the pinned API's
/// exact handle enable/disable and deliberately chord-wide remove semantics.
/// This is language/protocol proof, not compositor/native input acceptance.
#[test]
#[ignore = "Requires APPARENT_LUA_TEST_BIN pointing to a Lua 5.3+ stdin interpreter"]
fn lua_handle_lifetime_contract() {
    let mut script = String::from(r#"
objects = {}; dispatched = {}
hl = { dsp = { exec_cmd = function(command) return command end } }
function hl.dispatch(command) table.insert(dispatched, command) end
function hl.bind(chord, callback, options)
  local h = { chord = chord, callback = callback, enabled = true, description = options.description }
  function h:is_enabled() if self.expired then return nil end return self.enabled end
  function h:set_enabled(value) if not self.expired then self.enabled = value end end
  function h:remove() for _, other in ipairs(objects) do if other.chord == self.chord then other.expired = true end end end
  table.insert(objects, h); return h
end
function hl.unbind() error('Chord-wide removal is forbidden') end
function live() local n=0; for _, h in ipairs(objects) do if not h.expired then n=n+1 end end; return n end
"#);
    let mut eval = |code: String| {
        // Separate blocks match separate hyprctl evaluations while preserving _G.
        script.push_str("\ndo\n");
        script.push_str(code.strip_prefix("eval ").unwrap());
        script.push_str("\nend\n");
    };
    eval(enable("0000", "record", "CTRL + ALT + S", "old-command", false));
    // A new user binding on the same chord must survive cleanup, including the
    // review12 ordering: added after discovery but before our cleanup executes.
    eval("eval user = hl.bind('CTRL + ALT + S', function() end, {description='User'})".into());
    eval(disable("0000", None));
    eval("eval assert(objects[1]:is_enabled() == false); assert(user:is_enabled() == true); assert(live()==2)".into());
    // Restart reuses the exact object and updates only its command/owner. A late
    // old Quit must not disable a successor's lease; it cannot mutate the user.
    let command = "new 'path' \\ and \"quoted\"\ncommand";
    eval(enable("0001", "record", "CTRL + ALT + S", command, true));
    eval(disable("0000", None));
    eval(format!("eval assert(live()==2); assert(objects[1]:is_enabled()); objects[1].callback(); assert(dispatched[1]=={})", lua_str(command)));
    eval(disable("0001", Some("record")));
    eval("eval assert(not objects[1]:is_enabled()); assert(user:is_enabled())".into());
    // A description with no retained handle must not be used to claim a binding.
    let missing = enable("0001", "paste", "ALT + P", "paste", true);
    eval(format!("eval assert(not pcall(function() {} end)); assert(live()==2)", missing.strip_prefix("eval ").unwrap()));
    // Pause/unpause and repeated restarts reuse one object, not an ever-growing
    // set of disabled registrations. A reload expires it and permits recreation.
    for i in 0..80 {
        eval(enable(&format!("session-{i:04}"), "record", "CTRL + ALT + S", "command", true));
        eval(disable(&format!("session-{i:04}"), None));
    }
    eval("eval assert(live()==2); objects[1].expired=true".into());
    // Neither a delayed registration from a retired process nor from the
    // just-closed process may resurrect its binding.
    for owner in ["0000", "session-0079"] {
        let stale = enable(owner, "record", "CTRL + ALT + S", "stale", false);
        eval(format!("eval assert(not pcall(function() {} end))", stale.strip_prefix("eval ").unwrap()));
    }
    eval(enable("session-0080", "record", "CTRL + ALT + S", "after-reload", false));
    eval("eval assert(live()==2); assert(user:is_enabled())".into());
    // Finite resource bound; refusal cannot delete any existing binding.
    for i in 1..64 {
        eval(enable("session-0080", "record", &format!("CTRL + F{i}"), "command", false));
    }
    let overflow = enable("session-0080", "record", "CTRL + Overflow", "command", false);
    eval(format!("eval assert(not pcall(function() {} end)); assert(live()==65)", overflow.strip_prefix("eval ").unwrap()));
    // Reload discards only compositor-managed registrations in this model.
    // Both action slots survive a pause of one slot, but Quit disables both.
    eval("eval for _, h in ipairs(objects) do if h ~= user then h.expired = true end end".into());
    eval(enable("session-0081", "record", "ALT + S", "record", false));
    eval(enable("session-0081", "paste", "ALT + P", "paste", false));
    eval(disable("session-0081", Some("record")));
    eval("eval assert(not objects[#objects-1]:is_enabled()); assert(objects[#objects]:is_enabled()); assert(user:is_enabled())".into());
    eval(disable("session-0081", None));
    eval("eval assert(not objects[#objects]:is_enabled()); assert(user:is_enabled())".into());
    // Startup must retire older exact handles even when the new configuration
    // selects a permanent/external shortcut and never creates a replacement.
    eval(enable("session-0082", "record", "ALT + S", "record", true));
    eval(enable("session-0082", "paste", "ALT + P", "paste", true));
    eval(disable("session-0083", Some("paste")));
    eval("eval assert(not objects[#objects-1]:is_enabled()); assert(not objects[#objects]:is_enabled()); assert(user:is_enabled())".into());
    let mut child = Command::new(std::env::var_os("APPARENT_LUA_TEST_BIN").expect("Lua interpreter path"))
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    child.stdin.take().unwrap().write_all(script.as_bytes()).unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "Lua contract failed: {}", String::from_utf8_lossy(&output.stderr));
}
