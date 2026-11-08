import type { App } from 'electron';

export type Failure = 'main_exception' | 'main_rejection' | 'renderer_gone' | 'child_gone';
type Host = Pick<NodeJS.Process, 'on' | 'env'>;
const abnormal = new Set(['crashed', 'oom', 'abnormal-exit', 'launch-failed', 'integrity-failure']);

// Only fixed codes leave this owner. Never inspect Error, rejection, URL,
// process-name or crash-detail payloads. Rust owns consent and network delivery.
export function install(app: App, report: (kind: Failure) => Promise<unknown>, host: Host = process): (code: number) => void {
  if (host.env.APPARENT_DESKTOP_REPORTING !== '1') return code => app.exit(code);
  let dying = false;
  let exited = false;
  let pending = 0;
  let quitRequested = false;
  let quitReleased = false;
  let quitTimer: ReturnType<typeof setTimeout> | undefined;
  let exitTimer: ReturnType<typeof setTimeout> | undefined;
  let exitRequested: number | undefined;
  const exitNow = (code: number) => {
    if (exited) return;
    exited = true;
    clearTimeout(exitTimer);
    clearTimeout(quitTimer);
    app.exit(code);
  };
  const exitWhenAdmitted = (code: number) => {
    if (exited || exitRequested !== undefined) return;
    if (pending === 0) return exitNow(code);
    exitRequested = code;
    clearTimeout(quitTimer);
    exitTimer = setTimeout(() => exitNow(code), 600);
  };
  const quit = () => {
    clearTimeout(quitTimer);
    if (quitReleased || dying || exitRequested !== undefined) return;
    quitReleased = true;
    app.quit();
  };
  const send = (kind: Failure) => {
    pending += 1;
    return Promise.resolve().then(() => report(kind)).catch(() => {}).finally(() => {
      pending -= 1;
      if (pending === 0) {
        if (exitRequested !== undefined) exitNow(exitRequested);
        else if (quitRequested) quit();
      }
    });
  };
  // Notes quits when its renderer dies. Hold that existing quit only long
  // enough to attempt admission; this is not a delivery guarantee.
  app.on('before-quit', event => {
    if (quitReleased || pending === 0) return;
    event.preventDefault();
    if (!quitRequested) {
      quitRequested = true;
      quitTimer = setTimeout(quit, 600);
    }
  });
  const fatal = (kind: Failure) => {
    if (dying) return;
    dying = true;
    clearTimeout(quitTimer);
    void send(kind);
    exitWhenAdmitted(1);
  };
  host.on('uncaughtException', () => fatal('main_exception'));
  host.on('unhandledRejection', () => fatal('main_rejection'));
  app.on('web-contents-created', (_event, contents) => {
    contents.on('render-process-gone', (_event, details) => {
      if (abnormal.has(details.reason)) void send('renderer_gone');
    });
  });
  app.on('child-process-gone', (_event, details) => {
    if (abnormal.has(details.reason)) void send('child_gone');
  });
  return exitWhenAdmitted;
}
