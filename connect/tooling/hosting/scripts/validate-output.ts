import { lstat, readdir } from 'node:fs/promises'
import { resolve } from 'node:path'
import { pathToFileURL } from 'node:url'

/** The directory artifact must contain deployment bytes only, never test state. */
export async function validateOutput(root: string): Promise<void> {
  const files = new Set<string>()
  const directories = new Set(['dist', 'dist/connect', 'dist/connect/assets', 'dist/connect/device', 'dist/connect/shares'])
  const required = [
    'worker.js', 'wrangler.json', 'dist/connect/index.html',
    'dist/connect/device/index.html', 'dist/connect/shares/index.html',
    ...['HostGrotesk-Variable.ttf', 'apparent-wordmark.svg', 'source-pond-v4.png', 'agent-pond-v4.png', 'agent-reading-v4.png'].map(name => `dist/connect/assets/${name}`),
  ]
  async function walk(path = ''): Promise<void> {
    for (const name of await readdir(resolve(root, path))) {
      const relative = path ? `${path}/${name}` : name
      const stat = await lstat(resolve(root, relative))
      if (stat.isSymbolicLink()) throw new Error(`Unexpected artifact link: ${relative}`)
      if (stat.isDirectory()) {
        if (!directories.has(relative)) throw new Error(`Unexpected artifact directory: ${relative}`)
        await walk(relative)
      } else if (stat.isFile() && (required.includes(relative) || /^dist\/connect\/assets\/[^/.]+-[\w-]+\.(?:js|css)$/.test(relative))) files.add(relative)
      else throw new Error(`Unexpected artifact entry: ${relative}`)
    }
  }
  await walk()
  for (const path of required) if (!files.has(path)) throw new Error(`Missing artifact entry: ${path}`)
  for (const extension of ['js', 'css']) if ([...files].filter(path => path.startsWith('dist/') && path.endsWith(`.${extension}`)).length !== 1) throw new Error(`Expected exactly one browser ${extension} asset`)
}
if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  await validateOutput(resolve(import.meta.dirname, '../.output'))
}
