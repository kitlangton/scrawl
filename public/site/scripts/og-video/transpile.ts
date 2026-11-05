import { transform } from "esbuild"

/** Compile stage scripts for classic-script loading in the capture tool and browser fixture. */
export async function transpileOgVideoScript(source: string): Promise<string> {
  return (await transform(source, { loader: "ts", format: "iife" })).code
}
