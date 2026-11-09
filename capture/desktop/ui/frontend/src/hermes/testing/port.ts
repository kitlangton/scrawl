import { createServer } from 'node:net';
export async function availablePort() {
  for (let attempt = 0; attempt < 16; attempt++) {
    const value = 45000 + Math.floor(Math.random() * 15000);
    const server = createServer();
    try {
      await new Promise<void>((resolve, reject) => { server.once('error', reject); server.listen(value, '127.0.0.1', resolve); });
      await new Promise<void>(resolve => server.close(() => resolve()));
      return value;
    } catch { server.close(); }
  }
  throw new Error('No available test port');
}
