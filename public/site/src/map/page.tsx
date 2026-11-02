import { renderToStaticMarkup } from "react-dom/server"
import type { Audit } from "../../scripts/site-build/feature-audit.ts"
import { FeatureMap } from "./FeatureMap"
import css from "./map.css?raw"
import theme from "../../../../shared/ui/src/tokens.css?raw"

// A whole static document: no client script, kept out of search engines.
export function mapPage(audit: Audit): string {
  return `<!doctype html>
<html lang="en">
  <head>
    <meta charset="UTF-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1.0, viewport-fit=cover" />
    <meta name="robots" content="noindex,nofollow" />
    <meta name="color-scheme" content="light dark" />
    <meta name="theme-color" content="#0a0a0b" />
    <title>Apparent feature map</title>
    <link rel="icon" href="/favicon.ico" sizes="32x32" />
    <style>${(theme + css.replace(/^@import[^;]+;/, "")).replaceAll("</style", "<\\/style")}</style>
  </head>
  <body>${renderToStaticMarkup(<FeatureMap audit={audit} />)}</body>
</html>
`
}
