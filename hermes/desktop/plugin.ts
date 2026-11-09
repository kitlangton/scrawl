import { host, COMPOSER_AREAS, PALETTE_AREA, Button, Codicon, atom, useValue } from '@hermes/plugin-sdk';
import { jsx } from 'react/jsx-runtime';
import { launchURL, launchCommand, keyCommand, submission, validSource, validToken, mailboxId, sign, authenticatedState, runtime, sameRuntime, type Pending } from './controller.ts';
export default {
  id: 'apparent', name: 'Apparent',
  register(ctx) {
    let disposed = false;
    let working = false;
    let pending: Pending | null = null;
    let bridgeKey: { token: string; value: string } | undefined;
    let timer: (() => void) | undefined;
    const hasPending = atom(false);
    const save = () => { hasPending.set(Boolean(pending)); return ctx.storage.set('recording', pending); };
    const report = (message: string) => { if (!disposed) host.notify({ kind: 'error', message }); };
    ctx.onDispose(() => { disposed = true; timer?.(); });
    async function signingKey(operation: Pending) {
      if (bridgeKey?.token === operation.token) return bridgeKey.value;
      if (!operation.destination.local) throw new Error('Use Apparent directly and paste the recording into this remote conversation.');
      const route = (await host.profileRoutes()).find(r => r.connectionId === operation.destination.connectionId && r.profile === operation.destination.profile);
      if (!route || route.mode !== operation.destination.mode || route.targetProfile !== operation.destination.targetProfile) throw new Error('The original Hermes gateway mapping changed. Nothing was sent.');
      const release = await host.retainProfile(route, { spawnPriority: 'foreground' });
      try {
        if (disposed || pending !== operation) throw new Error('Handoff is no longer active');
        if (!sameRuntime(operation.runtime, runtime(await host.requestProfile(route, 'runtime.describe', {})))) throw new Error('The original Hermes runtime changed. Nothing was sent.');
        const snapshot = await host.requestProfile(route, 'session.resume', { session_id: operation.destination.session });
        if (snapshot?.session_id !== operation.destination.session || snapshot.authority_epoch !== operation.runtime.authority_epoch) throw new Error('The original session changed. Nothing was sent.');
        if (disposed || pending !== operation) throw new Error('Handoff is no longer active');
        const result = await host.requestProfile(route, 'shell.exec', { session_id: operation.destination.session, command: keyCommand(operation.token) });
        const key = result?.stdout?.trim();
        if (result?.code !== 0 || !validToken(key) || key === operation.token) throw new Error('Apparent recording bridge is unavailable');
        if (disposed || pending !== operation) throw new Error('Handoff is no longer active');
        bridgeKey = { token: operation.token, value: key };
        return key;
      } finally { release(); }
    }
    async function local(path: 'state' | 'ack', operation: Pending) {
      const key = await signingKey(operation);
      const id = await mailboxId(operation.token);
      const suffix = `${path}/${await sign(key, `${path}:${id}`)}`;
      const response = await fetch(`http://127.0.0.1:${operation.port}/${id}/${suffix}`, {
        method: path === 'ack' ? 'POST' : 'GET', cache: 'no-store', credentials: 'omit', redirect: 'error', signal: AbortSignal.timeout(3000),
      });
      if (!response.ok) throw new Error('Apparent recording bridge is unavailable');
      const reader = response.body?.getReader();
      if (!reader) throw new Error('Empty Apparent response');
      const chunks: Uint8Array[] = []; let size = 0;
      while (true) { const { done, value } = await reader.read(); if (done) break; size += value.length;
        if (size > 32768) { await reader.cancel(); throw new Error('Oversized Apparent response'); } chunks.push(value); }
      const bytes = new Uint8Array(size); let offset = 0; for (const chunk of chunks) { bytes.set(chunk, offset); offset += chunk.length; }
      const packet = JSON.parse(new TextDecoder().decode(bytes));
      return path === 'state' ? authenticatedState(key, packet) : packet;
    }
    async function poll() {
      if (disposed || working || !pending) return;
      working = true;
      const operation = pending;
      let releaseRoute = () => {};
      try {
        if (Date.now() - operation.created > 60 * 60 * 1000 || Date.now() < operation.created) {
          pending = null; await save(); throw new Error('Recording handoff expired. Your capture remains in Apparent; nothing was sent again.');
        }
        let state;
        try { state = await local('state', operation); }
        catch (error) {
          if (operation.source) state = { phase: 'saved', source: operation.source };
          else if (Date.now() - operation.created < 30000) return;
          else { pending = null; await save(); throw error; }
        }
        if (disposed || pending !== operation) return;
        if (state.phase === 'cancelled' || state.phase === 'error') {
          await local('ack', operation).catch(() => {});
          pending = null; await save(); throw new Error(state.phase === 'cancelled' ? 'Recording cancelled. Nothing was sent.' : state.code === 'occupied' ? 'Apparent is already recording or processing. Finish that capture before trying again.' : 'Apparent could not finish the recording. Open Apparent to inspect it.');
        }
        if (state.phase !== 'saved' || !validSource(state.source)) return;
        if (operation.source && operation.source !== state.source) throw new Error('Recording return changed. Nothing was resent.');
        operation.source = state.source;
        await save();
        if (disposed) return;
        // Submit through the originating registry connection, never the current foreground gateway.
        // Both retries use the same input identity and exact text; the authority deduplicates input_id.
        const route = (await host.profileRoutes()).find(r => r.connectionId === operation.destination.connectionId && r.profile === operation.destination.profile);
        if (!route) throw new Error('The original Hermes gateway is unavailable. Recording saved; nothing was redirected.');
        if (route.targetProfile !== operation.destination.targetProfile || route.mode !== operation.destination.mode) throw new Error('The original Hermes gateway mapping changed. Nothing was sent.');
        if (typeof host.retainProfile !== 'function') throw new Error('Hermes lacks a retained session connection. Update Hermes before sending this recording.');
        releaseRoute = await host.retainProfile(route, { spawnPriority: 'foreground' });
        const identity = runtime(await host.requestProfile(route, 'runtime.describe', {}));
        if (!sameRuntime(operation.runtime, identity)) throw new Error('The original Hermes runtime changed. Check the saved recording; nothing was redirected.');
        if (disposed) return;
        const params = submission(operation.destination, operation.token, state.source);
        // prompt.submit requires this socket to hold a subscription. Exact ID only;
        // no title/latest/create path, and no cwd/provider/settings overrides.
        const snapshot = await host.requestProfile(route, 'session.resume', { session_id: operation.destination.session });
        if (snapshot?.session_id !== operation.destination.session || snapshot.authority_epoch !== operation.runtime.authority_epoch) throw new Error('The original session identity changed. Nothing was sent.');
        operation.sending = true; await save();
        if (disposed) return;
        let receipt;
        for (let attempt = 0; attempt < 2; attempt++) {
          if (disposed) return;
          try { receipt = await host.requestProfile(route, 'prompt.submit', params); break; }
          catch (error) { if (attempt === 1) throw error; }
        }
        if (!receipt?.admission_id) throw new Error('Hermes did not acknowledge this message. Check the original conversation before trying again.');
        // Clear before acknowledgement so a missing native ACK cannot create another submission.
        pending = null; await save();
        await local('ack', operation).catch(() => {});
        if (!disposed) host.notify({ kind: 'success', message: 'Recording sent to the original conversation.' });
      } catch (error) {
        // Preserve a saved source for recovery, but stop automatic retries after a delivery error.
        timer?.(); timer = undefined;
        if (!operation.source && !operation.sending && pending === operation) { pending = null; await save(); }
        report(error instanceof Error ? error.message : 'Recording delivery failed. Check the original conversation.');
      } finally { releaseRoute(); working = false; }
    }
    function watch() { timer?.(); timer = ctx.setInterval(() => void poll(), 2000); void poll(); }
    async function record() {
      if (disposed || working) return;
      if (pending) { watch(); return; }
      working = true;
      let launchMayHaveStarted = false;
      try {
        const owner = host.state.focusedSessionOwner?.get();
        const session = host.state.focusedSessionId?.get();
        if (!owner || !session || typeof host.requestProfile !== 'function' || typeof host.retainProfile !== 'function') throw new Error('Open an existing Hermes conversation. This button requires the SDK’s connection-qualified session API.');
        const routes = (await host.profileRoutes()).filter(r => r.connectionId === owner.connectionId && r.profile === owner.profile);
        if (routes.length !== 1) throw new Error('The conversation’s gateway is ambiguous or unavailable.');
        if (typeof routes[0].targetProfile !== 'string' || typeof routes[0].mode !== 'string') throw new Error('Hermes lacks a complete conversation route. Update Hermes before recording.');
        if (routes[0].mode !== 'local') throw new Error('Use Apparent directly and paste the recording into this remote conversation.');
        const identity = runtime(await host.requestProfile(routes[0], 'runtime.describe', {}));
        if (disposed) return;
        const bytes = crypto.getRandomValues(new Uint32Array(1));
        bridgeKey = undefined;
        pending = { version: 2, token: crypto.randomUUID().replaceAll('-', ''), port: 32768 + bytes[0] % (60999 - 32768 + 1), destination: { session, connectionId: owner.connectionId, profile: owner.profile, targetProfile: routes[0].targetProfile, mode: routes[0].mode, local: true }, runtime: identity, created: Date.now() };
        await save();
        if (disposed) return;
        if (!await ctx.os.openExternal(launchURL(pending.token, pending.port))) {
          if (disposed) return;
          if (!pending.destination.local) throw new Error('Hermes blocks app links on this remote connection. Use Apparent directly and paste the recording handoff.');
          const operation = pending;
          const route = (await host.profileRoutes()).find(r => r.connectionId === operation.destination.connectionId && r.profile === operation.destination.profile);
          if (!route || route.mode !== operation.destination.mode || route.targetProfile !== operation.destination.targetProfile) throw new Error('The original Hermes gateway mapping changed. Nothing was launched.');
          const release = await host.retainProfile(route, { spawnPriority: 'foreground' });
          try {
            if (disposed) return;
            const current = runtime(await host.requestProfile(route, 'runtime.describe', {}));
            if (!sameRuntime(operation.runtime, current)) throw new Error('The original Hermes runtime changed. Nothing was launched.');
            if (disposed) return;
            const snapshot = await host.requestProfile(route, 'session.resume', { session_id: session });
            if (snapshot?.session_id !== session || snapshot.authority_epoch !== operation.runtime.authority_epoch) throw new Error('The original session changed. Nothing was launched.');
            if (disposed) return;
            launchMayHaveStarted = true;
            const result = await host.requestProfile(route, 'shell.exec', { session_id: session, command: launchCommand(operation.token, operation.port) });
            if (result?.code !== 0) { launchMayHaveStarted = false; throw new Error('Unable to open Apparent. Install the app and register its URL handler; the session must allow a local terminal.'); }
          } finally { release(); }
        }
        launchMayHaveStarted = true;
        if (!disposed) watch();
      } catch (error) {
        // An RPC timeout cannot establish that the OS did not open the helper.
        if (launchMayHaveStarted && pending) { if (!disposed) watch(); }
        else { pending = null; await save(); }
        report(error instanceof Error ? error.message : 'Unable to open Apparent');
      }
      finally { working = false; }
    }
    // No credentials or recording media are stored here; only an owned in-flight operation.
    Promise.resolve(ctx.storage.get('recording')).then(async value => {
      if (disposed || pending || !value) return;
      if (value.version !== 2) { await ctx.storage.set('recording', null); report('This older recording handoff cannot resume safely. Copy its recording from Apparent into Hermes.'); return; }
      if (!validToken(value.token)) return;
      if (!Number.isInteger(value.port) || value.port < 32768 || value.port > 60999 || !Number.isFinite(value.created) ||
          !value.runtime || !value.destination || typeof value.destination.local !== 'boolean' || !['session', 'connectionId', 'profile', 'targetProfile', 'mode'].every(k => typeof value.destination[k] === 'string' && value.destination[k])) return;
      runtime(value.runtime);
      pending = value; hasPending.set(true); watch();
    }).catch(() => {});
    function RecordButton() {
      const waiting = useValue(hasPending);
      const label = waiting ? 'Check Apparent recording handoff' : 'Record with Apparent';
      return jsx(Button, { variant: 'ghost', size: 'icon', title: label, 'aria-label': label,
        onClick: () => void record(), children: jsx(Codicon, { name: waiting ? 'sync' : 'record' }) });
    }
    ctx.register({ id: 'record', area: COMPOSER_AREAS.actions, render: RecordButton });
    ctx.register({ id: 'discard', area: PALETTE_AREA, data: {
      id: 'apparent-discard-handoff', label: 'Apparent: discard pending handoff', keywords: ['recording', 'cancel'],
      detail: () => pending ? 'Recording retained in Apparent' : 'No pending handoff',
      run: async () => {
        if (!pending || working) return;
        const operation = pending; pending = null; timer?.(); timer = undefined; await save();
        if (operation.source) await local('ack', operation).catch(() => {});
        if (!disposed) host.notify({ kind: 'info', message: 'Handoff discarded. The recording remains in Apparent.' });
      },
    } });
  },
};
