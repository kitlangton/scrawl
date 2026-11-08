import { posix } from 'node:path'

/** The published Electron development .app contains these framework aliases.
 * Keep the exception bound to that owner/target/layout, not arbitrary tar links.
 * Never extract here or follow a filesystem link from an artifact. */
export function validateArchiveKinds(names: string[], entries: string[], id: string, target: string, stage: string) {
  const fail = (): never => { throw new Error('Component archive contains links or unsupported filesystem entries') }
  if (entries.length !== names.length || entries.some(line => !/^[dl-][rwxStTs-]{9}/.test(line))) fail()
  const kinds = new Map(names.map((name, i) => [name.replace(/\/$/, ''), entries[i][0]]))
  const links = [...kinds].filter(([, kind]) => kind === 'l')
  if (!links.length) return
  if (id !== 'app-shell' || target !== 'macos-x64' || stage !== 'development-app') fail()
  if (names.some(name => /[^\x20-\x7e]/.test(name) || name.length > 4096 || name.split('/').length > 64)) fail()
  // Default Mac filesystems can fold case/Unicode. Keep this fixed-layout
  // exception ASCII and refuse collisions and alias traversal after case-folding.
  const folded = new Map([...kinds].map(([name, kind]) => [name.toLowerCase(), kind]))
  if (folded.size !== kinds.size) fail()
  const prefix = 'stream-payload/bundle/apparent-foundation-darwin-x64/apparent-foundation.app/Contents/Frameworks/'
  const allowed = new Map<string, { target: string; endpoint: string; kind: string; current?: string }>()
  for (const framework of ['Electron Framework', 'Mantle', 'ReactiveObjC', 'Squirrel']) {
    const base = prefix + framework + '.framework/'
    const current = base + 'Versions/Current'
    allowed.set(current, { target: 'A', endpoint: base + 'Versions/A', kind: 'd' })
    for (const alias of [framework, 'Resources', ...(framework === 'Electron Framework' ? ['Libraries', 'Helpers'] : [])]) {
      allowed.set(base + alias, { target: 'Versions/Current/' + alias,
        endpoint: base + 'Versions/A/' + alias, kind: alias === framework ? '-' : 'd', current })
    }
  }
  for (const [name] of links) {
    const rule = allowed.get(name) ?? fail()
    const line = entries[names.indexOf(name)]
    // Match the complete known alias and literal relative target. Unknown,
    // absolute, escaping, cyclic and alternate-target links cannot enter here.
    if (!line.endsWith(' ' + name + ' -> ' + rule.target)) fail()
    if (kinds.get(rule.endpoint) !== rule.kind || (rule.current && kinds.get(rule.current) !== 'l')) fail()
  }
  // A tar extraction must never traverse an alias (or file), even if a later
  // entry would otherwise replace data inside its declared target directory.
  for (const name of kinds.keys()) {
    let parent = posix.dirname(name)
    while (parent !== '.') {
      const kind = folded.get(parent.toLowerCase())
      if (kind && kind !== 'd') fail()
      parent = posix.dirname(parent)
    }
  }
}
