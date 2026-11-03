/** The accepted statement and photos render with the page, without runtime overlays. */
export function PhotoStory() {
  return <>
    <section className="preview-statement" id="statement-preview" aria-label="What we believe">
      <h2 id="belief">We believe <span className="word work">work<svg aria-hidden="true" viewBox="0 0 240 120" preserveAspectRatio="none"><path className="work-circle" pathLength="1" d="M224 68 C232 102 177 116 113 109 C43 116 5 93 12 55 C10 17 63 7 127 12 C192 8 231 27 224 68 C224 78 219 85 211 89" /></svg></span> should be <span className="word enjoyable"><svg aria-hidden="true" viewBox="0 0 240 100" preserveAspectRatio="none"><path className="enjoy-highlight" pathLength="1" d="M8 60 C65 50 123 66 232 55" /></svg><span>enjoyable</span></span> and leave us free for late nights with friends.</h2>
    </section>
    <section className="photo-story" id="photo-preview" aria-label="Dancing and cuddling with dogs">
      <div className="cutout-composition">
        <div className="cutout-person dancing-cutout"><img src="/pond/simon-cutout.png" alt="Simon dancing, with the background removed" /><span className="cutout-ink">Simon dancing</span><svg viewBox="0 0 100 150" aria-hidden="true"><path d="M15 135 Q12 65 73 22 M54 26 L76 18 L74 42" /></svg></div>
        <div className="cutout-person dog-cutout"><img src="/pond/olivia-dog-cutout.png" alt="Olivia cuddling her dog, with her clothes preserved and the background removed" /><span className="cutout-ink">Olivia cuddling<br />a dog</span><svg viewBox="0 0 120 100" aria-hidden="true"><path d="M20 15 Q78 5 85 74 M70 60 L86 78 L96 59" /></svg></div>
      </div>
      <div className="photo-caption"><span>us having fun</span><svg viewBox="0 0 260 60" aria-hidden="true"><path d="M18 39 C80 43 163 40 236 38" /></svg></div>
    </section>
  </>
}
