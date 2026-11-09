import { createServer, type Server } from 'node:http';
import { createHash, createHmac } from 'node:crypto';
import { tokenValid, sourceValid, type View } from './protocol.ts';
// Read-only capability for this one recording. No capture controls, arbitrary file reads or credentials.
export async function mailbox(token: string, port: number, key: string, read: () => View, acknowledged: () => void): Promise<Server> {
  if (!tokenValid(token) || !tokenValid(key) || key === token || !Number.isInteger(port) || port < 32768 || port > 60999) throw new Error('Invalid bridge address');
  let acknowledgedOnce = false;
  // The public launch ID is not an authentication secret. The separate key is
  // read through the user's local Hermes session, never through this listener.
  const id = createHash('sha256').update(token).digest('hex');
  const mac = (text: string) => createHmac('sha256', key).update(text).digest('hex');
  const server = createServer({ maxHeaderSize: 2048, headersTimeout: 3000, requestTimeout: 3000, connectionsCheckingInterval: 1000, keepAliveTimeout: 1000 }, (request, response) => {
    const origin = request.headers.origin;
    // Electron's file renderer can omit Origin. Authentication is the signed
    // capability, not this header; reject explicit web origins and rebinding.
    if (request.headers.host !== `127.0.0.1:${port}` || (origin !== undefined && origin !== 'null' && origin !== 'file://')) {
      response.writeHead(403).end(); return;
    }
    const headers = { 'Access-Control-Allow-Origin': origin ?? 'null', 'Cache-Control': 'no-store', 'Content-Type': 'application/json', 'X-Content-Type-Options': 'nosniff' };
    const view = read();
    if (request.url === `/${id}/state/${mac(`state:${id}`)}` && request.method === 'GET') {
      const state = { phase: view.phase, ...(view.code === 'occupied' ? { code: 'occupied' } : {}), ...(view.phase === 'saved' && sourceValid(view.source) ? { source: view.source } : {}) };
      const payload = JSON.stringify(state);
      response.writeHead(200, headers).end(JSON.stringify({ payload, mac: mac(payload) }));
    } else if (!acknowledgedOnce && request.url === `/${id}/ack/${mac(`ack:${id}`)}` && request.method === 'POST' && ['saved', 'cancelled', 'error'].includes(view.phase)) {
      acknowledgedOnce = true;
      response.writeHead(200, headers).end('{"ok":true}', acknowledged);
    } else { response.writeHead(404, headers).end('{"error":"unknown_request"}'); }
  });
  server.maxConnections = 4;
  server.maxRequestsPerSocket = 4;
  server.setTimeout(3000, socket => socket.destroy());
  await new Promise<void>((resolve, reject) => { server.once('error', reject); server.listen({ host: '127.0.0.1', port, exclusive: true }, resolve); });
  return server;
}
