// Scrawl dev SDK, served by the Scrawl app at http://127.0.0.1:47777/sdk.js.
// <script src="http://127.0.0.1:47777/sdk.js" data-session="ses_…"></script>
// or data-directory="/abs/project" to use that project's newest OpenCode session.
// Registers this tab with Scrawl while it's open, so a Mac capture of it is sent to that
// session. It does nothing outside localhost unless data-token (the ingest token) is set.
// When Scrawl asks for it (loopback pages only), it also tells Scrawl what's under clicks and
// ⇧ marks (element, component, source file:line), video seeks, console errors and failed
// requests. It never sends request bodies, headers, cookies, query strings or typed values.
(() => {
  const script = document.currentScript;
  if (!script || window.__scrawlSdk) return;
  const host = location.hostname;
  const loopback =
    host === "localhost" ||
    host.endsWith(".localhost") ||
    host === "[::1]" ||
    host === "::1" ||
    /^127\./.test(host);
  const token = script.dataset.token;
  if (!loopback && !token) return;
  window.__scrawlSdk = true;

  const base = new URL(script.src).origin;
  const tab = Math.random().toString(36).slice(2, 14);
  const fetchOriginal = window.fetch.bind(window);
  // text/plain keeps loopback registrations simple requests (no preflight).
  const headers = token
    ? { "Content-Type": "application/json", Authorization: "Bearer " + token }
    : { "Content-Type": "text/plain" };
  const post = (path, body, keepalive) =>
    fetchOriginal(base + path, {
      method: "POST",
      mode: "cors",
      keepalive,
      headers,
      body: JSON.stringify(body),
    })
      .then((r) => r.json())
      .catch(() => null);
  const send = (path, keepalive) =>
    post(
      path,
      {
        origin: location.origin,
        url: location.href,
        title: document.title,
        session: script.dataset.session || undefined,
        directory: script.dataset.directory || undefined,
        tab,
      },
      keepalive,
    ).then((reply) => {
      if (reply && reply.context && loopback) watch();
    });

  const beat = () => {
    if (document.visibilityState !== "hidden") send("/web/register");
  };
  beat();
  setInterval(beat, 30000);
  document.addEventListener("visibilitychange", beat);
  addEventListener("pageshow", beat);
  addEventListener("pagehide", () => send("/web/unregister", true));

  // What the page sees, batched to /web/context.
  let watching = false;
  function watch() {
    if (watching) return;
    watching = true;
    const queue = [];
    let timer = 0;
    const flush = () => {
      timer = 0;
      if (!queue.length) return;
      const events = queue.splice(0, 64);
      post("/web/context", { tab, events }, true);
      if (queue.length) timer = setTimeout(flush, 300);
    };
    const push = (event) => {
      event.t = Date.now();
      event.focused = document.hasFocus();
      queue.push(event);
      if (queue.length > 200) queue.splice(0, queue.length - 200);
      if (!timer) timer = setTimeout(flush, 400);
    };
    const clip = (text, max) => {
      const s = String(text == null ? "" : text).replace(/\s+/g, " ").trim();
      return s ? s.slice(0, max) : undefined;
    };
    const ignored = /\/node_modules\/|\/\.vite\/deps\/|react-dom|jsx-dev-runtime|jsx-runtime|\/sdk\.js/;
    // `src/ui/Pill.tsx:142` from the first stack frame in the project's own code.
    const stackSource = (stack) => {
      for (const line of String(stack || "").split("\n")) {
        const m = line.match(/((?:https?|file):\/\/[^\s()]+?):(\d+):\d+\)?\s*$/);
        if (!m || ignored.test(m[1])) continue;
        try {
          const path = decodeURIComponent(new URL(m[1]).pathname).replace(/^\/(@fs\/)?/, "");
          return path + ":" + m[2];
        } catch {}
      }
      return undefined;
    };
    const short = (file, line) => {
      try {
        return decodeURIComponent(new URL(file, location.href).pathname).replace(/^\//, "") + (line ? ":" + line : "");
      } catch {
        return undefined;
      }
    };
    // Source locators some dev setups add (solid-devtools, react-dev-inspector, Sentry).
    const located = (el) => {
      const d = el.dataset || {};
      if (d.sourceLoc) return { source: d.sourceLoc.replace(/^(.*:\d+):\d+$/, "$1") };
      if (d.inspectorRelativePath) return { source: d.inspectorRelativePath + (d.inspectorLine ? ":" + d.inspectorLine : "") };
      if (d.sentrySourceFile) return { component: d.sentryComponent, source: d.sentrySourceFile };
      return null;
    };
    const fiberOf = (el) => {
      for (const key in el) {
        if (key.startsWith("__reactFiber$") || key.startsWith("__reactInternalInstance$")) return el[key];
      }
      return null;
    };
    const reactInfo = (fiber) => {
      const out = {};
      const src = fiber._debugSource;
      if (src && src.fileName) out.source = short(src.fileName, src.lineNumber);
      else if (fiber._debugStack) out.source = stackSource(fiber._debugStack.stack);
      for (let f = fiber; f; f = f.return) {
        const type = f.type;
        const fn = typeof type === "function" ? type : type && typeof type === "object" ? type.render || type.type : null;
        const name = fn && (fn.displayName || fn.name);
        if (name) {
          out.component = name;
          break;
        }
      }
      return out;
    };
    const component = (el) => {
      for (let node = el, depth = 0; node && depth < 30; node = node.parentElement, depth++) {
        const found = located(node);
        if (found) return found;
        const fiber = fiberOf(node);
        if (fiber) return reactInfo(fiber);
      }
      return {};
    };
    const typed = "input, textarea, select, [contenteditable=''], [contenteditable='true']";
    const describe = (el) => {
      if (!(el instanceof Element)) return undefined;
      el = el.closest("button, a, [role=button], [role=tab], [role=menuitem], [role=link], label, summary") || el;
      const target = { tag: el.tagName.toLowerCase() };
      if (el.id) target.id = clip(el.id, 60);
      const cls = [...el.classList].filter((c) => c.length <= 30 && !/[:[\]/]/.test(c)).slice(0, 2);
      if (cls.length) target.cls = cls.join(".");
      const field = el.closest(typed);
      let text =
        el.getAttribute("aria-label") || el.getAttribute("title") || el.getAttribute("alt") || el.getAttribute("data-testid");
      if (!text && field) {
        if (field.getAttribute("type") !== "password") text = field.getAttribute("placeholder") || field.getAttribute("name");
      } else if (!text) {
        text = (el.textContent || "").slice(0, 200);
      }
      const label = clip(text, 60);
      if (label) target.text = label;
      return Object.assign(target, component(el));
    };

    addEventListener("pointerdown", (e) => push({ kind: "pointer", target: describe(e.target) }), {
      capture: true,
      passive: true,
    });
    let lastMark = 0;
    addEventListener(
      "pointermove",
      (e) => {
        if (!e.shiftKey || e.buttons) return;
        const now = Date.now();
        if (now - lastMark < 250) return;
        lastMark = now;
        push({ kind: "mark", target: describe(document.elementFromPoint(e.clientX, e.clientY)) });
      },
      { capture: true, passive: true },
    );

    for (const type of ["seeked", "play", "pause"]) {
      document.addEventListener(
        type,
        (e) => {
          const m = e.target;
          if (!(m instanceof HTMLMediaElement)) return;
          const src = (m.currentSrc || m.src || "").split(/[?#]/)[0].split("/").pop();
          push({
            kind: "media",
            event: type,
            media: { src: clip(src, 60) || "", time: m.currentTime, duration: m.duration || 0, paused: m.paused },
          });
        },
        true,
      );
    }

    const describeValue = (value) =>
      value instanceof Error ? value.message : typeof value === "string" ? value : value && value.constructor ? "[" + value.constructor.name + "]" : String(value);
    const consoleError = console.error;
    console.error = function (...args) {
      try {
        const error = args.find((a) => a instanceof Error);
        push({
          kind: "error",
          message: clip(args.map(describeValue).join(" "), 200),
          source: stackSource(error ? error.stack : new Error().stack),
        });
      } catch {}
      return consoleError.apply(this, args);
    };
    addEventListener("error", (e) => {
      if (!e.message) return;
      push({
        kind: "error",
        message: clip(/^Uncaught/.test(e.message) ? e.message : "Uncaught " + e.message, 200),
        source: (e.error && stackSource(e.error.stack)) || (e.filename ? short(e.filename, e.lineno) : undefined),
      });
    });
    addEventListener("unhandledrejection", (e) =>
      push({
        kind: "error",
        message: clip("Unhandled rejection: " + describeValue(e.reason), 200),
        source: e.reason instanceof Error ? stackSource(e.reason.stack) : undefined,
      }),
    );

    // Failed requests: method, path (no query) and status; never bodies or headers.
    const pathOf = (url) => {
      try {
        const u = new URL(url, location.href);
        if (u.origin === base) return null;
        return (u.origin === location.origin ? "" : u.host) + u.pathname;
      } catch {
        return null;
      }
    };
    const failed = (method, url, status, started) => {
      const path = pathOf(url);
      if (path) push({ kind: "request", method, path, status, ms: Math.round(performance.now() - started) });
    };
    window.fetch = function (input, init) {
      const started = performance.now();
      const method = String((init && init.method) || (input && input.method) || "GET").toUpperCase();
      const url = typeof input === "string" ? input : input && (input.url || String(input));
      const pending = fetchOriginal(input, init);
      pending.then(
        (r) => {
          if (!r.ok) failed(method, r.url || url, r.status, started);
        },
        () => failed(method, url, 0, started),
      );
      return pending;
    };
    const open = XMLHttpRequest.prototype.open;
    const xhrSend = XMLHttpRequest.prototype.send;
    XMLHttpRequest.prototype.open = function (method, url, ...rest) {
      this.__scrawl = { method: String(method || "GET").toUpperCase(), url: String(url) };
      return open.call(this, method, url, ...rest);
    };
    XMLHttpRequest.prototype.send = function (...args) {
      const info = this.__scrawl;
      if (info) {
        const started = performance.now();
        this.addEventListener("loadend", () => {
          if (this.status === 0 || this.status >= 400) failed(info.method, info.url, this.status, started);
        });
      }
      return xhrSend.apply(this, args);
    };
  }
})();
