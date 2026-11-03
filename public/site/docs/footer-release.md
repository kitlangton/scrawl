# Homepage feedback footer

The accepted footer is a compact invitation beneath the photo cutouts:
“We made this. Show us what you can do.” It has the shared Try apparent button,
a LinkedIn chooser, the wordmark and GitHub link. The old theatre artwork and
rotating credit are not visible in this accepted design.

`src/footer/Footer.tsx`, markup and lifecycle-scoped behavior own the implementation.
The social chooser displays Olivia and Simon side by side. The first Simon click
shows the accepted speech bubble; the next follows his profile. Closing resets
that state and returns focus to the opener. No feedback is recorded or sent.

The primary button first attempts `apparent://open`. Before that navigation, its text changes to “Not installed yet? No worries—click again to download ↗”. The next click activates the existing hero download link, retaining its platform/CPU selection, latest-release lookup, busy guard and release-list fallback. Without a supported hero download, it opens the Mac/Linux release chooser. There is no installation detection, timer or automatic download. The label remains in its download state until the page reloads, including after a custom-element reconnect. The footer retains its accepted lilac rounded style; the hero retains its own accepted download treatment.

Opening the app requires an updated installed app with native URL-handler support. Older installs or a rejected browser prompt leave the explicit download action available; a website cannot verify that launch succeeded. Website tests prove the requested URL and download routing, not native launch on macOS or Linux. Opening must not start capture, upload or send feedback. The LinkedIn chooser retains its native dialog, Escape, focus containment and focus return.

## Verification and publication

Build with `APPARENT_PUBLIC_SITE=1 npm run build`, package with
`node scripts/site-build/package-apparent.mjs`, then run
`node --test scripts/pages/test-public-homepage.ts` for desktop/mobile appearance,
frog interactions, feature frames and social dialog focus/reset. Screenshots stay
outside served content through `SITE_EVIDENCE`.

`SITE_ARTIFACT=/absolute/artifact SITE_EVIDENCE=/private/evidence npm run test:try-button` exercises the two-click flow at 320/390/768/1280px, with mocked Mac ARM and Linux x86/ARM browser hints, unknown CPUs, unsupported platforms, failed release API, keyboard activation and custom-element reconnect. It observes the exact protocol navigation and asserts no release request after the first click, then downloads only a clearly labeled text fixture on the second. It never opens a real app or fetches a real installer.

The headless test uses real mouse/keyboard input for launch and a DOM-dispatched follow-up click for download routing: browser-owned external-protocol prompts can block later automated input. It does not certify native consent/dismissal. Use a desktop browser to verify both physical clicks and the native prompt before release.

The accepted footer, photo annotations and social dialog now ship directly from
source. Build and package the website normally, then run the source-homepage
browser regression and the Try button scenarios. No compiled-bundle patches or
baseline overlays are supported. Main-branch Pages CD publishes the verified
artifact; rollback selects a previously accepted Pages deployment (`pages-deploy.md`).
