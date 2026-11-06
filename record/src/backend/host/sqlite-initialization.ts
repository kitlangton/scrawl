import type { DatabaseSync } from "node:sqlite";

const maxWalAttempts = 8;
const walRetryDelayMs = 25;

/** Set WAL only retrying SQLite's numeric busy/locked result during first-open contention. */
export async function initializePrivateSqlite(db: DatabaseSync): Promise<void> {
  db.exec("PRAGMA busy_timeout = 100;");
  for (let attempt = 0; ; attempt++) {
    try {
      db.exec("PRAGMA journal_mode = WAL;");
      break;
    } catch (error) {
      const code = (error as { errcode?: unknown }).errcode;
      if ((code !== 5 && code !== 6) || attempt + 1 >= maxWalAttempts) throw error;
      await new Promise(resolve => setTimeout(resolve, walRetryDelayMs * Math.min(attempt + 1, 4)));
    }
  }
  db.exec("PRAGMA synchronous = FULL; PRAGMA busy_timeout = 5000;");
}
