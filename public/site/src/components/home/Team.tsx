const TEAM_ASSETS = "/team"

export function Team() {
  return (
    <section className="pond-team" id="team" aria-labelledby="team-heading">
      <div className="pond-team-inner">
        <div className="pond-team-heading">
          <span>The team</span>
          <h2 id="team-heading">Built for the problem.</h2>
        </div>
        <div className="pond-team-grid">
          <article className="pond-team-member pond-team-olivia">
            <a className="pond-team-profile" href="https://www.linkedin.com/in/olivia-allen-busin/" target="_blank" rel="noopener noreferrer" aria-label="Olivia on LinkedIn (opens in a new tab)">
              <img src={`${TEAM_ASSETS}/olivia.png`} alt="Olivia, CEO" loading="lazy" width="667" height="477" />
              <h3>Olivia <span>CEO</span></h3>
            </a>
            <div className="pond-team-proof">
              <div>
                <a href="https://start-berlin.com" target="_blank" rel="noopener noreferrer" aria-label="START Berlin (opens in a new tab)"><img src={`${TEAM_ASSETS}/start-berlin.png`} alt="START Berlin" loading="lazy" /></a>
                <p><strong>Head of Growth</strong><br />#1 · growth + engagement<br />20 chapters</p>
              </div>
              <div>
                <a href="https://plugandplaytechcenter.com" target="_blank" rel="noopener noreferrer" aria-label="Plug and Play (opens in a new tab)"><img src={`${TEAM_ASSETS}/plug-and-play.png`} alt="Plug and Play" loading="lazy" /></a>
                <p><strong>VC + Fundraising</strong><br />experience</p>
              </div>
            </div>
          </article>
          <article className="pond-team-member pond-team-simon">
            <a className="pond-team-profile" href="https://www.linkedin.com/in/simon-van-laak/" target="_blank" rel="noopener noreferrer" aria-label="Simon on LinkedIn (opens in a new tab)">
              <img src={`${TEAM_ASSETS}/simon.png`} alt="Simon, CTO" loading="lazy" width="559" height="488" />
              <h3>Simon <span>CTO</span></h3>
            </a>
            <div className="pond-team-proof">
              <div>
                <a href="https://imago-images.com" target="_blank" rel="noopener noreferrer" aria-label="IMAGO (opens in a new tab)"><img src={`${TEAM_ASSETS}/imago.png`} alt="IMAGO" loading="lazy" /></a>
                <p><strong>Lead Software Engineer</strong><br />10,000+ images / day</p>
              </div>
              <div>
                <a href="https://openclaw.ai" target="_blank" rel="noopener noreferrer" aria-label="OpenClaw (opens in a new tab)"><img src={`${TEAM_ASSETS}/openclaw.svg`} alt="OpenClaw" loading="lazy" /></a>
                <p><strong>Open Source</strong><br />contributor</p>
              </div>
            </div>
          </article>
          <div className="pond-team-history">
            <div><span>Part of</span><a href="https://engine.hpi.de" target="_blank" rel="noopener noreferrer" aria-label="HPI Engine (opens in a new tab)"><img src={`${TEAM_ASSETS}/hpi-engine.svg`} alt="HPI Engine" loading="lazy" /></a></div>
            <div><span>Educated at</span><a href="https://code.berlin" target="_blank" rel="noopener noreferrer" aria-label="CODE (opens in a new tab)"><img src={`${TEAM_ASSETS}/code-wordmark.svg`} alt="CODE" loading="lazy" /></a></div>
          </div>
        </div>
      </div>
    </section>
  )
}
