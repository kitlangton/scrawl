type StateBridge<State> = {
  readState: () => Promise<State | null>;
  onState: (receive: (state: State) => void) => () => void;
};

// Live state wins over the snapshot requested during renderer startup. That
// request can finish after a newer screen or after this subscription is gone.
export function subscribeOnboardingState<State>(bridge: StateBridge<State>, receive: (state: State | null) => void) {
  let active = true, receivedLive = false;
  const stop = bridge.onState(state => {
    if (!active) return;
    receivedLive = true;
    receive(state);
  });
  void bridge.readState().then(state => {
    if (active && !receivedLive) receive(state);
  }, () => { /* A subsequent live state can still establish the connection. */ });
  return () => { active = false; stop(); };
}
