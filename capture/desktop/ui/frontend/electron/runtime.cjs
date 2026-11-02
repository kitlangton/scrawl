// Read the existing Rust GUI contract; never launch it or send an action.
const { lstat } = require('node:fs/promises');
const net = require('node:net');
const path = require('node:path');

const phases = new Set(['idle', 'selecting', 'starting', 'recording', 'processing', 'completed']);
const failure = { status: 'error', message: 'The Rust app did not return a valid state. Retry when it is responsive.' };

async function readRuntimeState(home, timeoutMs = 3500) {
  // One absolute timeout, including metadata checks and partial replies.
  let socket;
  let timer;
  const deadline = new Promise((resolve) => {
    timer = setTimeout(() => {
      socket?.destroy();
      resolve(failure);
    }, timeoutMs);
  });
  const read = async () => {
    try {
      const directory = path.join(home, '.config/apparent/agent-control');
      const address = path.join(directory, 'capture.sock');
      const owner = process.geteuid();
      const dir = await lstat(directory);
      const entry = await lstat(address);
      if (!dir.isDirectory() || dir.uid !== owner || (dir.mode & 0o077) !== 0 ||
          !entry.isSocket() || entry.uid !== owner || (entry.mode & 0o777) !== 0o600) {
        return { status: 'error', message: 'The local control socket is not private. It was not contacted.' };
      }
      // Metadata may have finished after the overall deadline. Never connect then.
      if (expired) return failure;
      return await new Promise((resolve) => {
        const chunks = [];
        let length = 0;
        const finish = (result) => { socket.destroy(); resolve(result); };
        socket = net.createConnection(address);
        socket.once('connect', () => socket.write('state\n'));
        socket.on('data', (chunk) => {
          length += chunk.length;
          if (length > 4096) return finish(failure);
          chunks.push(chunk);
          const reply = Buffer.concat(chunks);
          const newline = reply.indexOf(10);
          if (newline < 0) return;
          try {
            const value = JSON.parse(reply.subarray(0, newline).toString('utf8'));
            if (!value.error && phases.has(value.state?.phase)) {
              finish({ status: 'connected', phase: value.state.phase });
            } else finish(failure);
          } catch { finish(failure); }
        });
        socket.once('error', (error) => finish(
          ['ENOENT', 'ECONNREFUSED'].includes(error.code) ? { status: 'disconnected' } : failure,
        ));
        socket.once('end', () => finish(failure));
      });
    } catch (error) {
      return error.code === 'ENOENT' ? { status: 'disconnected' } : failure;
    }
  };
  let expired = false;
  try {
    return await Promise.race([read(), deadline.then((result) => { expired = true; return result; })]);
  } finally {
    clearTimeout(timer);
  }
}

module.exports = { readRuntimeState };
