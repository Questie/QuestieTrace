// Observations of every processed submission, kept between runs so a run only has to observe
// the submissions added since the last one.
//
// Layout of the store directory:
//   manifest.json                     format and observer version and, per live submission
//                                     key, its batch, load errors and counts
//   batches/<n>.sessions.parquet      one row per session copy, including the copies not
//                                     observed because a newer one was used (`duplicate`)
//   batches/<n>.observations.parquet  one row per observation of the observed copies (enUS
//                                     lookup observations are left out: lookups skip enUS)
//   sessions.parquet, views.sql       written by writeQueryViews for ad-hoc SQL
//
// Rows are never rewritten. A submission that disappears from the source is dropped from the
// manifest only, and only the batches and submissions the manifest lists are read. When the
// observer code changes (see observer-version.ts) the whole store is dropped.
//
// Replay hands the sessions over in the order a full run folds them: submissions newest first,
// their sessions in export order, and of each session (by key) only the first copy met.

import { DuckDBInstance, type DuckDBAppender, type DuckDBConnection } from "@duckdb/node-api";
import { existsSync, mkdirSync, readdirSync, readFileSync, renameSync, rmSync, writeFileSync } from "fs";
import { join } from "path";
import type { StagedSession } from "../extract";
import type { StagedLookupSession } from "../extract/lookups";
import type { ScalarGroup } from "../extract/aggregate";
import type { Confidence, Observation } from "../extract/observation";
import { SKIPPED_LOCALE } from "../extract/observe-lookups";
import type { ObservedSubmission } from "../observe-submission";

const FORMAT_VERSION = 4;

export interface ManifestEntry {
  batch: number;
  submissionId: string;
  exportCount: number;
  droppedExports: string[];
  error?: string;
  /** Session copies not observed because a newer copy was used. */
  duplicates: number;
}

interface Manifest {
  formatVersion: number;
  observerVersion: string;
  nextBatch: number;
  submissions: Record<string, ManifestEntry>;
}

export interface StoredSubmission {
  /** Source key, see store/source.ts. */
  key: string;
  submission: ObservedSubmission;
}

/** One session as replay hands it over. */
export interface ReplayedSession {
  submissionKey: string;
  /** Set when an observer threw on this session (see ObservedSession.error). */
  error?: string;
  extract?: StagedSession;
  lookup?: StagedLookupSession;
}

const CONFIDENCES: Confidence[] = ["low", "medium", "high"];

/** One value group of a scalar or lookup field, see ObservedStore.scalarGroups. */
export interface SummarizedGroup {
  lookup: boolean;
  field: string;
  /** The client locale of a lookup value; null for scalar fields. */
  locale: string | null;
  entityId: number;
  group: Omit<ScalarGroup<unknown>, "lastSessionKey">;
}

/**
 * Session names are not stored: players can name sessions freely. Replayed observations carry
 * the session key as their `provenance.session` instead; merges use `provenance.sessionKey`.
 */
const UNNAMED_SESSION = "(unnamed session)";

/** Submissions replayed per query. */
const REPLAY_SLICE = 2000;
/** `kind` of an observation: a scalar field, a field with a custom merge, or a lookup (translation). */
const KINDS = ["scalar", "custom", "lookup"] as const;
type Kind = (typeof KINDS)[number];

function sqlString(value: string): string {
  return `'${value.replaceAll("'", "''")}'`;
}

function appendNullable(appender: DuckDBAppender, value: string | number | null | undefined, integer = false): void {
  if (value === null || value === undefined) appender.appendNull();
  else if (typeof value === "number") {
    if (integer) appender.appendInteger(value);
    else appender.appendDouble(value);
  } else appender.appendVarchar(value);
}

/** What an observation holds beyond its columns (e.g. the spawn observers' `token`), as JSON; null when nothing. */
function extraOf(observation: Observation<unknown>): string | null {
  const { entityId: _entityId, value: _value, confidence: _confidence, provenance, ...rest } = observation;
  const { session: _session, t: _t, ...provenanceRest } = provenance;
  const hasRest = Object.keys(rest).length > 0;
  const hasProvenanceRest = Object.keys(provenanceRest).length > 0;
  if (!hasRest && !hasProvenanceRest) return null;
  return JSON.stringify({ ...(hasRest ? { o: rest } : {}), ...(hasProvenanceRest ? { p: provenanceRest } : {}) });
}

export class ObservedStore {
  /** Whether table `winners` matches the manifest; cleared when the manifest changes. */
  private winnersReady = false;

  private constructor(
    private readonly dir: string,
    private readonly manifest: Manifest,
    private readonly instance: DuckDBInstance,
    private readonly connection: DuckDBConnection,
  ) {}

  /** Opens the store in `dir`, dropping its contents when they were made by another observer version. */
  static async open(dir: string, observerVersion: string, log: (message: string) => void = () => undefined): Promise<ObservedStore> {
    mkdirSync(dir, { recursive: true });
    let manifest = readManifest(dir, log);
    if (!manifest || manifest.observerVersion !== observerVersion || manifest.formatVersion !== FORMAT_VERSION) {
      if (manifest) log("The observer code or store format changed: observing every submission again");
      rmSync(join(dir, "batches"), { recursive: true, force: true });
      // They would point at batches that no longer exist, or at new ones with the same names.
      rmSync(join(dir, "sessions.parquet"), { force: true });
      rmSync(join(dir, "views.sql"), { force: true });
      manifest = { formatVersion: FORMAT_VERSION, observerVersion, nextBatch: 0, submissions: {} };
      writeManifest(dir, manifest);
    }
    mkdirSync(join(dir, "batches"), { recursive: true });
    // Left behind by a run that was killed.
    rmSync(join(dir, ".tmp"), { recursive: true, force: true });
    for (const name of readdirSync(join(dir, "batches"))) if (name.endsWith(".tmp")) rmSync(join(dir, "batches", name));
    const instance = await DuckDBInstance.create(":memory:", { temp_directory: join(dir, ".tmp"), memory_limit: "4GB", max_temp_directory_size: "20GB" });
    try {
      return new ObservedStore(dir, manifest, instance, await instance.connect());
    } catch (e) {
      instance.closeSync();
      throw e;
    }
  }

  close(): void {
    this.connection.closeSync();
    this.instance.closeSync();
    rmSync(join(this.dir, ".tmp"), { recursive: true, force: true });
  }

  keys(): Set<string> {
    return new Set(Object.keys(this.manifest.submissions));
  }

  entries(): [string, ManifestEntry][] {
    return Object.entries(this.manifest.submissions);
  }

  /** Drops submissions from the store. */
  forget(keys: Iterable<string>): void {
    for (const key of keys) delete this.manifest.submissions[key];
    this.winnersReady = false;
    writeManifest(this.dir, this.manifest);
  }

  /**
   * The other stored submissions holding unobserved copies of sessions that `keys` observed:
   * without `keys`, those copies are needed, so these submissions must be observed again.
   */
  async holdingCopiesOf(keys: string[]): Promise<string[]> {
    const removed = new Set(keys);
    const entries = this.entries();
    if (entries.length === 0 || keys.length === 0) return [];
    await this.loadSubmissions("removed", entries.filter(([key]) => removed.has(key)));
    await this.loadSubmissions("kept", entries.filter(([key]) => !removed.has(key)));
    const reader = await this.connection.runAndReadAll(`
      SELECT DISTINCT copy.submission
      FROM ${this.sessionsOf("kept")} copy
      WHERE copy.duplicate AND copy.session_key IN (
        SELECT used.session_key FROM ${this.sessionsOf("removed")} used WHERE NOT used.duplicate AND used.session_key IS NOT NULL
      )`);
    return reader.getRowObjectsJS().map((row) => row.submission as string);
  }

  /** Stores the observations of `submissions` as one batch. */
  async add(submissions: StoredSubmission[]): Promise<void> {
    if (submissions.length === 0) return;
    const batch = this.manifest.nextBatch;
    const c = this.connection;
    await c.run(`CREATE OR REPLACE TEMP TABLE sessions (submission VARCHAR, batch INTEGER, idx INTEGER, session_key VARCHAR, duplicate BOOLEAN, contributor VARCHAR, session_time DOUBLE, interface_version INTEGER, build INTEGER, locale VARCHAR, has_extract BOOLEAN, has_lookup BOOLEAN, error VARCHAR)`);
    await c.run(`CREATE OR REPLACE TEMP TABLE observations (submission VARCHAR, batch INTEGER, idx INTEGER, kind VARCHAR, pos SMALLINT, field VARCHAR, seq INTEGER, entity INTEGER, locale VARCHAR, confidence VARCHAR, t DOUBLE, value VARCHAR, extra VARCHAR)`);
    const sessions = await c.createAppender("sessions");
    try {
      const observations = await c.createAppender("observations");
      try {
        const appendFields = (key: string, idx: number, kind: Kind, fields: [string, Observation<unknown>[]][]) => {
          fields.forEach(([field, list], pos) => {
            list.forEach((observation, seq) => {
              // Lookup values are {locale, value}: the locale gets its own column.
              const localized = kind === "lookup" ? (observation.value as { locale?: unknown; value?: unknown }) : undefined;
              if (localized?.locale === SKIPPED_LOCALE) return;
              observations.appendVarchar(key);
              observations.appendInteger(batch);
              observations.appendInteger(idx);
              observations.appendVarchar(kind);
              observations.appendSmallInt(pos);
              observations.appendVarchar(field);
              observations.appendInteger(seq);
              observations.appendInteger(observation.entityId);
              appendNullable(observations, typeof localized?.locale === "string" ? localized.locale : null);
              observations.appendVarchar(observation.confidence);
              appendNullable(observations, observation.provenance.t);
              observations.appendVarchar(JSON.stringify(localized ? localized.value : observation.value) ?? "null");
              appendNullable(observations, extraOf(observation));
              observations.endRow();
            });
          });
        };

        for (const { key, submission } of submissions) {
          submission.sessions.forEach((session, idx) => {
            const staged = session.extract ?? session.lookup;
            sessions.appendVarchar(key);
            sessions.appendInteger(batch);
            sessions.appendInteger(idx);
            appendNullable(sessions, session.key);
            sessions.appendBoolean(session.duplicate === true);
            appendNullable(sessions, staged?.contributorId);
            appendNullable(sessions, staged?.sessionTime);
            appendNullable(sessions, session.client?.interfaceVersion, true);
            appendNullable(sessions, session.client?.build, true);
            appendNullable(sessions, session.client?.locale);
            sessions.appendBoolean(session.extract !== undefined);
            sessions.appendBoolean(session.lookup !== undefined);
            appendNullable(sessions, session.error);
            sessions.endRow();
            appendFields(key, idx, "scalar", session.extract?.scalars ?? []);
            appendFields(key, idx, "custom", session.extract?.observations ?? []);
            appendFields(key, idx, "lookup", (session.lookup?.observations ?? []) as [string, Observation<unknown>[]][]);
          });
        }
      } finally {
        observations.closeSync();
      }
    } finally {
      sessions.closeSync();
    }

    // Both files are written under temporary names first, so a crash never leaves half a batch.
    const sessionsFile = this.batchFile(batch, "sessions");
    const observationsFile = this.batchFile(batch, "observations");
    await c.run(`COPY sessions TO ${sqlString(`${sessionsFile}.tmp`)} (FORMAT parquet, COMPRESSION zstd)`);
    await c.run(`COPY (SELECT * FROM observations ORDER BY submission, idx, kind, pos, seq) TO ${sqlString(`${observationsFile}.tmp`)} (FORMAT parquet, COMPRESSION zstd)`);
    await c.run("DROP TABLE sessions");
    await c.run("DROP TABLE observations");
    renameSync(`${sessionsFile}.tmp`, sessionsFile);
    renameSync(`${observationsFile}.tmp`, observationsFile);

    // Only now that the batch is written: a crash before leaves files no manifest entry points at.
    for (const { key, submission } of submissions) {
      this.manifest.submissions[key] = {
        batch,
        submissionId: submission.submissionId,
        exportCount: submission.exportCount,
        droppedExports: submission.droppedExports,
        ...(submission.error === undefined ? {} : { error: submission.error }),
        duplicates: submission.sessions.filter((session) => session.duplicate).length,
      };
    }
    this.manifest.nextBatch = batch + 1;
    this.winnersReady = false;
    writeManifest(this.dir, this.manifest);
  }

  /**
   * Hands every used session to `onSession`, in fold order (see the top of this file). With
   * `customOnly`, sessions carry only the observations of fields with a custom merge: the scalar
   * and lookup fields come from scalarGroups instead.
   * Returns how many observed session copies were left out because a newer copy was used.
   */
  async replay(
    onSession: (session: ReplayedSession) => void,
    { onProgress, slice = REPLAY_SLICE, customOnly = false }: { onProgress?: (submissions: number, total: number) => void; slice?: number; customOnly?: boolean } = {},
  ): Promise<{ duplicateCopies: number }> {
    const entries = this.entries();
    if (entries.length === 0) return { duplicateCopies: 0 };
    const c = this.connection;
    await this.prepareWinners();
    const counts = await c.runAndReadAll(`
      SELECT (SELECT count(*) FROM ${this.sessionsOf("live")} s WHERE NOT s.duplicate) - (SELECT count(*) FROM winners) AS duplicates`);
    const duplicateCopies = Number(counts.getRowObjectsJS()[0].duplicates);
    const kindFilter = customOnly ? "AND kind = 'custom'" : "";

    let current: { submission: string; idx: number; label: string; session: ReplayedSession; fields: Map<string, Observation<unknown>[]>[] } | undefined;
    const flush = () => {
      if (!current) return;
      const [scalars, observations, lookups] = current.fields;
      const { extract, lookup } = current.session;
      if (extract) {
        extract.scalars = [...scalars] as StagedSession["scalars"];
        extract.observations = [...observations] as StagedSession["observations"];
      }
      if (lookup) lookup.observations = [...lookups] as StagedLookupSession["observations"];
      onSession(current.session);
    };

    // A slice of submissions at a time: sorting every observation at once spills tens of GB.
    // Observation files are sorted by submission, so each slice only reads its own row groups.
    // Slices are rank ranges (fold order). The key range of a slice only lets DuckDB skip row
    // groups; the join with the slice's sessions decides which observations belong to it.
    for (let start = 0; start < entries.length; start += slice) {
      const ranks = `BETWEEN ${start} AND ${start + slice - 1}`;
      const bounds = (await c.runAndReadAll(`SELECT min(submission) AS oldest, max(submission) AS newest FROM live WHERE rank ${ranks}`)).getRowObjectsJS()[0];
      const range = `BETWEEN ${sqlString(bounds.oldest as string)} AND ${sqlString(bounds.newest as string)}`;
      const result = await c.stream(`
        SELECT w.submission, w.idx, w.session_key, w.contributor, w.session_time, w.has_extract, w.has_lookup, w.error,
               o.kind, o.field, o.entity, o.locale, o.confidence, o.t, o.value, o.extra
        FROM (SELECT * FROM winners WHERE rank ${ranks}) w
        LEFT JOIN (SELECT * FROM read_parquet(${this.files("observations")}) WHERE submission ${range} ${kindFilter}) o
          ON o.submission = w.submission AND o.batch = w.batch AND o.idx = w.idx
        ORDER BY w.rank, w.idx, o.kind, o.pos, o.seq`);
      for await (const rows of result.yieldRowObjectJs()) {
        for (const row of rows) {
          const submission = row.submission as string;
          const idx = row.idx as number;
          if (!current || current.submission !== submission || current.idx !== idx) {
            flush();
            const identity = {
              ...(row.session_key === null ? {} : { sessionKey: row.session_key as string }),
              ...(row.contributor === null ? {} : { contributorId: row.contributor as string }),
            };
            const sessionTime = (row.session_time as number | null) ?? null;
            const session: ReplayedSession = { submissionKey: submission };
            if (row.error !== null) session.error = row.error as string;
            if (row.has_extract) session.extract = { ...identity, sessionTime, scalars: [], observations: [] };
            if (row.has_lookup) session.lookup = { ...identity, sessionTime, observations: [] };
            current = { submission, idx, label: (row.session_key as string | null) ?? UNNAMED_SESSION, session, fields: KINDS.map(() => new Map()) };
          }
          if (row.kind === null) continue;
          const kind = row.kind as Kind;
          const byField = current.fields[KINDS.indexOf(kind)];
          const field = row.field as string;
          let list = byField.get(field);
          if (!list) {
            list = [];
            byField.set(field, list);
          }
          const extra = row.extra === null ? {} : (JSON.parse(row.extra as string) as { o?: object; p?: object });
          const value: unknown = JSON.parse(row.value as string);
          list.push({
            ...extra.o,
            entityId: row.entity as number,
            value: kind === "lookup" ? { locale: row.locale, value } : value,
            confidence: row.confidence as Confidence,
            provenance: { session: current.label, t: row.t as number, ...extra.p },
          });
        }
      }
      onProgress?.(Math.min(start + slice, entries.length), entries.length);
    }
    flush();
    return { duplicateCopies };
  }

  /**
   * Per scalar or lookup field, entity, locale (lookups only) and value: the summary the scalar
   * merge needs (see ScalarGroup), over the used session copies. Groups come in the order a full
   * fold would first meet their locale, entity and value.
   */
  async scalarGroups(onGroup: (group: SummarizedGroup) => void): Promise<void> {
    if (this.entries().length === 0) return;
    await this.prepareWinners();
    const result = await this.connection.stream(`
      WITH rows AS (
        SELECT o.kind, o.field, o.entity, o.confidence, o.t, w.session_time,
               [w.rank, w.idx] AS session,
               [w.rank, w.idx, o.seq] AS seen,
               o.locale, o.value
        FROM read_parquet(${this.files("observations")}) o
        JOIN winners w ON o.submission = w.submission AND o.batch = w.batch AND o.idx = w.idx
        WHERE o.kind IN ('scalar', 'lookup')
      ), groups AS (
        SELECT kind, field, locale, entity, value,
               max(CASE confidence WHEN 'high' THEN 3 WHEN 'medium' THEN 2 ELSE 1 END) AS confidence,
               count(*) AS count,
               max(t) AS latest_t,
               count(DISTINCT session) FILTER (WHERE session_time IS NOT NULL) AS timed_sessions,
               max(session_time) AS newest_session_time,
               min(seen) AS seen
        FROM rows GROUP BY ALL
      )
      SELECT kind, field, locale, entity, value, confidence, count, latest_t, timed_sessions, newest_session_time
      FROM groups
      ORDER BY kind, field,
               min(seen) OVER (PARTITION BY kind, field, locale),
               min(seen) OVER (PARTITION BY kind, field, locale, entity),
               seen`);
    for await (const rows of result.yieldRowObjectJs()) {
      for (const row of rows) {
        onGroup({
          lookup: row.kind === "lookup",
          field: row.field as string,
          locale: row.locale as string | null,
          entityId: row.entity as number,
          group: {
            value: JSON.parse(row.value as string),
            confidence: CONFIDENCES[(row.confidence as number) - 1],
            count: Number(row.count),
            latestT: row.latest_t as number,
            timedSessionCount: Number(row.timed_sessions),
            newestSessionTime: (row.newest_session_time as number | null) ?? null,
          },
        });
      }
    }
  }

  /**
   * Writes sessions.parquet (the used sessions, see prepareWinners) and views.sql, so the store
   * can be queried without filtering stale rows: `duckdb -init views.sql` in the store directory.
   */
  async writeQueryViews(): Promise<void> {
    if (this.entries().length === 0) return;
    await this.prepareWinners();
    const path = join(this.dir, "sessions.parquet");
    await this.connection.run(`
      COPY (SELECT submission, batch, idx, rank, session_key, contributor, session_time, interface_version, build, locale
            FROM winners ORDER BY rank, idx)
      TO ${sqlString(`${path}.tmp`)} (FORMAT parquet, COMPRESSION zstd)`);
    renameSync(`${path}.tmp`, path);
    const batches = [...new Set(this.entries().map(([, entry]) => entry.batch))].sort((a, b) => a - b);
    const files = batches.map((batch) => sqlString(`batches/${batch}.observations.parquet`)).join(", ");
    writeFileSync(
      join(this.dir, "views.sql"),
      `-- Generated by the corrections-extractor. Run from this directory: duckdb -init views.sql
-- sessions: one row per used session (newest copy), rank 0 being the newest submission.
-- observations: their observations, with the session's columns joined in.
CREATE OR REPLACE VIEW sessions AS SELECT * FROM 'sessions.parquet';
CREATE OR REPLACE VIEW observations AS
SELECT s.session_key, s.contributor, s.session_time, s.interface_version, s.build, s.locale AS client_locale,
       o.kind, o.field, o.entity, o.locale, o.value, o.confidence, o.t, o.extra, o.submission, o.idx
FROM read_parquet([${files}]) o JOIN sessions s USING (submission, batch, idx);
`,
    );
  }

  /** Table `winners`: the first copy met of each session (every copy of sessions without a key), with its rank. */
  private async prepareWinners(): Promise<void> {
    if (this.winnersReady) return;
    await this.loadSubmissions("live", this.entries());
    await this.connection.run(`
      CREATE OR REPLACE TEMP TABLE winners AS
      SELECT * EXCLUDE (copy) FROM (
        SELECT s.*, row_number() OVER (PARTITION BY s.session_key ORDER BY s.rank, s.idx) AS copy
        FROM ${this.sessionsOf("live")} s
        WHERE NOT s.duplicate
      ) WHERE session_key IS NULL OR copy = 1`);
    this.winnersReady = true;
  }

  private batchFile(batch: number, kind: "sessions" | "observations"): string {
    return join(this.dir, "batches", `${batch}.${kind}.parquet`);
  }

  /** Loads `entries` into table `name` (submission, batch, rank), rank 0 being the newest submission. */
  private async loadSubmissions(name: string, entries: [string, ManifestEntry][]): Promise<void> {
    const sorted = [...entries].sort(([a], [b]) => (a < b ? 1 : a > b ? -1 : 0));
    await this.connection.run(`CREATE OR REPLACE TEMP TABLE ${name} (submission VARCHAR, batch INTEGER, rank INTEGER)`);
    const appender = await this.connection.createAppender(name);
    try {
      sorted.forEach(([key, entry], rank) => {
        appender.appendVarchar(key);
        appender.appendInteger(entry.batch);
        appender.appendInteger(rank);
        appender.endRow();
      });
    } finally {
      appender.closeSync();
    }
  }

  /** The parquet files of the batches the manifest points at. */
  private files(kind: "sessions" | "observations"): string {
    const batches = [...new Set(this.entries().map(([, entry]) => entry.batch))].sort((a, b) => a - b);
    return `[${batches.map((batch) => sqlString(this.batchFile(batch, kind))).join(", ")}]`;
  }

  /** The session rows of the submissions in table `name`, with their rank. */
  private sessionsOf(name: string): string {
    return `(SELECT s.*, ${name}.rank FROM read_parquet(${this.files("sessions")}) s JOIN ${name} USING (submission, batch))`;
  }
}

function readManifest(dir: string, log: (message: string) => void): Manifest | undefined {
  const path = join(dir, "manifest.json");
  if (!existsSync(path)) return undefined;
  try {
    return JSON.parse(readFileSync(path, "utf8")) as Manifest;
  } catch {
    // A torn write: start over rather than fail every run.
    log(`${path} is unreadable: observing every submission again`);
    return undefined;
  }
}

function writeManifest(dir: string, manifest: Manifest): void {
  const path = join(dir, "manifest.json");
  writeFileSync(`${path}.tmp`, JSON.stringify(manifest), { flush: true });
  renameSync(`${path}.tmp`, path);
}
