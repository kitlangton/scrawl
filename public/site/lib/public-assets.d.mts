import type { Plugin } from "vite"

export interface PublicAssetEntry {
  sourcePath: string
  servedPath: string
  contentType: string
}

export function publicAssets(assetRoot: string): PublicAssetEntry[]
export function publicAssetPlugin(assetRoot: string): Plugin
export function readPublicAsset(entry: PublicAssetEntry): Uint8Array
