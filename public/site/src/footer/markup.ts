// Accepted paper-above / theatre-below composition. No prototype code or private media.
export const footerMarkup = `
<section class="footer-scene" aria-labelledby="footer-heading">
  <img class="footer-art" src="/pond/footer-theatre.webp" width="1672" height="941" alt="" loading="lazy" decoding="async">
  <div class="footer-invite">
    <p class="footer-greeting">Hey, fellow bottom-of-the-page person.</p>
    <h2 id="footer-heading">We made this.<br>Show us what you can do.</h2>
    <p class="footer-body">Try <img src="/apparent-wordmark.svg" width="104" height="24" alt="Apparent"> on this page. Show us what you love or what you’d change.</p>
    <div class="footer-actions">
      <button type="button" class="footer-try" aria-live="polite"><span class="try-label">Try</span><img src="/apparent-wordmark.svg" alt="apparent"><span class="try-arrow" aria-hidden="true">↗</span></button>
      <button type="button" class="footer-hi" data-open="social">Or say hi on LinkedIn ↗</button>
    </div>
  </div>
</section>
<div class="footer-base">
  <a class="footer-brand" href="#top" aria-label="Apparent home"><img src="/apparent-wordmark.svg" width="132" height="31" alt="apparent"></a>
  <div class="footer-credit"><span>Made</span><button type="button" class="footer-wheel" aria-label="Made with love in Berlin. Pause rotation" aria-pressed="false"><span>with love</span></button><span>in Berlin.</span></div>
  <a class="footer-github" href="https://github.com/tryapparent/apparent">GitHub ↗</a>
</div>
<dialog class="footer-dialog" data-dialog="social" aria-labelledby="footer-social-title">
  <button type="button" class="footer-close" aria-label="Close">×</button>
  <h2 id="footer-social-title">Say hi to us.</h2>
  <div class="social-names"><p class="olivia-speech"><a href="https://www.linkedin.com/in/olivia-allen-busin/" target="_blank" rel="noopener noreferrer">Olivia</a><span class="rude-bubble" hidden role="status"><svg viewBox="0 0 130 64" aria-hidden="true"><path d="M30 15 L3 9 Q10 20 20 27 C11 42 30 56 62 56 C105 59 139 39 118 18 C101 3 56 2 30 15 Z"/></svg><span>rude</span></span></p>
  <p><a href="https://www.linkedin.com/in/simon-van-laak/" target="_blank" rel="noopener noreferrer">Simon</a></p></div>
</dialog>`
