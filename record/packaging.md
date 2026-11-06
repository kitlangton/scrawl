# Record package lifecycle

`npm ci` retains the source-author `prepare` build. That source checkout
build (`npm run build`) uses `shared/ui` and its shared font assets, plus the
Record TypeScript, Tailwind and UI build tools; this is an author/build-machine
requirement and does not involve Rust. `npm pack` runs `prepack`, which builds
the complete artifact, including its precompiled UI stylesheet and font.

Consumers install the resulting tarball with ordinary `npm install`; this does
not run the dependency’s source-author `prepare` lifecycle. The packed
package includes built `dist` files, so install and headless development need no
repository siblings or build tooling. React and React DOM remain peers for the
`/ui` export and are marked optional so root, `/host`, `/processing` and
`/capabilities` consumers do not acquire them. UI consumers install compatible
React peers themselves. The package smoke test builds first, then checks a
headless tarball consumer and a separate UI consumer with explicitly installed
React peers.
