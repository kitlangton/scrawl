import { mkdtempSync, readFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { pathToFileURL } from 'node:url'
import { installUbuntuDependencies, type UbuntuSetup } from './runtime/ubuntu-dependencies.ts'

export const mediaPackages = [
  'ffmpeg', 'pkg-config', 'libgstreamer1.0-dev', 'libgstreamer-plugins-base1.0-dev',
  'gstreamer1.0-plugins-base', 'gstreamer1.0-plugins-good', 'gstreamer1.0-libav',
] as const

// Capture/2 ships X11/PipeWire sources, H264 encoding and the standard PipeWire
// client modules, SPA plugins and client configuration sealed by its producer.
export const capturePackages = [...mediaPackages, 'xvfb', 'x11-xserver-utils', 'libx11-dev', 'gstreamer1.0-x',
  'gstreamer1.0-plugins-bad', 'gstreamer1.0-plugins-ugly', 'gstreamer1.0-pipewire',
  'libpipewire-0.3-modules', 'libspa-0.2-modules', 'pipewire-bin'] as const

export function installCaptureDependencies(setup: UbuntuSetup): void {
  installUbuntuDependencies(setup, capturePackages)
}

export function installMediaDependencies(setup: UbuntuSetup): void {
  installUbuntuDependencies(setup, mediaPackages)
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  // Apt owns files beneath this unique temporary directory; the disposable
  // runner removes them with its other job state. Nothing here is an artifact.
  const directory = mkdtempSync(join(tmpdir(), 'apparent-media-apt-'))
  const install = process.argv.includes('--capture') ? installCaptureDependencies : installMediaDependencies
  install({ directory, osRelease: readFileSync('/etc/os-release', 'utf8'),
    environment: process.env, root: process.getuid?.() === 0 })
}
