Synthetic bundle and golden output compatibility fixtures retained byte-for-byte
from the existing Rust inspection work at commit
5707c5a395691bf0781a011375b893a6ee57bc9b, paths
capture/agent/inspection/tests/fixtures/{current,legacy}.json,
{current,legacy}.md and current.marks.json (PR617).
They contain synthetic evidence, not private recordings. This reuses compatibility
inputs without adopting or modifying that unmerged Rust implementation.

`semantic.json` is a synthetic producer-schema fixture covering timed words,
marks, repeated interactions and context changes. `semantic.md`,
`semantic.agent.md` and `current.agent.md` were generated with that same Rust
inspection implementation, using a synthetic silent PCM16 WAV where required.
Only the temporary absolute bundle path was replaced with `@BUNDLE@`.
