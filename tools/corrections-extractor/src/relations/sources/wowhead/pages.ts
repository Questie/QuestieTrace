// Read-only access to scraper-questie's raw page caches (data/raw/<version>.db).
// The scraper may be running while we read, so connections are opened read-only and only ever
// SELECT; never write to these files.

import { existsSync } from "fs";
import { DatabaseSync } from "node:sqlite";

export interface StoredQuestPage {
  questId: number;
  body: string;
  fetchedAt: string;
}

export function openRawCache(path: string): DatabaseSync {
  if (!existsSync(path)) throw new Error(`${path} does not exist. Point SCRAPER_DIR at a scraper-questie checkout with raw caches.`);
  return new DatabaseSync(path, { readOnly: true });
}

/**
 * English quest pages for registered, non-excluded quests, in id order. Excluded entries (junk,
 * placeholder, unused) can still have a cached page from before they were excluded.
 */
export function* questPages(db: DatabaseSync): Generator<StoredQuestPage> {
  const statement = db.prepare(
    `SELECT p.entity_id AS questId, p.body AS body, p.fetched_at AS fetchedAt
       FROM pages p
       JOIN entries e ON e.entity_type = p.entity_type AND e.entity_id = p.entity_id
      WHERE p.entity_type = 'quest' AND p.locale = 'enUS' AND e.excluded = 0
      ORDER BY p.entity_id`,
  );
  for (const row of statement.iterate()) yield row as unknown as StoredQuestPage;
}
