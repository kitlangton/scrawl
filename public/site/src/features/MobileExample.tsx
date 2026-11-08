import "./mobile-example.css"

const GHOST = "/feature-demos/how-it-works/.local-media/ghost-after-aligned.png"

/** Small-screen explanations use readable semantic content, not scaled desktop chrome. */
export function MobileExample({ path }: { path: string }) {
  return <figure className={`feature-mobile mobile-${path}`}>
    <figcaption>Illustrative example</figcaption>
    {path === "how-it-works" && <>
      <p className="example-label">Draw the change</p>
      <div className="example-board"><svg viewBox="0 0 280 190" aria-label="A sketch of a ghost with large eyes" role="img"><path d="M62 164 L64 70 Q60 20 131 20 Q203 16 210 70 L216 166 L191 148 L169 169 L143 149 L117 170 L92 148 Z M98 65 C74 66 76 110 98 110 C122 108 120 65 98 65 M167 63 C144 63 145 108 167 108 C190 107 191 63 167 63 M112 128 Q138 151 164 126" /></svg></div>
      <p className="example-note">“Make the eyes bigger, like this.”</p>
    </>}
    {path === "timeline" && <>
      <p className="example-label">One connected timeline</p>
      <ol className="example-timeline">
        <li><time>00:03</time><span>Point to the ghost</span></li>
        <li><time>00:08</time><span>“Make the eyes bigger”</span></li>
        <li><time>00:13</time><span>Sketch the new shape</span></li>
      </ol>
      <p className="example-note">Voice, clicks and sketches stay together.</p>
    </>}
    {path === "handoff" && <>
      <p className="example-label">Ready for your agent</p>
      <div className="example-package"><span>feedback.apparent</span><ul><li>Recording and images</li><li>Timestamped transcript</li><li>Clicks and sketches</li></ul></div>
      <p className="example-note">Stop recording. Paste the references into your agent.</p>
    </>}
    {path === "anywhere" && <>
      <p className="example-label">See the result</p>
      <div className="example-result"><img src={GHOST} width={180} height={180} alt="The updated ghost with large hollow eyes" loading="lazy" /></div>
      <p className="example-note">The agent returns a walkthrough of what changed.</p>
    </>}
  </figure>
}
