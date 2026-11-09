/** Post-publication policy only. Native owns finalization, receipts and effect admission. */
export type Capture = Readonly<{generation: string; captureId: string; requestId: string | null}>;
export type Outcome = Readonly<{kind: 'saved'; bundlePath: string} | {kind: 'failed'; message: string}>;
export type Observation = Readonly<{
  capture: Capture; outcome: Outcome; clipboard: 'notRequested' | 'copied' | 'failed'; current: boolean;
}>;
export type Context = Readonly<{
  capture: Capture;
  platform: 'linux' | 'macos';
  source: 'screen' | 'phone';
  agentRequested: boolean;
  automaticTrialEnabled: boolean;
  lab: boolean;
  hyprland: boolean;
  wayland: boolean;
}>;
export type State = Readonly<{
  context: Context;
  observation: Observation | null;
  current: boolean;
  connected: boolean;
  copyPending: boolean;
  status: string | null;
  notification: Readonly<{title: string; body: string}> | null;
}>;
export type Event =
  | {type: 'finalized'; capture: Capture; outcome: Outcome}
  | {type: 'clipboard'; capture: Capture; result: 'copied' | 'failed'}
  | {type: 'dismiss' | 'cancel' | 'revoke' | 'disconnect' | 'reconnect'};
export type Effect = Readonly<{type: 'copy'; capture: Capture; mode: 'text' | 'bundle'}>;
export type Decision = Readonly<{state: State; effects: readonly Effect[]}>;

export function begin(context: Context, current: boolean): State {
  return {context, observation: null, current, connected: true, copyPending: false, status: null, notification: null};
}
export function sameCapture(a: Capture, b: Capture): boolean {
  return a.generation === b.generation && a.captureId === b.captureId && a.requestId === b.requestId;
}
function manualTrial(context: Context): boolean {
  return context.platform === 'linux' && context.source === 'screen' && !context.agentRequested
    && context.automaticTrialEnabled && !context.lab && context.hyprland && context.wayland;
}
export function reduce(previous: State, event: Event): Decision {
  let state = previous;
  const effects: Effect[] = [];
  if (event.type === 'dismiss' || event.type === 'cancel' || event.type === 'revoke') {
    state = {...state, current: false, copyPending: false,
      observation: state.observation && {...state.observation, current: false}};
  } else if (event.type === 'disconnect') {
    // A lost effect acknowledgement must never cause an automatic repeat write.
    state = {...state, connected: false, copyPending: false};
  } else if (event.type === 'reconnect') {
    state = {...state, connected: true};
  } else if ('capture' in event && sameCapture(state.context.capture, event.capture)) {
    if (event.type === 'finalized' && !state.observation) {
      const saved = event.outcome.kind === 'saved';
      state = {...state, observation: {capture: state.context.capture, outcome: event.outcome,
        clipboard: 'notRequested', current: state.current},
        copyPending: saved && state.current && state.connected,
        status: event.outcome.kind === 'saved' ? 'Saved' : event.outcome.message.startsWith('No screen frames were captured')
          ? 'Capture failed — no screen frames' : 'Capture failed'};
      if (state.copyPending) effects.push({type: 'copy', capture: state.context.capture,
        mode: manualTrial(state.context) ? 'text' : 'bundle'});
    } else if (event.type === 'clipboard' && state.copyPending && state.observation?.outcome.kind === 'saved') {
      state = {...state, copyPending: false, observation: {...state.observation, clipboard: event.result},
        status: event.result === 'failed' ? 'Saved · open bundle (copy failed)'
          : manualTrial(state.context) ? 'Saved · paste manually' : 'Saved · copied',
        notification: manualTrial(state.context) ? {title: 'Draft paste skipped', body: event.result === 'failed'
          ? 'Could not copy the handoff. Use the saved bundle from the paper tile.'
          : `No verified attachment observer or stable draft identity; use the manual handoff. Press Ctrl+V for the text; attach contact-sheet.png from ${state.observation.outcome.bundlePath} if needed`} : null};
    }
  }
  return {state, effects};
}
