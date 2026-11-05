Synthetic bundle and golden output compatibility fixtures retained byte-for-byte
from the existing Rust inspection work at commit
5707c5a395691bf0781a011375b893a6ee57bc9b, paths
capture/agent/inspection/tests/fixtures/{current,legacy}.json,
{current,legacy}.md and current.marks.json (PR617).
They contain synthetic evidence, not private recordings. This reuses compatibility
inputs without adopting or modifying that unmerged Rust implementation.
